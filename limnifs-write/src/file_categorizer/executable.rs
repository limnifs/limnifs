//! Executable categorizer — routes ELF / PE / Mach-O binaries to
//! BCJ-x86 or BCJ-ARM64 composite codecs.
//!
//! ## Algorithm
//!
//! 1. Detect executable format by magic bytes at offset 0:
//!    - ELF: `\x7FELF`
//!    - PE:  `MZ` then PE header at offset stored at 0x3C
//!    - Mach-O: `0xFEEDFACE`, `0xFEEDFACF`, `0xCEFAEDFE`, `0xCFFAEDFE`
//! 2. Read architecture from the format-specific header field:
//!    - ELF: `e_machine` at offset 18 (u16 LE)
//!      - `0x3E` = x86_64 → BCJ-x86
//!      - `0xB7` = aarch64 → BCJ-ARM64
//!    - PE: `Machine` at offset 0 in PE header (u16 LE)
//!      - `0x8664` = x86_64 → BCJ-x86
//!      - `0xAA64` = aarch64 → BCJ-ARM64
//!    - Mach-O: `cputype` at offset 4 (u32 LE for native-endian magic)
//!      - `0x01000007` = x86_64 → BCJ-x86
//!      - `0x0100000C` = arm64 → BCJ-ARM64
//! 3. Pick the LZ4 variant for write-heavy profiles (faster encode)
//!    and the ZSTD variant otherwise. We pick LZ4 by default for
//!    speed; the tournament in `process_whole_file_drop` will
//!    re-evaluate against ZSTD and pick the smaller result.
//!
//! ## Coverage
//!
//! All four major executable formats × the two architectures that
//! have a BCJ filter implementation in omnizip-filters today. 32-bit
//! ARM (no BCJ filter published), PowerPC, SPARC, IA-64 routes to
//! plain LZ4 (no benefit from a filter we don't have).

use std::path::Path;

use super::{Categorization, FileCategorizer};
use limnifs_core::codec::{CODEC_BCJ_ARM64_LZ4, CODEC_BCJ_X86_LZ4};

/// Minimum size worth running through BCJ + categorizer overhead.
const MIN_EXEC_SIZE: usize = 1024;

/// ELF magic.
const ELF_MAGIC: [u8; 4] = [0x7F, b'E', b'L', b'F'];
/// DOS/PE magic (MZ).
const DOS_MAGIC: [u8; 2] = [b'M', b'Z'];
/// Mach-O magics (4 bytes each, big- and little-endian).
const MACHO_MAGICS: &[[u8; 4]] = &[
    [0xFE, 0xED, 0xFA, 0xCE], // 32-bit native-endian
    [0xFE, 0xED, 0xFA, 0xCF], // 64-bit native-endian
    [0xCE, 0xFA, 0xED, 0xFE], // 32-bit swapped-endian
    [0xCF, 0xFA, 0xED, 0xFE], // 64-bit swapped-endian
];

/// ELF e_machine values we route.
const EM_X86_64: u16 = 0x3E;
const EM_AARCH64: u16 = 0xB7;

/// PE Machine values we route.
const PE_MACHINE_AMD64: u16 = 0x8664;
const PE_MACHINE_ARM64: u16 = 0xAA64;

/// Mach-O cputype values we route (CPU_TYPE_x86_64, CPU_TYPE_ARM64).
const CPU_TYPE_X86_64: u32 = 0x0100_0007;
const CPU_TYPE_ARM64: u32 = 0x0100_000C;

/// Pick a codec for the given executable bytes, or `None` if not a
/// recognized executable format.
fn pick_codec(data: &[u8]) -> Option<u8> {
    if data.len() < MIN_EXEC_SIZE {
        return None;
    }

    if data.starts_with(&ELF_MAGIC) {
        return parse_elf(data);
    }
    if data.starts_with(&DOS_MAGIC) {
        return parse_pe(data);
    }
    if MACHO_MAGICS.iter().any(|m| data.starts_with(m)) {
        return parse_macho(data);
    }
    None
}

fn parse_elf(data: &[u8]) -> Option<u8> {
    // ELF64 header layout: e_ident[16], e_type(2), e_machine(2), ...
    // e_machine is at offset 18, u16 LE on little-endian ELF
    // (EI_DATA byte at offset 5; we assume LE for now, which covers
    // every modern x86_64 / aarch64 binary).
    if data.len() < 20 {
        return None;
    }
    let machine = u16::from_le_bytes([data[18], data[19]]);
    route_by_arch(machine == EM_X86_64, machine == EM_AARCH64)
}

fn parse_pe(data: &[u8]) -> Option<u8> {
    // PE: e_lfanew at offset 0x3C (u32 LE) points to PE\0\0 header.
    // PE header: signature(4) + machine(2).
    if data.len() < 0x40 {
        return None;
    }
    let lfanew = u32::from_le_bytes([data[0x3C], data[0x3D], data[0x3E], data[0x3F]]) as usize;
    let pe_off = lfanew.checked_add(4)?;
    if data.len() < pe_off + 2 {
        return None;
    }
    // Confirm PE signature.
    if &data[lfanew..lfanew + 4] != b"PE\0\0" {
        return None;
    }
    let machine = u16::from_le_bytes([data[pe_off], data[pe_off + 1]]);
    route_by_arch(machine == PE_MACHINE_AMD64, machine == PE_MACHINE_ARM64)
}

fn parse_macho(data: &[u8]) -> Option<u8> {
    // Mach-O: magic(4), cputype(4), cpusubtype(4), filetype(4), ...
    // cputype interpretation depends on magic endianness, but the
    // constant value is the same in the file's native byte order.
    if data.len() < 12 {
        return None;
    }
    let magic = [data[0], data[1], data[2], data[3]];
    let is_le = matches!(magic, [0xFE, 0xED, 0xFA, 0xCE] | [0xFE, 0xED, 0xFA, 0xCF]);
    let cputype = if is_le {
        u32::from_le_bytes([data[4], data[5], data[6], data[7]])
    } else {
        u32::from_be_bytes([data[4], data[5], data[6], data[7]])
    };
    route_by_arch(cputype == CPU_TYPE_X86_64, cputype == CPU_TYPE_ARM64)
}

fn route_by_arch(x86_64: bool, arm64: bool) -> Option<u8> {
    if x86_64 {
        Some(CODEC_BCJ_X86_LZ4)
    } else if arm64 {
        Some(CODEC_BCJ_ARM64_LZ4)
    } else {
        None
    }
}

/// Categorizer for executable binaries. Detects ELF / PE / Mach-O
/// magic and routes x86_64 / aarch64 architectures to BCJ-x86 /
/// BCJ-ARM64 composite codecs.
pub struct ExecutableCategorizer;

impl FileCategorizer for ExecutableCategorizer {
    fn name(&self) -> &'static str {
        "executable"
    }

    fn categories(&self) -> &'static [&'static str] {
        &["binary/executable"]
    }

    fn categorize(&self, _path: &Path, data: &[u8]) -> Option<Categorization> {
        let codec_id = pick_codec(data)?;
        Some(Categorization {
            codec_id,
            codec_params: Vec::new(),
            category: "binary/executable",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn elf_x86_64() -> Vec<u8> {
        // Minimal ELF64 header prefix with e_machine = EM_X86_64,
        // padded to MIN_EXEC_SIZE so the categorizer accepts it.
        let mut v = vec![0u8; MIN_EXEC_SIZE];
        v[0..4].copy_from_slice(&ELF_MAGIC);
        v[4] = 2; // EI_CLASS = ELFCLASS64
        v[5] = 1; // EI_DATA = ELFDATA2LSB
        v[16..18].copy_from_slice(&2u16.to_le_bytes()); // e_type = ET_EXEC
        v[18..20].copy_from_slice(&EM_X86_64.to_le_bytes());
        v
    }

    fn elf_aarch64() -> Vec<u8> {
        let mut v = elf_x86_64();
        v[18..20].copy_from_slice(&EM_AARCH64.to_le_bytes());
        v
    }

    fn elf_unknown_arch() -> Vec<u8> {
        let mut v = elf_x86_64();
        v[18..20].copy_from_slice(&0x1234u16.to_le_bytes());
        v
    }

    #[test]
    fn detects_elf_x86_64() {
        let data = elf_x86_64();
        assert_eq!(
            pick_codec(&data),
            Some(CODEC_BCJ_X86_LZ4),
            "ELF x86_64 should route to BCJ-x86+LZ4"
        );
    }

    #[test]
    fn detects_elf_aarch64() {
        let data = elf_aarch64();
        assert_eq!(
            pick_codec(&data),
            Some(CODEC_BCJ_ARM64_LZ4),
            "ELF aarch64 should route to BCJ-ARM64+LZ4"
        );
    }

    #[test]
    fn unknown_arch_returns_none() {
        // Unknown architecture should NOT route — caller falls back
        // to plain FastCDC. BCJ on a different arch would corrupt
        // the binary.
        let data = elf_unknown_arch();
        assert_eq!(pick_codec(&data), None);
    }

    #[test]
    fn small_input_returns_none() {
        let mut v = elf_x86_64();
        v.truncate(100); // below MIN_EXEC_SIZE
        assert_eq!(pick_codec(&v), None);
    }

    #[test]
    fn pe_x86_64_routes_correctly() {
        // Construct a minimal DOS + PE header for x86_64.
        let mut v = vec![0u8; MIN_EXEC_SIZE];
        v[0..2].copy_from_slice(&DOS_MAGIC);
        let pe_offset: u32 = 0x40;
        v[0x3C..0x40].copy_from_slice(&pe_offset.to_le_bytes());
        v[0x40..0x44].copy_from_slice(b"PE\0\0");
        v[0x44..0x46].copy_from_slice(&PE_MACHINE_AMD64.to_le_bytes());
        assert_eq!(pick_codec(&v), Some(CODEC_BCJ_X86_LZ4));
    }

    #[test]
    fn macho_x86_64_routes_correctly() {
        let mut v = vec![0u8; MIN_EXEC_SIZE];
        v[0..4].copy_from_slice(&[0xFE, 0xED, 0xFA, 0xCF]); // MH_MAGIC_64
        v[4..8].copy_from_slice(&CPU_TYPE_X86_64.to_le_bytes());
        assert_eq!(pick_codec(&v), Some(CODEC_BCJ_X86_LZ4));
    }

    #[test]
    fn non_executable_returns_none() {
        assert_eq!(pick_codec(b"hello world text not exec"), None);
        assert_eq!(pick_codec(&vec![0u8; 4096]), None);
    }
}
