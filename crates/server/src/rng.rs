//! Randomness for dice. The only place a production die gets its entropy.

use tt_rules::dice::Rng;

/// A random source that can report that the operating system failed to
/// provide entropy. The core checks [`RollSource::take_failure`] after every
/// roll and rejects the intent rather than ever using a weak value.
pub trait RollSource: Rng {
    /// True if any value handed out since the last call was not real
    /// entropy. Clears the flag.
    fn take_failure(&mut self) -> bool;
}

/// Production source: 8 bytes from the operating system per call.
#[derive(Debug, Default)]
pub struct OsRolls {
    failed: bool,
}

impl OsRolls {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Rng for OsRolls {
    fn next_u64(&mut self) -> u64 {
        let mut bytes = [0_u8; 8];
        match getrandom::fill(&mut bytes) {
            Ok(()) => u64::from_le_bytes(bytes),
            Err(_) => {
                // The value is meaningless; the caller discards the roll.
                self.failed = true;
                0
            }
        }
    }
}

impl RollSource for OsRolls {
    fn take_failure(&mut self) -> bool {
        std::mem::take(&mut self.failed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn os_source_works_and_reports_no_failure() {
        let mut r = OsRolls::new();
        let a = r.next_u64();
        let b = r.next_u64();
        assert!(!r.take_failure());
        // 2^-64 chance of a false failure.
        assert_ne!(a, b);
    }
}
