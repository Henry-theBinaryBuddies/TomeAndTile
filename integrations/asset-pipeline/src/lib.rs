//! Asset pipeline, first half (issue #33): takes the bytes of an uploaded map or token image,
//! decides whether to accept it, and describes it as an [`tt_contracts::v1::AssetRef`]. Storing
//! the file and serving it (`url`) come later; see integrations/CLAUDE.md and ADR 0010.
//!
//! [`inspect`] is a pure function: no network, no disk, no clock. It never decodes a full image,
//! only each format's fixed-size header, so a hostile multi-megapixel claim costs nothing to
//! reject.

mod formats;
mod sha256;

use std::fmt;

use tt_contracts::v1::AssetRef;

/// A file must be no larger than this to be inspected at all.
pub const MAX_FILE_BYTES: u64 = 20 * 1024 * 1024;
/// Neither dimension may exceed this.
pub const MAX_DIMENSION: u32 = 16384;
/// The product of width and height may not exceed this.
pub const MAX_PIXELS: u64 = 100_000_000;

/// Why a file was rejected. `code()` gives the stable string the caller can match on or log;
/// `Display` gives a human-readable message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssetError {
    Empty,
    TooLarge { max_bytes: u64, actual_bytes: u64 },
    UnknownFormat,
    DisallowedFormat { media_type: &'static str },
    TruncatedHeader,
    InvalidDimensions { width: u32, height: u32 },
    TooManyPixels { pixels: u64, max_pixels: u64 },
}

impl AssetError {
    /// A stable code safe to match on, log or show to a GM, independent of the message wording.
    pub fn code(&self) -> &'static str {
        match self {
            AssetError::Empty => "empty_file",
            AssetError::TooLarge { .. } => "file_too_large",
            AssetError::UnknownFormat => "unknown_format",
            AssetError::DisallowedFormat { .. } => "disallowed_format",
            AssetError::TruncatedHeader => "truncated_header",
            AssetError::InvalidDimensions { .. } => "invalid_dimensions",
            AssetError::TooManyPixels { .. } => "too_many_pixels",
        }
    }
}

impl fmt::Display for AssetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AssetError::Empty => write!(f, "the file is empty"),
            AssetError::TooLarge { max_bytes, actual_bytes } => {
                write!(f, "the file is {actual_bytes} bytes, over the {max_bytes}-byte limit")
            }
            AssetError::UnknownFormat => {
                write!(f, "the file's bytes do not match a supported image format")
            }
            AssetError::DisallowedFormat { media_type } => {
                write!(f, "{media_type} is not an accepted format")
            }
            AssetError::TruncatedHeader => {
                write!(f, "the file's header is missing, truncated or malformed")
            }
            AssetError::InvalidDimensions { width, height } => {
                write!(f, "the image is {width}x{height}, outside the accepted range")
            }
            AssetError::TooManyPixels { pixels, max_pixels } => {
                write!(f, "the image is {pixels} pixels, over the {max_pixels}-pixel limit")
            }
        }
    }
}

impl std::error::Error for AssetError {}

/// Inspects the bytes of an uploaded file and either accepts it, returning an [`AssetRef`] with
/// `url` left empty (set later, once the file is stored), or rejects it with a reason.
///
/// Pure: no network, no disk, no clock. Never panics on any input, however malformed or hostile.
pub fn inspect(bytes: &[u8]) -> Result<AssetRef, AssetError> {
    if bytes.is_empty() {
        return Err(AssetError::Empty);
    }
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(AssetError::TooLarge {
            max_bytes: MAX_FILE_BYTES,
            actual_bytes: bytes.len() as u64,
        });
    }
    if looks_like_svg(bytes) {
        return Err(AssetError::DisallowedFormat { media_type: "image/svg+xml" });
    }

    let (media_type, width, height) = formats::sniff(bytes)?;

    if width == 0 || height == 0 || width > MAX_DIMENSION || height > MAX_DIMENSION {
        return Err(AssetError::InvalidDimensions { width, height });
    }
    let pixels = width as u64 * height as u64;
    if pixels > MAX_PIXELS {
        return Err(AssetError::TooManyPixels { pixels, max_pixels: MAX_PIXELS });
    }

    Ok(AssetRef {
        asset_id: sha256::hex_digest(bytes),
        media_type: media_type.to_string(),
        url: String::new(),
        width,
        height,
    })
}

/// A cheap, deliberately loose check: anything that looks like it might be read as SVG/XML (and
/// so could carry a `<script>`) is rejected outright, rather than trying to tell a safe SVG from
/// an unsafe one. Scans only the first kilobyte, in case of a leading BOM or whitespace.
fn looks_like_svg(bytes: &[u8]) -> bool {
    let head = &bytes[..bytes.len().min(1024)];
    let text = String::from_utf8_lossy(head).to_ascii_lowercase();
    let trimmed = text.trim_start_matches(|c: char| c == '\u{feff}' || c == '\u{0}').trim_start();
    trimmed.starts_with("<?xml") || trimmed.starts_with("<svg") || text.contains("<svg")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_file_is_rejected() {
        assert_eq!(inspect(&[]), Err(AssetError::Empty));
    }

    #[test]
    fn a_file_over_the_size_limit_is_rejected_before_any_parsing() {
        let bytes = vec![0u8; (MAX_FILE_BYTES + 1) as usize];
        assert_eq!(
            inspect(&bytes),
            Err(AssetError::TooLarge { max_bytes: MAX_FILE_BYTES, actual_bytes: MAX_FILE_BYTES + 1 })
        );
    }

    #[test]
    fn random_bytes_are_an_unknown_format() {
        let bytes: Vec<u8> = (0..256u32).map(|b| b as u8).collect();
        assert_eq!(inspect(&bytes), Err(AssetError::UnknownFormat));
    }

    #[test]
    fn svg_is_rejected_regardless_of_case_or_leading_whitespace() {
        assert_eq!(
            inspect(b"  \n<SVG xmlns=\"http://www.w3.org/2000/svg\"></SVG>"),
            Err(AssetError::DisallowedFormat { media_type: "image/svg+xml" })
        );
        assert_eq!(
            inspect(b"<?xml version=\"1.0\"?><svg></svg>"),
            Err(AssetError::DisallowedFormat { media_type: "image/svg+xml" })
        );
    }

    #[test]
    fn a_header_claiming_huge_dimensions_is_rejected_without_needing_the_real_pixels() {
        // A hand-built PNG IHDR claiming 20000x20000 (over MAX_DIMENSION), backed by only the
        // 24-byte header. If this decoded the image it would need ~1.2GB; it must not even try.
        let mut bytes = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        bytes.extend_from_slice(&[0, 0, 0, 13]); // chunk length (not actually validated here)
        bytes.extend_from_slice(b"IHDR");
        bytes.extend_from_slice(&20_000u32.to_be_bytes());
        bytes.extend_from_slice(&20_000u32.to_be_bytes());
        assert_eq!(
            inspect(&bytes),
            Err(AssetError::InvalidDimensions { width: 20_000, height: 20_000 })
        );
    }

    #[test]
    fn a_zero_dimension_is_rejected() {
        let mut bytes = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        bytes.extend_from_slice(&[0, 0, 0, 13]);
        bytes.extend_from_slice(b"IHDR");
        bytes.extend_from_slice(&0u32.to_be_bytes());
        bytes.extend_from_slice(&64u32.to_be_bytes());
        assert_eq!(inspect(&bytes), Err(AssetError::InvalidDimensions { width: 0, height: 64 }));
    }

    #[test]
    fn a_truncated_png_header_is_rejected_not_panicked_on() {
        let bytes = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0, 0];
        assert_eq!(inspect(&bytes), Err(AssetError::TruncatedHeader));
    }

    #[test]
    fn the_same_bytes_always_get_the_same_asset_id() {
        let bytes = sample_png(4, 4);
        let first = inspect(&bytes).unwrap();
        let second = inspect(&bytes).unwrap();
        assert_eq!(first.asset_id, second.asset_id);
        assert_eq!(first.asset_id.len(), 64);
        assert!(first.asset_id.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    }

    #[test]
    fn url_is_left_empty() {
        let bytes = sample_png(4, 4);
        assert_eq!(inspect(&bytes).unwrap().url, "");
    }

    /// A minimal, real (CRC-correct is not required here since this crate never checks it)
    /// one-chunk-header PNG, for tests that need *some* valid PNG bytes but not a real image.
    fn sample_png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        bytes.extend_from_slice(&[0, 0, 0, 13]);
        bytes.extend_from_slice(b"IHDR");
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes
    }
}
