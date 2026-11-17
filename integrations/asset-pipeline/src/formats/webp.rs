//! WebP header sniffing: the RIFF/WEBP container plus whichever of the three chunk shapes
//! (`VP8 ` lossy, `VP8L` lossless, `VP8X` extended) holds the size. No full decode.

use crate::AssetError;

/// `None` if `bytes` is not a RIFF/WEBP container at all.
pub fn sniff(bytes: &[u8]) -> Option<Result<(u32, u32), AssetError>> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WEBP" {
        return None;
    }
    // RIFF header(12) + chunk fourcc(4) + chunk size(4) = 20, before the chunk's own payload.
    if bytes.len() < 20 {
        return Some(Err(AssetError::TruncatedHeader));
    }
    let fourcc = &bytes[12..16];
    let payload = &bytes[20..];

    Some(match fourcc {
        b"VP8 " => sniff_lossy(payload),
        b"VP8L" => sniff_lossless(payload),
        b"VP8X" => sniff_extended(payload),
        _ => Err(AssetError::TruncatedHeader),
    })
}

fn sniff_lossy(payload: &[u8]) -> Result<(u32, u32), AssetError> {
    // Frame tag(3) + start code(3, 0x9d012a) + width(2 LE, low 14 bits) + height(2 LE, low 14 bits).
    if payload.len() < 10 {
        return Err(AssetError::TruncatedHeader);
    }
    if payload[3..6] != [0x9d, 0x01, 0x2a] {
        return Err(AssetError::TruncatedHeader);
    }
    let width = u16::from_le_bytes([payload[6], payload[7]]) & 0x3FFF;
    let height = u16::from_le_bytes([payload[8], payload[9]]) & 0x3FFF;
    Ok((width as u32, height as u32))
}

fn sniff_lossless(payload: &[u8]) -> Result<(u32, u32), AssetError> {
    // Signature(1, 0x2f) + 4 bytes little-endian bit-packed: 14 bits width-1, 14 bits height-1,
    // then the alpha flag and version bits (not needed here).
    if payload.len() < 5 || payload[0] != 0x2F {
        return Err(AssetError::TruncatedHeader);
    }
    let bits = u32::from_le_bytes(payload[1..5].try_into().unwrap());
    let width = (bits & 0x3FFF) + 1;
    let height = ((bits >> 14) & 0x3FFF) + 1;
    Ok((width, height))
}

fn sniff_extended(payload: &[u8]) -> Result<(u32, u32), AssetError> {
    // Flags(1) + reserved(3) + canvas width-1(3, LE) + canvas height-1(3, LE).
    if payload.len() < 10 {
        return Err(AssetError::TruncatedHeader);
    }
    let width = u24_le(&payload[4..7]) + 1;
    let height = u24_le(&payload[7..10]) + 1;
    Ok((width, height))
}

fn u24_le(bytes: &[u8]) -> u32 {
    bytes[0] as u32 | (bytes[1] as u32) << 8 | (bytes[2] as u32) << 16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_webp_bytes_are_not_claimed() {
        assert!(sniff(b"not a webp at all, but long enough").is_none());
    }

    #[test]
    fn a_riff_webp_header_with_no_chunk_is_truncated() {
        let mut bytes = b"RIFF".to_vec();
        bytes.extend_from_slice(&[0, 0, 0, 0]);
        bytes.extend_from_slice(b"WEBP");
        assert_eq!(sniff(&bytes), Some(Err(AssetError::TruncatedHeader)));
    }

    #[test]
    fn an_unknown_chunk_fourcc_is_rejected_rather_than_misread() {
        let mut bytes = b"RIFF".to_vec();
        bytes.extend_from_slice(&[0, 0, 0, 0]);
        bytes.extend_from_slice(b"WEBP");
        bytes.extend_from_slice(b"ANIM");
        bytes.extend_from_slice(&[0, 0, 0, 0]);
        assert_eq!(sniff(&bytes), Some(Err(AssetError::TruncatedHeader)));
    }
}
