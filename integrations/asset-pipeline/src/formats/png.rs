//! PNG header sniffing: signature + the mandatory first (`IHDR`) chunk. No full decode.

use crate::AssetError;

const SIGNATURE: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];

/// `None` if `bytes` does not start with the PNG signature (not a PNG). `Some(Err(..))` if it
/// does but the header is too short or malformed to read a size from.
pub fn sniff(bytes: &[u8]) -> Option<Result<(u32, u32), AssetError>> {
    if !bytes.starts_with(&SIGNATURE) {
        return None;
    }
    // Signature(8) + chunk length(4) + chunk type "IHDR"(4) + width(4) + height(4) = 24.
    if bytes.len() < 24 {
        return Some(Err(AssetError::TruncatedHeader));
    }
    if &bytes[12..16] != b"IHDR" {
        // The PNG spec requires IHDR to be the first chunk; anything else is not a PNG we
        // can trust the size of.
        return Some(Err(AssetError::TruncatedHeader));
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
    let height = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
    Some(Ok((width, height)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_png_bytes_are_not_claimed() {
        assert!(sniff(b"not a png at all").is_none());
    }

    #[test]
    fn a_signature_with_nothing_after_it_is_truncated() {
        assert_eq!(sniff(&SIGNATURE), Some(Err(AssetError::TruncatedHeader)));
    }
}
