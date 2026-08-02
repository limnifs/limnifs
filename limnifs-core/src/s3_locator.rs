//! S3 locator — fetches slab bytes from S3-compatible object stores
//! via path-style HTTP requests.
//!
//! ## Design
//!
//! S3's wire protocol is HTTP. The `s3:` scheme is a shorthand for a
//! bucket+key pair on a configurable endpoint. This locator translates
//! `s3://bucket/key` into `{endpoint}/bucket/key` and delegates to
//! [`HttpLocator`] for the actual fetch. The same code path supports
//! `AWS S3`, `MinIO`, `DigitalOcean Spaces`, `Backblaze B2`, etc. —
//! any store that honours path-style GET with byte-range requests.
//!
//! ## Authentication
//!
//! v1 (this module) targets **public buckets** and **`MinIO` with
//! anonymous access** — enough for static hosting and CI integration
//! tests. `SigV4` signing (private buckets, IAM roles, STS sessions)
//! will land behind a future `aws-sigv4` feature so callers who don't
//! need AWS auth pay no cryptographic dep cost.
//!
//! ## Endpoint resolution
//!
//! | Pattern | Endpoint |
//! |---|---|
//! | AWS S3 public | `https://s3.<region>.amazonaws.com` |
//! | `MinIO` (local) | `http://localhost:9000` |
//! | Custom gateway | caller-supplied |
//!
//! See [`S3Locator`] and task `08-s3-locator.md`.

#![cfg(feature = "http")]

use crate::fetch::{Locator, LocatorError};
use crate::http_locator::HttpLocator;

/// Default S3 endpoint (AWS, `us-east-1`).
pub const DEFAULT_S3_ENDPOINT: &str = "https://s3.us-east-1.amazonaws.com";

/// A locator that fetches from S3-compatible object stores.
///
/// Build with [`S3Locator::with_endpoint`] for `MinIO` or custom
/// gateways; use [`S3Locator::new`] (or [`Default::default`]) for the
/// AWS public endpoint.
#[derive(Debug, Clone)]
pub struct S3Locator {
    endpoint: String,
    inner: HttpLocator,
}

impl S3Locator {
    /// Create an S3 locator pointing at the AWS public endpoint
    /// (`https://s3.us-east-1.amazonaws.com`).
    #[must_use]
    pub fn new() -> Self {
        Self::with_endpoint_and_agent(DEFAULT_S3_ENDPOINT, HttpLocator::new())
    }

    /// Create an S3 locator pointing at a custom endpoint (e.g. `MinIO`
    /// at `http://localhost:9000`, or another region's AWS endpoint).
    #[must_use]
    pub fn with_endpoint(endpoint: impl Into<String>) -> Self {
        Self::with_endpoint_and_agent(endpoint, HttpLocator::new())
    }

    /// Compose an S3 locator with a pre-built [`HttpLocator`] (e.g. to
    /// inject a custom User-Agent).
    #[must_use]
    pub fn with_endpoint_and_agent(endpoint: impl Into<String>, inner: HttpLocator) -> Self {
        let mut endpoint = endpoint.into();
        if endpoint.ends_with('/') {
            endpoint.pop();
        }
        Self { endpoint, inner }
    }

    fn translate(&self, uri: &str) -> Result<String, LocatorError> {
        let path = uri
            .strip_prefix("s3://")
            .ok_or_else(|| LocatorError::UnsupportedScheme {
                scheme: scheme_of(uri).to_owned(),
            })?;
        if path.is_empty() {
            return Err(LocatorError::InvalidUri {
                reason: "s3 URI has empty bucket/key".into(),
            });
        }
        // Path-style: s3://bucket/key -> {endpoint}/bucket/key
        // The bucket portion runs to the first '/' (the key boundary).
        if !path.contains('/') {
            return Err(LocatorError::InvalidUri {
                reason: format!("s3 URI {uri:?} is missing the key (expected s3://bucket/key)"),
            });
        }
        Ok(format!("{}/{path}", self.endpoint))
    }
}

impl Default for S3Locator {
    fn default() -> Self {
        Self::new()
    }
}

impl Locator for S3Locator {
    fn fetch(&self, uri: &str) -> Result<Vec<u8>, LocatorError> {
        let http_uri = self.translate(uri)?;
        self.inner.fetch(&http_uri)
    }

    fn fetch_range(&self, uri: &str, offset: u64, length: u64) -> Result<Vec<u8>, LocatorError> {
        let http_uri = self.translate(uri)?;
        self.inner.fetch_range(&http_uri, offset, length)
    }

    fn scheme(&self) -> &'static str {
        "s3"
    }
}

fn scheme_of(uri: &str) -> &str {
    match uri.find(':') {
        Some(pos) => &uri[..pos],
        None => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;

    /// Translate-only tests (no network).
    #[test]
    fn translates_basic_uri() {
        let s3 = S3Locator::with_endpoint("http://localhost:9000");
        assert_eq!(
            s3.translate("s3://my-bucket/slab-0.bin").unwrap(),
            "http://localhost:9000/my-bucket/slab-0.bin"
        );
    }

    #[test]
    fn translates_strips_trailing_slash_from_endpoint() {
        let s3 = S3Locator::with_endpoint("http://localhost:9000/");
        assert_eq!(
            s3.translate("s3://b/k").unwrap(),
            "http://localhost:9000/b/k"
        );
    }

    #[test]
    fn translates_key_with_nested_path() {
        let s3 = S3Locator::with_endpoint("https://s3.us-west-2.amazonaws.com");
        assert_eq!(
            s3.translate("s3://bucket/path/to/object.bin").unwrap(),
            "https://s3.us-west-2.amazonaws.com/bucket/path/to/object.bin"
        );
    }

    #[test]
    fn rejects_non_s3_scheme() {
        let s3 = S3Locator::new();
        match s3.translate("http://x/y") {
            Err(LocatorError::UnsupportedScheme { scheme }) => assert_eq!(scheme, "http"),
            other => panic!("expected UnsupportedScheme, got {other:?}"),
        }
    }

    #[test]
    fn rejects_missing_key() {
        let s3 = S3Locator::with_endpoint("http://localhost:9000");
        match s3.translate("s3://bucket-only") {
            Err(LocatorError::InvalidUri { .. }) => {}
            other => panic!("expected InvalidUri, got {other:?}"),
        }
    }

    #[test]
    fn rejects_empty_path() {
        let s3 = S3Locator::with_endpoint("http://localhost:9000");
        match s3.translate("s3://") {
            Err(LocatorError::InvalidUri { .. }) => {}
            other => panic!("expected InvalidUri, got {other:?}"),
        }
    }

    #[test]
    fn reports_s3_scheme() {
        let s3 = S3Locator::new();
        assert_eq!(s3.scheme(), "s3");
    }

    /// Round-trip fetch via a mini path-style S3 server.
    /// Confirms `HttpLocator` delegation works end-to-end.
    struct MiniS3 {
        listener: TcpListener,
        objects: std::collections::HashMap<String, Vec<u8>>,
    }

    impl MiniS3 {
        fn new() -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            Self {
                listener,
                objects: std::collections::HashMap::new(),
            }
        }

        fn with_object(mut self, key: &str, body: Vec<u8>) -> Self {
            self.objects.insert(format!("/bucket/{key}"), body);
            self
        }

        fn local_addr(&self) -> std::net::SocketAddr {
            self.listener.local_addr().unwrap()
        }

        fn serve_fully(&self) {
            let (mut sock, _) = self.listener.accept().unwrap();
            let mut reader = BufReader::new(sock.try_clone().unwrap());
            let mut want_range: Option<(u64, u64)> = None;
            let mut path = String::new();
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap() == 0 {
                    break;
                }
                let trimmed = line.trim_end();
                if trimmed.is_empty() {
                    break;
                }
                if let Some(rest) = trimmed.strip_prefix("GET ") {
                    path = rest.split_whitespace().next().unwrap_or("").to_owned();
                }
                if let Some(value) = trimmed.strip_prefix("Range:") {
                    let v = value.trim();
                    if let Some(rest) = v.strip_prefix("bytes=") {
                        if let Some((s, e)) = rest.split_once('-') {
                            let start = s.parse::<u64>().unwrap_or(0);
                            let end = e.parse::<u64>().unwrap_or(u64::MAX);
                            want_range = Some((start, end));
                        }
                    }
                }
            }
            let body = self.objects.get(&path);
            match body {
                None => {
                    let h = "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n";
                    sock.write_all(h.as_bytes()).unwrap();
                }
                Some(body) => {
                    if let Some((start, end)) = want_range {
                        let start_us = usize::try_from(start).unwrap_or(usize::MAX);
                        let end_us = end
                            .checked_add(1)
                            .map_or(usize::MAX, |v| usize::try_from(v).unwrap_or(usize::MAX))
                            .min(body.len());
                        if start_us > body.len() {
                            let h =
                                "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Length: 0\r\n\r\n";
                            sock.write_all(h.as_bytes()).unwrap();
                            return;
                        }
                        let chunk = &body[start_us..end_us];
                        let header = format!(
                            "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\n\
                             Content-Range: bytes {start}-{end}/{}\r\n\r\n",
                            chunk.len(),
                            body.len(),
                        );
                        sock.write_all(header.as_bytes()).unwrap();
                        sock.write_all(chunk).unwrap();
                    } else {
                        let header =
                            format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len());
                        sock.write_all(header.as_bytes()).unwrap();
                        sock.write_all(body).unwrap();
                    }
                }
            }
        }
    }

    #[test]
    fn s3_round_trip_full_fetch() {
        let server = MiniS3::new().with_object("slab.bin", b"hello s3".to_vec());
        let addr = server.local_addr();
        let endpoint = format!("http://{addr}");

        let handle = std::thread::spawn(move || server.serve_fully());
        let s3 = S3Locator::with_endpoint(endpoint);
        let data = s3.fetch("s3://bucket/slab.bin").expect("fetch");
        handle.join().unwrap();
        assert_eq!(data, b"hello s3");
    }

    #[test]
    fn s3_round_trip_range_fetch() {
        let payload = b"0123456789ABCDEFGHIJ".to_vec();
        let server = MiniS3::new().with_object("slab.bin", payload);
        let addr = server.local_addr();
        let endpoint = format!("http://{addr}");

        let handle = std::thread::spawn(move || server.serve_fully());
        let s3 = S3Locator::with_endpoint(endpoint);
        let data = s3
            .fetch_range("s3://bucket/slab.bin", 5, 10)
            .expect("fetch_range");
        handle.join().unwrap();
        assert_eq!(data, b"56789ABCDE");
    }

    #[test]
    fn s3_returns_not_found_for_missing_object() {
        let server = MiniS3::new();
        let addr = server.local_addr();
        let endpoint = format!("http://{addr}");

        let handle = std::thread::spawn(move || server.serve_fully());
        let s3 = S3Locator::with_endpoint(endpoint);
        match s3.fetch("s3://bucket/no-such.bin") {
            Err(LocatorError::NotFound) => {}
            other => panic!("expected NotFound, got {other:?}"),
        }
        handle.join().unwrap();
    }
}
