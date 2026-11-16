//! Id minting. Contracts say ids are UUIDv7 text (common.proto).

/// Mints ids. Time is passed in; the core never reads a clock.
pub trait IdSource {
    fn next_id(&mut self, now_ms: i64) -> String;
}

/// Format a UUIDv7: 48-bit big-endian unix-ms timestamp, then version 7,
/// then variant `10`, the rest taken from `random`. Lowercase 8-4-4-4-12.
pub fn format_uuid7(now_ms: i64, random: [u8; 10]) -> String {
    let ms = now_ms.max(0) as u64 & 0xFFFF_FFFF_FFFF;
    let mut b = [0_u8; 16];
    b[..6].copy_from_slice(&ms.to_be_bytes()[2..]);
    b[6] = 0x70 | (random[0] & 0x0F);
    b[7] = random[1];
    b[8] = 0x80 | (random[2] & 0x3F);
    b[9..].copy_from_slice(&random[3..]);
    let mut out = String::with_capacity(36);
    for (i, byte) in b.iter().enumerate() {
        if matches!(i, 4 | 6 | 8 | 10) {
            out.push('-');
        }
        out.push(HEX[usize::from(byte >> 4)] as char);
        out.push(HEX[usize::from(byte & 0x0F)] as char);
    }
    out
}

const HEX: &[u8; 16] = b"0123456789abcdef";

/// Production ids: random bits from the operating system. Ids are not
/// secrets, so if the OS call fails the random bits come from a per-process
/// counter instead (still unique within this process).
#[derive(Debug, Default)]
pub struct Uuid7Ids {
    fallback: u64,
}

impl Uuid7Ids {
    pub fn new() -> Self {
        Self::default()
    }
}

impl IdSource for Uuid7Ids {
    fn next_id(&mut self, now_ms: i64) -> String {
        let mut random = [0_u8; 10];
        if getrandom::fill(&mut random).is_err() {
            self.fallback = self.fallback.wrapping_add(1);
            random[2..].copy_from_slice(&self.fallback.to_be_bytes());
        }
        format_uuid7(now_ms, random)
    }
}

/// Deterministic ids for tests: a counter in the random bits.
#[derive(Debug, Default)]
pub struct SequentialIds {
    counter: u64,
}

impl SequentialIds {
    pub fn new() -> Self {
        Self::default()
    }
}

impl IdSource for SequentialIds {
    fn next_id(&mut self, now_ms: i64) -> String {
        self.counter += 1;
        let mut random = [0_u8; 10];
        random[2..].copy_from_slice(&self.counter.to_be_bytes());
        format_uuid7(now_ms, random)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check_shape(id: &str) {
        assert_eq!(id.len(), 36);
        for (i, c) in id.chars().enumerate() {
            if matches!(i, 8 | 13 | 18 | 23) {
                assert_eq!(c, '-', "{id}");
            } else {
                assert!(
                    c.is_ascii_digit() || ('a'..='f').contains(&c),
                    "bad char in {id}"
                );
            }
        }
        assert_eq!(id.as_bytes()[14], b'7', "version nibble in {id}");
        assert!(
            matches!(id.as_bytes()[19], b'8' | b'9' | b'a' | b'b'),
            "variant bits in {id}"
        );
    }

    #[test]
    fn format_version_variant_and_timestamp() {
        let id = format_uuid7(0x0123_4567_89AB, [0xFF; 10]);
        check_shape(&id);
        assert!(id.starts_with("01234567-89ab-7"), "{id}");
        let id = format_uuid7(1, [0; 10]);
        assert_eq!(id, "00000000-0001-7000-8000-000000000000");
    }

    #[test]
    fn negative_and_oversized_times_do_not_panic() {
        check_shape(&format_uuid7(-5, [1; 10]));
        check_shape(&format_uuid7(i64::MAX, [1; 10]));
    }

    #[test]
    fn production_ids_are_well_formed_and_distinct() {
        let mut ids = Uuid7Ids::new();
        let a = ids.next_id(1_700_000_000_000);
        let b = ids.next_id(1_700_000_000_000);
        check_shape(&a);
        check_shape(&b);
        assert_ne!(a, b);
        // 1_700_000_000_000 ms = 0x018B_CFE5_6800
        assert!(a.starts_with("018bcfe5-6800-7"), "{a}");
    }

    #[test]
    fn sequential_ids_are_deterministic() {
        let mut a = SequentialIds::new();
        let mut b = SequentialIds::new();
        let (a1, b1) = (a.next_id(5), b.next_id(5));
        assert_eq!(a1, b1);
        let a2 = a.next_id(5);
        assert_ne!(a1, a2);
        check_shape(&a2);
    }
}
