//! JPEG header sniffing: walk the marker segments until a Start-Of-Frame (SOF) marker gives the
//! dimensions, without decoding any entropy-coded scan data.

use crate::AssetError;

/// `None` if `bytes` does not start with the JPEG SOI marker (not a JPEG).
pub fn sniff(bytes: &[u8]) -> Option<Result<(u32, u32), AssetError>> {
    if bytes.len() < 2 || bytes[0] != 0xFF || bytes[1] != 0xD8 {
        return None;
    }

    let mut pos = 2;
    loop {
        match read_segment(bytes, pos) {
            Ok(Segment::StartOfFrame { width, height }) => return Some(Ok((width, height))),
            Ok(Segment::Skip { next }) => pos = next,
            Err(error) => return Some(Err(error)),
        }
    }
}

enum Segment {
    StartOfFrame { width: u32, height: u32 },
    Skip { next: usize },
}

fn read_segment(bytes: &[u8], pos: usize) -> Result<Segment, AssetError> {
    if pos + 1 >= bytes.len() {
        return Err(AssetError::TruncatedHeader);
    }
    if bytes[pos] != 0xFF {
        // Not at a marker where one was expected: not a structure we can read reliably.
        return Err(AssetError::TruncatedHeader);
    }
    let marker = bytes[pos + 1];
    let after_marker = pos + 2;

    // Standalone markers that carry no length: TEM, the RSTn run and EOI.
    if marker == 0x01 || (0xD0..=0xD9).contains(&marker) {
        if marker == 0xD9 {
            // Hit End-Of-Image without ever finding a Start-Of-Frame.
            return Err(AssetError::TruncatedHeader);
        }
        return Ok(Segment::Skip { next: after_marker });
    }

    if after_marker + 1 >= bytes.len() {
        return Err(AssetError::TruncatedHeader);
    }
    let seg_len = u16::from_be_bytes([bytes[after_marker], bytes[after_marker + 1]]) as usize;
    if seg_len < 2 {
        return Err(AssetError::TruncatedHeader);
    }

    let is_start_of_frame = matches!(marker, 0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF);
    if is_start_of_frame {
        // Segment payload: length(2, already read) + precision(1) + height(2) + width(2) + ...
        if seg_len < 7 || after_marker + seg_len > bytes.len() {
            return Err(AssetError::TruncatedHeader);
        }
        let height = u16::from_be_bytes([bytes[after_marker + 3], bytes[after_marker + 4]]) as u32;
        let width = u16::from_be_bytes([bytes[after_marker + 5], bytes[after_marker + 6]]) as u32;
        return Ok(Segment::StartOfFrame { width, height });
    }

    let next = after_marker + seg_len;
    if next > bytes.len() {
        return Err(AssetError::TruncatedHeader);
    }
    Ok(Segment::Skip { next })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_jpeg_bytes_are_not_claimed() {
        assert!(sniff(b"not a jpeg at all").is_none());
    }

    #[test]
    fn soi_with_nothing_after_it_is_truncated() {
        assert_eq!(sniff(&[0xFF, 0xD8]), Some(Err(AssetError::TruncatedHeader)));
    }

    #[test]
    fn an_eoi_with_no_start_of_frame_is_truncated() {
        assert_eq!(
            sniff(&[0xFF, 0xD8, 0xFF, 0xD9]),
            Some(Err(AssetError::TruncatedHeader))
        );
    }

    #[test]
    fn a_minimal_hand_built_sof0_segment_reads_back_its_dimensions() {
        // SOI, then a SOF0 segment: marker(C0) len(0008, includes itself) precision(08)
        // height(0010=16) width(0014=20) component_count(00, not read).
        let bytes = [
            0xFF, 0xD8, 0xFF, 0xC0, 0x00, 0x08, 0x08, 0x00, 0x10, 0x00, 0x14, 0x00,
        ];
        assert_eq!(sniff(&bytes), Some(Ok((20, 16))));
    }
}
