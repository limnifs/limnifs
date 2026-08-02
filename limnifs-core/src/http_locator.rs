//! HTTP range-streaming locator — fetches slab bytes via HTTP/1.1
//! byte-range requests.
//!
//! ## Design
//!
//! Hand-rolled HTTP/1.1 client over `std::net::TcpStream`. No TLS,
//! no async, no external HTTP crate. The wire format is small enough
//! that a focused implementation is both clearer and dependency-free
//! than wrapping `reqwest` or `ureq`.
//!
//! For HTTPS support, callers wrap the URI with a TLS-terminating
//! proxy or wait for the optional `https` feature (planned: rustls
//! adapter). The CI tests use a local HTTP server.
//!
//! ## Wire details
//!
//! - Method: `GET`
//! - Headers: `Host`, `Connection: close`, optional `Range: bytes=...`
//! - Body: full bytes (200 OK) or requested range (206 Partial Content)
//! - Errors: 4xx / 5xx mapped to [`LocatorError::Status`]
//! - 404 → [`LocatorError::NotFound`]
//!
//! See [`HttpLocator`] and task `08-http-range-streaming.md`.

#![cfg(feature = "http")]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use crate::fetch::{Locator, LocatorError};

/// Connect/read timeout for HTTP requests.
const HTTP_TIMEOUT: Duration = Duration::from_secs(30);

/// A locator that fetches bytes over HTTP/1.1.
///
/// Use [`HttpLocator::new`] for the default instance, or
/// [`HttpLocator::with_agent`] to set a custom User-Agent.
#[derive(Debug, Clone)]
pub struct HttpLocator {
    agent: String,
}

impl HttpLocator {
    /// Create an HTTP locator with the default User-Agent.
    #[must_use]
    pub fn new() -> Self {
        Self {
            agent: format!("limnifs/{}", env!("CARGO_PKG_VERSION")),
        }
    }

    /// Create an HTTP locator with a custom User-Agent string.
    #[must_use]
    pub fn with_agent(agent: impl Into<String>) -> Self {
        Self {
            agent: agent.into(),
        }
    }
}

impl Default for HttpLocator {
    fn default() -> Self {
        Self::new()
    }
}

impl Locator for HttpLocator {
    fn fetch(&self, uri: &str) -> Result<Vec<u8>, LocatorError> {
        let response = HttpRequest::new(uri, self.agent.clone())?.send()?;
        response.into_body(uri)
    }

    fn fetch_range(&self, uri: &str, offset: u64, length: u64) -> Result<Vec<u8>, LocatorError> {
        let end = offset
            .checked_add(length)
            .ok_or_else(|| LocatorError::InvalidUri {
                reason: format!("offset ({offset}) + length ({length}) overflows u64"),
            })?
            .saturating_sub(1);
        let range = format!("bytes={offset}-{end}");
        let response = HttpRequest::new(uri, self.agent.clone())?
            .with_range(range)
            .send()?;
        // 206 Partial Content is expected for a range request; a
        // server that ignores Range (returning 200) is also tolerated
        // — we slice in memory to honour the caller's contract.
        if response.status == 200 {
            let body = response.into_body(uri)?;
            let start = offset.min(body.len() as u64);
            let end = start.saturating_add(length).min(body.len() as u64);
            let s = usize::try_from(start).unwrap_or(usize::MAX);
            let e = usize::try_from(end).unwrap_or(usize::MAX);
            return Ok(body[s..e].to_vec());
        }
        if response.status == 206 {
            return response.into_body(uri);
        }
        if response.status == 416 {
            // Range past EOF — return empty per the Locator contract
            // (mirrors the default-impl behaviour of clamping offset
            // to total length).
            return Ok(Vec::new());
        }
        Err(LocatorError::Status {
            code: response.status,
            body: response.body_string(),
        })
    }

    fn scheme(&self) -> &'static str {
        "http"
    }
}

/// A single HTTP/1.1 request, accumulated as a builder.
struct HttpRequest {
    host: String,
    port: u16,
    path: String,
    range: Option<String>,
    agent: String,
}

impl HttpRequest {
    /// Parse `uri` into a request.
    fn new(uri: &str, agent: String) -> Result<Self, LocatorError> {
        let (host, port, path) = parse_http_url(uri)?;
        Ok(Self {
            host,
            port,
            path,
            range: None,
            agent,
        })
    }

    fn with_range(mut self, range: String) -> Self {
        self.range = Some(range);
        self
    }

    /// Serialise, send, read response.
    fn send(self) -> Result<RawResponse, LocatorError> {
        let target = format!("{}:{}", self.host, self.port);
        let mut stream = TcpStream::connect_timeout(
            &target
                .to_socket_addrs_first()
                .ok_or_else(|| LocatorError::InvalidUri {
                    reason: format!("cannot resolve host:port {target}"),
                })?,
            HTTP_TIMEOUT,
        )?;
        stream.set_read_timeout(Some(HTTP_TIMEOUT))?;
        stream.set_write_timeout(Some(HTTP_TIMEOUT))?;

        let range_header = self
            .range
            .as_ref()
            .map(|r| format!("Range: {r}\r\n"))
            .unwrap_or_default();
        let request = format!(
            "GET {path} HTTP/1.1\r\nHost: {host}\r\nUser-Agent: {agent}\r\n\
             Connection: close\r\nAccept: */*\r\n{range_header}\r\n",
            path = self.path,
            host = self.host,
            agent = self.agent,
            range_header = range_header,
        );
        stream.write_all(request.as_bytes())?;
        stream.flush()?;

        read_response(&mut stream)
    }
}

/// Parse an `http(s)://host[:port]/path` URI.
///
/// Returns `(host, port, path)`. Port defaults to 80 for http, 443 for
/// https. Path defaults to `/` when absent. https is accepted by the
/// parser (so callers can detect the scheme) but the actual transport
/// only supports plain TCP — see module docs.
fn parse_http_url(uri: &str) -> Result<(String, u16, String), LocatorError> {
    let rest = uri
        .strip_prefix("http://")
        .or_else(|| uri.strip_prefix("https://"))
        .ok_or_else(|| LocatorError::UnsupportedScheme {
            scheme: scheme_of(uri).to_owned(),
        })?;
    let (authority, path) = match rest.find('/') {
        Some(pos) => (&rest[..pos], &rest[pos..]),
        None => (rest, "/"),
    };
    if authority.is_empty() {
        return Err(LocatorError::InvalidUri {
            reason: "http URI has empty authority".into(),
        });
    }
    let (host, port) = match authority.find(':') {
        Some(pos) => {
            let h = &authority[..pos];
            let p = &authority[pos + 1..];
            let port = p.parse::<u16>().map_err(|_| LocatorError::InvalidUri {
                reason: format!("invalid port: {p}"),
            })?;
            (h.to_owned(), port)
        }
        None => (authority.to_owned(), 80),
    };
    Ok((host, port, path.to_owned()))
}

fn scheme_of(uri: &str) -> &str {
    match uri.find(':') {
        Some(pos) => &uri[..pos],
        None => "",
    }
}

/// Read and parse the HTTP/1.1 response. Body is read until EOF
/// (Connection: close). Supports both Content-Length and chunked
/// transfer encoding.
fn read_response(stream: &mut TcpStream) -> Result<RawResponse, LocatorError> {
    let mut reader = BufReader::new(stream);
    let mut status_line = String::new();
    reader.read_line(&mut status_line)?;
    let status = parse_status_line(&status_line)?;

    let mut content_length: Option<usize> = None;
    let mut chunked = false;
    loop {
        let mut header = String::new();
        let bytes_read = reader.read_line(&mut header)?;
        if bytes_read == 0 {
            return Err(LocatorError::Io(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "response truncated mid-headers",
            )));
        }
        let trimmed = header.trim_end();
        if trimmed.is_empty() {
            break;
        }
        let Some((name, value)) = trimmed.split_once(':') else {
            continue;
        };
        let name_lower = name.to_ascii_lowercase();
        let value = value.trim();
        if name_lower == "content-length" {
            content_length = value.parse::<usize>().ok();
        } else if name_lower == "transfer-encoding" && value.eq_ignore_ascii_case("chunked") {
            chunked = true;
        }
    }

    let body = if chunked {
        read_chunked(&mut reader)?
    } else if let Some(n) = content_length {
        let mut buf = vec![0u8; n];
        reader.read_exact(&mut buf)?;
        buf
    } else {
        let mut buf = Vec::new();
        reader.read_to_end(&mut buf)?;
        buf
    };

    Ok(RawResponse { status, body })
}

fn read_chunked(reader: &mut impl BufRead) -> Result<Vec<u8>, LocatorError> {
    let mut buf = Vec::new();
    loop {
        let mut size_line = String::new();
        let bytes_read = reader.read_line(&mut size_line)?;
        if bytes_read == 0 {
            return Err(LocatorError::Io(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "chunked response truncated mid-size",
            )));
        }
        let size_str = size_line.trim().split(';').next().unwrap_or("").trim();
        let chunk_size = usize::from_str_radix(size_str, 16).map_err(|_| {
            LocatorError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("invalid chunk size: {size_str:?}"),
            ))
        })?;
        if chunk_size == 0 {
            // Trailing headers (we read until empty line) then done.
            loop {
                let mut tail = String::new();
                let n = reader.read_line(&mut tail)?;
                if n == 0 || tail.trim().is_empty() {
                    break;
                }
            }
            return Ok(buf);
        }
        let mut chunk = vec![0u8; chunk_size];
        reader.read_exact(&mut chunk)?;
        buf.extend_from_slice(&chunk);
        // Consume CRLF after chunk data.
        let mut crlf = [0u8; 2];
        reader.read_exact(&mut crlf)?;
    }
}

fn parse_status_line(line: &str) -> Result<u16, LocatorError> {
    let trimmed = line.trim();
    let parts: Vec<&str> = trimmed.splitn(3, ' ').collect();
    if parts.len() < 2 {
        return Err(LocatorError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("malformed status line: {trimmed:?}"),
        )));
    }
    parts[1].parse::<u16>().map_err(|_| {
        LocatorError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("non-numeric status: {:?}", parts[1]),
        ))
    })
}

/// Raw HTTP response: status code + body bytes.
struct RawResponse {
    status: u16,
    body: Vec<u8>,
}

impl RawResponse {
    /// Map status code to a [`LocatorError`] or return the body.
    fn into_body(self, _uri: &str) -> Result<Vec<u8>, LocatorError> {
        if self.status == 200 || self.status == 206 {
            Ok(self.body)
        } else if self.status == 404 {
            Err(LocatorError::NotFound)
        } else {
            Err(LocatorError::Status {
                code: self.status,
                body: String::from_utf8_lossy(&self.body).into_owned(),
            })
        }
    }

    fn body_string(self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

/// Helper trait: `to_socket_addrs` returning the first match.
trait ToSocketAddrsFirst {
    fn to_socket_addrs_first(&self) -> Option<std::net::SocketAddr>;
}

impl ToSocketAddrsFirst for str {
    fn to_socket_addrs_first(&self) -> Option<std::net::SocketAddr> {
        use std::net::ToSocketAddrs;
        self.to_socket_addrs().ok()?.next()
    }
}

impl ToSocketAddrsFirst for String {
    fn to_socket_addrs_first(&self) -> Option<std::net::SocketAddr> {
        self.as_str().to_socket_addrs_first()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::net::TcpListener;

    /// A minimal HTTP/1.1 server for tests. Honours `Range` requests.
    /// Supports Content-Length and chunked encoding (to exercise both).
    struct MiniHttp {
        listener: TcpListener,
        body: Vec<u8>,
        chunked: bool,
    }

    impl MiniHttp {
        fn bind(body: Vec<u8>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
            Self {
                listener,
                body,
                chunked: false,
            }
        }

        fn chunked(mut self) -> Self {
            self.chunked = true;
            self
        }

        fn local_addr(&self) -> std::net::SocketAddr {
            self.listener.local_addr().expect("addr")
        }

        fn serve_fully(&self) {
            let (mut sock, _) = self.listener.accept().expect("accept");
            let mut reader = BufReader::new(sock.try_clone().unwrap());
            let mut range: Option<(u64, u64)> = None;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap() == 0 {
                    break;
                }
                let trimmed = line.trim_end();
                if trimmed.is_empty() {
                    break;
                }
                if let Some(value) = trimmed.strip_prefix("Range:") {
                    let value = value.trim();
                    if let Some(rest) = value.strip_prefix("bytes=") {
                        if let Some((s, e)) = rest.split_once('-') {
                            let start = s.parse::<u64>().unwrap_or(0);
                            let end = e.parse::<u64>().unwrap_or(self.body.len() as u64);
                            range = Some((start, end));
                        }
                    }
                }
            }
            if let Some((start, end)) = range {
                let start_us = usize::try_from(start).unwrap_or(usize::MAX);
                if start_us > self.body.len() {
                    let header = "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Length: 0\r\n\r\n";
                    sock.write_all(header.as_bytes()).unwrap();
                    return;
                }
                let end_plus_one = end
                    .checked_add(1)
                    .map_or(usize::MAX, |v| usize::try_from(v).unwrap_or(usize::MAX));
                let e = end_plus_one.min(self.body.len());
                let chunk = &self.body[start_us..e];
                let header = format!(
                    "HTTP/1.1 206 Partial Content\r\nContent-Type: application/octet-stream\r\n\
                     Content-Range: bytes {start}-{end}/{total}\r\nContent-Length: {len}\r\n\r\n",
                    total = self.body.len(),
                    len = chunk.len(),
                );
                sock.write_all(header.as_bytes()).unwrap();
                sock.write_all(chunk).unwrap();
            } else {
                let body = &self.body;
                if self.chunked {
                    let header = "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nTransfer-Encoding: chunked\r\n\r\n";
                    sock.write_all(header.as_bytes()).unwrap();
                    let line = format!("{:x}\r\n", body.len());
                    sock.write_all(line.as_bytes()).unwrap();
                    sock.write_all(body).unwrap();
                    sock.write_all(b"\r\n0\r\n\r\n").unwrap();
                } else {
                    let header = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\n\r\n",
                        body.len(),
                    );
                    sock.write_all(header.as_bytes()).unwrap();
                    sock.write_all(body).unwrap();
                }
            }
        }
    }

    #[test]
    fn parse_http_url_default_port() {
        let (h, p, path) = parse_http_url("http://example.com/foo").unwrap();
        assert_eq!(h, "example.com");
        assert_eq!(p, 80);
        assert_eq!(path, "/foo");
    }

    #[test]
    fn parse_http_url_explicit_port() {
        let (h, p, path) = parse_http_url("http://localhost:8080/bar").unwrap();
        assert_eq!(h, "localhost");
        assert_eq!(p, 8080);
        assert_eq!(path, "/bar");
    }

    #[test]
    fn parse_http_url_no_path() {
        let (_h, _p, path) = parse_http_url("http://example.com").unwrap();
        assert_eq!(path, "/");
    }

    #[test]
    fn parse_http_url_rejects_other_schemes() {
        match parse_http_url("file:///tmp/x") {
            Err(LocatorError::UnsupportedScheme { scheme }) => assert_eq!(scheme, "file"),
            other => panic!("expected UnsupportedScheme, got {other:?}"),
        }
    }

    #[test]
    fn http_locator_full_fetch() {
        let payload = b"the quick brown fox".to_vec();
        let server = MiniHttp::bind(payload);
        let addr = server.local_addr();
        let uri = format!("http://{addr}/slab.bin");

        let handle = std::thread::spawn(move || server.serve_fully());
        let locator = HttpLocator::new();
        let data = locator.fetch(&uri).expect("fetch");
        handle.join().unwrap();
        assert_eq!(data, b"the quick brown fox");
    }

    #[test]
    fn http_locator_range_fetch() {
        let payload = b"0123456789ABCDEFGHIJ".to_vec(); // 20 bytes
        let server = MiniHttp::bind(payload);
        let addr = server.local_addr();
        let uri = format!("http://{addr}/slab.bin");

        let handle = std::thread::spawn(move || server.serve_fully());
        let locator = HttpLocator::new();
        let data = locator.fetch_range(&uri, 5, 10).expect("fetch_range");
        handle.join().unwrap();
        assert_eq!(data, b"56789ABCDE");
    }

    #[test]
    fn http_locator_range_past_eof_clamps() {
        let payload = b"0123456789".to_vec();
        let server = MiniHttp::bind(payload);
        let addr = server.local_addr();
        let uri = format!("http://{addr}/slab.bin");

        let handle = std::thread::spawn(move || server.serve_fully());
        let locator = HttpLocator::new();
        let data = locator.fetch_range(&uri, 5, 100).expect("fetch_range");
        handle.join().unwrap();
        assert_eq!(data, b"56789");
    }

    #[test]
    fn http_locator_offset_past_eof_empty() {
        let payload = b"0123456789".to_vec();
        let server = MiniHttp::bind(payload);
        let addr = server.local_addr();
        let uri = format!("http://{addr}/slab.bin");

        let handle = std::thread::spawn(move || server.serve_fully());
        let locator = HttpLocator::new();
        let data = locator.fetch_range(&uri, 100, 10).expect("fetch_range");
        handle.join().unwrap();
        assert_eq!(data, b"");
    }

    #[test]
    fn http_locator_chunked_encoding() {
        let payload = b"chunked body data".to_vec();
        let server = MiniHttp::bind(payload).chunked();
        let addr = server.local_addr();
        let uri = format!("http://{addr}/slab.bin");

        let handle = std::thread::spawn(move || server.serve_fully());
        let locator = HttpLocator::new();
        let data = locator.fetch(&uri).expect("fetch");
        handle.join().unwrap();
        assert_eq!(data, b"chunked body data");
    }

    #[test]
    fn http_locator_404_returns_not_found() {
        let server = MiniHttp::bind(b"not found body".to_vec());
        let addr = server.local_addr();
        let uri = format!("http://{addr}/missing.bin");

        // Custom server that returns 404.
        let handle = std::thread::spawn(move || {
            let (mut sock, _) = server.listener.accept().unwrap();
            let mut reader = BufReader::new(sock.try_clone().unwrap());
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap() == 0 {
                    break;
                }
                if line.trim().is_empty() {
                    break;
                }
            }
            let body = b"not found";
            let header = format!(
                "HTTP/1.1 404 Not Found\r\nContent-Length: {}\r\n\r\n",
                body.len(),
            );
            sock.write_all(header.as_bytes()).unwrap();
            sock.write_all(body).unwrap();
        });
        let locator = HttpLocator::new();
        match locator.fetch(&uri) {
            Err(LocatorError::NotFound) => {}
            other => panic!("expected NotFound, got {other:?}"),
        }
        handle.join().unwrap();
    }

    #[test]
    fn http_locator_500_returns_status_error() {
        let server = MiniHttp::bind(Vec::new());
        let addr = server.local_addr();
        let uri = format!("http://{addr}/fail.bin");

        let handle = std::thread::spawn(move || {
            let (mut sock, _) = server.listener.accept().unwrap();
            let mut reader = BufReader::new(sock.try_clone().unwrap());
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap() == 0 {
                    break;
                }
                if line.trim().is_empty() {
                    break;
                }
            }
            let body = b"internal error";
            let header = format!(
                "HTTP/1.1 500 Internal Server Error\r\nContent-Length: {}\r\n\r\n",
                body.len(),
            );
            sock.write_all(header.as_bytes()).unwrap();
            sock.write_all(body).unwrap();
        });
        let locator = HttpLocator::new();
        match locator.fetch(&uri) {
            Err(LocatorError::Status { code, .. }) => assert_eq!(code, 500),
            other => panic!("expected Status, got {other:?}"),
        }
        handle.join().unwrap();
    }

    #[test]
    fn http_locator_reports_http_scheme() {
        let locator = HttpLocator::new();
        assert_eq!(locator.scheme(), "http");
    }

    #[test]
    fn http_locator_rejects_non_http_uri() {
        let locator = HttpLocator::new();
        match locator.fetch("file:///tmp/x") {
            Err(LocatorError::UnsupportedScheme { scheme }) => assert_eq!(scheme, "file"),
            other => panic!("expected UnsupportedScheme, got {other:?}"),
        }
    }

    #[test]
    fn http_locator_rejects_empty_authority() {
        let locator = HttpLocator::new();
        match locator.fetch("http:///path-only") {
            Err(LocatorError::InvalidUri { .. }) => {}
            other => panic!("expected InvalidUri, got {other:?}"),
        }
    }
}
