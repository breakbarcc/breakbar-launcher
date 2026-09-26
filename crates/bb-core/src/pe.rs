//! Minimal PE (Portable Executable) header inspection.

/// Number of bytes from the start of a file that [`machine`] needs in the common case.
pub const HEADER_PROBE_LEN: usize = 4096;

/// `IMAGE_FILE_MACHINE_AMD64`.
pub const MACHINE_AMD64: u16 = 0x8664;

/// Returns the target machine of a PE image, read from the start of the file.
///
/// Returns `None` if `header` is not a PE image or is too short to contain the PE header.
#[must_use]
pub fn machine(header: &[u8]) -> Option<u16> {
    if header.get(..2)? != b"MZ" {
        return None;
    }
    let pe_offset = u32::from_le_bytes(header.get(0x3C..0x40)?.try_into().ok()?) as usize;
    if header.get(pe_offset..pe_offset.checked_add(4)?)? != b"PE\0\0" {
        return None;
    }
    let machine = header.get(pe_offset + 4..pe_offset + 6)?;
    Some(u16::from_le_bytes([machine[0], machine[1]]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(machine: u16) -> Vec<u8> {
        let mut data = vec![0u8; 0x100];
        data[..2].copy_from_slice(b"MZ");
        data[0x3C..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        data[0x80..0x84].copy_from_slice(b"PE\0\0");
        data[0x84..0x86].copy_from_slice(&machine.to_le_bytes());
        data
    }

    #[test]
    fn detects_x64_image() {
        assert_eq!(machine(&image(MACHINE_AMD64)), Some(MACHINE_AMD64));
    }

    #[test]
    fn detects_x86_image() {
        assert_eq!(machine(&image(0x014C)), Some(0x014C));
    }

    #[test]
    fn rejects_non_pe_data() {
        assert_eq!(machine(b"not an executable"), None);
        assert_eq!(machine(b""), None);
    }

    #[test]
    fn rejects_truncated_header() {
        assert_eq!(machine(&image(MACHINE_AMD64)[..0x82]), None);
    }

    #[test]
    fn rejects_out_of_range_offset() {
        let mut data = image(MACHINE_AMD64);
        data[0x3C..0x40].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(machine(&data), None);
    }
}
