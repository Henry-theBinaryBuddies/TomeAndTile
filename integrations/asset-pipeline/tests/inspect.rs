//! Tests against real sample images in `tests/samples/` plus hostile ones built in-line, per
//! issue #33's acceptance tests.

use tt_asset_pipeline::{inspect, AssetError, MAX_DIMENSION, MAX_FILE_BYTES, MAX_PIXELS};

fn sample(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/samples/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap_or_else(|error| panic!("reading tests/samples/{name}: {error}"))
}

#[test]
fn a_real_png_token_is_accepted_with_its_real_dimensions() {
    let asset = inspect(&sample("token.png")).expect("token.png should be accepted");
    assert_eq!(asset.media_type, "image/png");
    assert_eq!((asset.width, asset.height), (64, 64));
    assert_eq!(asset.url, "");
    assert_eq!(asset.asset_id.len(), 64);
}

#[test]
fn a_one_pixel_png_is_accepted() {
    let asset = inspect(&sample("tiny.png")).expect("tiny.png should be accepted");
    assert_eq!((asset.width, asset.height), (1, 1));
}

#[test]
fn a_real_jpeg_map_is_accepted_with_its_real_dimensions() {
    let asset = inspect(&sample("map.jpg")).expect("map.jpg should be accepted");
    assert_eq!(asset.media_type, "image/jpeg");
    assert_eq!((asset.width, asset.height), (200, 120));
}

#[test]
fn a_real_lossy_webp_icon_is_accepted_with_its_real_dimensions() {
    let asset = inspect(&sample("icon-lossy.webp")).expect("icon-lossy.webp should be accepted");
    assert_eq!(asset.media_type, "image/webp");
    assert_eq!((asset.width, asset.height), (32, 32));
}

#[test]
fn a_real_lossless_webp_icon_is_accepted_with_its_real_dimensions() {
    let asset =
        inspect(&sample("icon-lossless.webp")).expect("icon-lossless.webp should be accepted");
    assert_eq!(asset.media_type, "image/webp");
    assert_eq!((asset.width, asset.height), (32, 32));
}

#[test]
fn two_different_real_files_get_two_different_asset_ids() {
    let a = inspect(&sample("token.png")).unwrap();
    let b = inspect(&sample("map.jpg")).unwrap();
    assert_ne!(a.asset_id, b.asset_id);
}

#[test]
fn a_png_renamed_as_jpg_is_still_read_as_a_png_by_its_bytes() {
    // The function takes only bytes, never a file name, so this is really a sanity check that
    // byte content, not any extension, decides the format. A real "wrong extension" file would
    // just be these same bytes saved under a different name.
    let asset = inspect(&sample("token.png")).unwrap();
    assert_eq!(asset.media_type, "image/png");
}

#[test]
fn a_png_truncated_right_after_the_signature_is_rejected_not_panicked_on() {
    let bytes = sample("token.png");
    let truncated = &bytes[..8];
    assert_eq!(inspect(truncated), Err(AssetError::TruncatedHeader));
}

#[test]
fn a_png_truncated_mid_ihdr_is_rejected_not_panicked_on() {
    let bytes = sample("token.png");
    let truncated = &bytes[..20];
    assert_eq!(inspect(truncated), Err(AssetError::TruncatedHeader));
}

#[test]
fn a_jpeg_truncated_right_after_soi_is_rejected_not_panicked_on() {
    let bytes = sample("map.jpg");
    let truncated = &bytes[..2];
    assert_eq!(inspect(truncated), Err(AssetError::TruncatedHeader));
}

#[test]
fn a_webp_truncated_before_its_chunk_payload_is_rejected_not_panicked_on() {
    let bytes = sample("icon-lossy.webp");
    let truncated = &bytes[..16];
    assert_eq!(inspect(truncated), Err(AssetError::TruncatedHeader));
}

#[test]
fn a_header_claiming_dimensions_over_the_limit_is_rejected() {
    let mut bytes = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    bytes.extend_from_slice(&[0, 0, 0, 13]);
    bytes.extend_from_slice(b"IHDR");
    bytes.extend_from_slice(&(MAX_DIMENSION + 1).to_be_bytes());
    bytes.extend_from_slice(&1u32.to_be_bytes());
    assert_eq!(
        inspect(&bytes),
        Err(AssetError::InvalidDimensions {
            width: MAX_DIMENSION + 1,
            height: 1
        })
    );
}

#[test]
fn dimensions_each_under_the_limit_but_over_the_pixel_budget_are_rejected() {
    // 16000 x 16000 = 256,000,000 pixels: both dimensions legal alone, product well over
    // MAX_PIXELS (100,000,000).
    let width = 16_000u32;
    let height = 16_000u32;
    assert!(width <= MAX_DIMENSION && height <= MAX_DIMENSION);
    assert!((width as u64) * (height as u64) > MAX_PIXELS);

    let mut bytes = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    bytes.extend_from_slice(&[0, 0, 0, 13]);
    bytes.extend_from_slice(b"IHDR");
    bytes.extend_from_slice(&width.to_be_bytes());
    bytes.extend_from_slice(&height.to_be_bytes());
    match inspect(&bytes) {
        Err(AssetError::TooManyPixels { pixels, max_pixels }) => {
            assert_eq!(pixels, 256_000_000);
            assert_eq!(max_pixels, MAX_PIXELS);
        }
        other => panic!("expected TooManyPixels, got {other:?}"),
    }
}

#[test]
fn an_oversized_file_is_rejected_by_size_alone_before_any_format_parsing() {
    let bytes = vec![0u8; (MAX_FILE_BYTES + 1) as usize];
    assert_eq!(
        inspect(&bytes),
        Err(AssetError::TooLarge {
            max_bytes: MAX_FILE_BYTES,
            actual_bytes: MAX_FILE_BYTES + 1
        })
    );
}

#[test]
fn an_empty_file_is_rejected() {
    assert_eq!(inspect(&[]), Err(AssetError::Empty));
}

#[test]
fn completely_random_bytes_are_rejected_as_unknown_not_panicked_on() {
    // A fixed pseudo-random-looking sequence, not any recognized magic bytes.
    let bytes: Vec<u8> = (0u32..4096)
        .map(|i| ((i.wrapping_mul(2654435761u32)) >> 24) as u8)
        .collect();
    assert_eq!(inspect(&bytes), Err(AssetError::UnknownFormat));
}

#[test]
fn an_svg_is_rejected_even_with_a_png_like_byte_or_two_nearby() {
    let bytes =
        b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<svg onload=\"alert(1)\"></svg>".to_vec();
    assert_eq!(
        inspect(&bytes),
        Err(AssetError::DisallowedFormat {
            media_type: "image/svg+xml"
        })
    );
}

#[test]
fn nothing_panics_across_a_spread_of_hostile_short_inputs() {
    let cases: Vec<Vec<u8>> = vec![
        vec![],
        vec![0u8; 1],
        vec![0xFFu8; 1],
        vec![0x89, 0x50, 0x4E, 0x47], // PNG signature, cut off
        vec![0xFF, 0xD8],             // JPEG SOI, cut off
        b"RIFF".to_vec(),             // WebP container, cut off
        b"RIFF\0\0\0\0WEBP".to_vec(), // WebP container, no chunk
        vec![0xFFu8; 4096],           // all 0xFF: looks JPEG-ish but has no valid markers
    ];
    for bytes in cases {
        let _ = inspect(&bytes); // must return, never panic
    }
}
