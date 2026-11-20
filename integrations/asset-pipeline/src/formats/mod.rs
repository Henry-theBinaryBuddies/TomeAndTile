mod jpeg;
mod png;
mod webp;

use crate::AssetError;

/// Reads the media type and dimensions from the file's own bytes. The first sniffer that
/// recognizes the magic bytes wins; `UnknownFormat` only if none do.
pub fn sniff(bytes: &[u8]) -> Result<(&'static str, u32, u32), AssetError> {
    if let Some(result) = png::sniff(bytes) {
        return result.map(|(width, height)| ("image/png", width, height));
    }
    if let Some(result) = jpeg::sniff(bytes) {
        return result.map(|(width, height)| ("image/jpeg", width, height));
    }
    if let Some(result) = webp::sniff(bytes) {
        return result.map(|(width, height)| ("image/webp", width, height));
    }
    Err(AssetError::UnknownFormat)
}
