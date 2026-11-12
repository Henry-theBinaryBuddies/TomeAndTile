//! Dice notation parsing and rolling for the mock server.
//!
//! The random source here is NOT suitable for the real server. It is a tiny
//! non-cryptographic generator seeded from the clock, with a small modulo
//! bias, chosen only so the mock needs no extra dependency. The real server
//! must roll with a proper source (ADR 0006).

use std::time::{SystemTime, UNIX_EPOCH};

use tt_contracts::v1::{Die, Modifier, RollMode, RollPurpose, RollResult, RollVisibility};

/// A source of random numbers. Tests inject a fixed seed.
pub trait Rng {
    fn next_u64(&mut self) -> u64;
}

/// xorshift64*: tiny, fast, and predictable. Mock use only.
pub struct XorShift64Star {
    state: u64,
}

impl XorShift64Star {
    /// A zero seed is replaced, since xorshift cannot leave the zero state.
    pub fn new(seed: u64) -> Self {
        Self {
            state: if seed == 0 {
                0x9E37_79B9_7F4A_7C15
            } else {
                seed
            },
        }
    }

    pub fn from_clock() -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(1);
        Self::new(nanos)
    }
}

impl Rng for XorShift64Star {
    fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
}

/// A parsed `NdM`, `NdM+K` or `NdM-K` expression.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Notation {
    pub count: u32,
    pub sides: u32,
    /// `Some` when the text carried a `+K` or `-K` part (even `+0`).
    pub modifier: Option<i32>,
}

impl Notation {
    /// Canonical text: no whitespace, lowercase `d`.
    pub fn canonical(&self) -> String {
        match self.modifier {
            Some(k) if k < 0 => format!("{}d{}{}", self.count, self.sides, k),
            Some(k) => format!("{}d{}+{}", self.count, self.sides, k),
            None => format!("{}d{}", self.count, self.sides),
        }
    }

    fn is_single_d20(&self) -> bool {
        self.count == 1 && self.sides == 20
    }
}

fn all_digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

/// Parse the supported notation; whitespace is allowed anywhere.
/// 1 <= N <= 100, M in {4,6,8,10,12,20,100}.
pub fn parse_notation(text: &str) -> Option<Notation> {
    let compact: String = text
        .chars()
        .filter(|c| !c.is_whitespace())
        .map(|c| c.to_ascii_lowercase())
        .collect();
    let (count_text, rest) = compact.split_once('d')?;
    if !all_digits(count_text) || count_text.len() > 3 {
        return None;
    }
    let count: u32 = count_text.parse().ok()?;
    if !(1..=100).contains(&count) {
        return None;
    }
    let digits_end = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    let (sides_text, tail) = rest.split_at(digits_end);
    if !all_digits(sides_text) || sides_text.len() > 3 {
        return None;
    }
    let sides: u32 = sides_text.parse().ok()?;
    if ![4, 6, 8, 10, 12, 20, 100].contains(&sides) {
        return None;
    }
    let modifier = if tail.is_empty() {
        None
    } else {
        let sign = match tail.as_bytes()[0] {
            b'+' => 1_i32,
            b'-' => -1_i32,
            _ => return None,
        };
        let digits = &tail[1..];
        if !all_digits(digits) {
            return None;
        }
        let magnitude: i32 = digits.parse().ok()?;
        Some(sign * magnitude)
    };
    Some(Notation {
        count,
        sides,
        modifier,
    })
}

/// Roll one die: uniform enough for a mock (modulo bias is ignored).
fn roll_die(rng: &mut dyn Rng, sides: u32) -> u32 {
    (rng.next_u64() % u64::from(sides)) as u32 + 1
}

/// Roll `notation` into a full `RollResult`, always PUBLIC. Advantage and
/// disadvantage apply only to a single d20; otherwise the mode is reported as
/// NORMAL. Returns `None` if the total does not fit an `i32`.
pub fn roll(
    rng: &mut dyn Rng,
    notation: &Notation,
    requested: RollMode,
    roll_id: String,
    roller_user_id: &str,
    label: String,
) -> Option<RollResult> {
    let single = notation.is_single_d20();
    let mode = match requested {
        RollMode::Advantage | RollMode::Disadvantage if single => requested,
        _ => RollMode::Normal,
    };
    let mut dice: Vec<Die> = Vec::new();
    if single && mode != RollMode::Normal {
        let first = roll_die(rng, 20);
        let second = roll_die(rng, 20);
        let keep_first = match mode {
            RollMode::Advantage => first >= second,
            _ => first <= second,
        };
        dice.push(Die {
            sides: 20,
            value: first,
            dropped: !keep_first,
        });
        dice.push(Die {
            sides: 20,
            value: second,
            dropped: keep_first,
        });
    } else {
        for _ in 0..notation.count {
            dice.push(Die {
                sides: notation.sides,
                value: roll_die(rng, notation.sides),
                dropped: false,
            });
        }
    }
    let kept_sum: i32 = dice
        .iter()
        .filter(|d| !d.dropped)
        .map(|d| d.value as i32)
        .sum();
    let total = kept_sum.checked_add(notation.modifier.unwrap_or(0))?;
    let kept_single = dice.iter().find(|d| !d.dropped).map(|d| d.value);
    let modifiers = notation
        .modifier
        .map(|value| {
            vec![Modifier {
                source: "modifier".to_string(),
                value,
            }]
        })
        .unwrap_or_default();
    Some(RollResult {
        roll_id,
        notation: notation.canonical(),
        purpose: RollPurpose::Freeform as i32,
        mode: mode as i32,
        dice,
        modifiers,
        total,
        critical: single && kept_single == Some(20),
        fumble: single && kept_single == Some(1),
        visibility: RollVisibility::Public as i32,
        roller_user_id: roller_user_id.to_string(),
        roller_creature_id: String::new(),
        label,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(count: u32, sides: u32, modifier: Option<i32>) -> Option<Notation> {
        Some(Notation {
            count,
            sides,
            modifier,
        })
    }

    #[test]
    fn parses_supported_forms() {
        assert_eq!(parse_notation("1d20"), n(1, 20, None));
        assert_eq!(parse_notation("2d6+3"), n(2, 6, Some(3)));
        assert_eq!(parse_notation("4D8-2"), n(4, 8, Some(-2)));
        assert_eq!(parse_notation("  3 d 10 + 5 "), n(3, 10, Some(5)));
        assert_eq!(parse_notation("100d100"), n(100, 100, None));
        assert_eq!(parse_notation("1d4+0"), n(1, 4, Some(0)));
    }

    #[test]
    fn rejects_everything_else() {
        for bad in [
            "",
            "banana",
            "d20",
            "1d",
            "0d6",
            "101d6",
            "1d7",
            "1d20+",
            "1d20+-1",
            "1d20++1",
            "1d20*2",
            "2d6kh1",
            "+1d20",
            "-1d6",
            "1d6+3x",
            "1d6 3",
            "1.5d6",
            "1d6+1.5",
            "1d+3",
            "1d20+99999999999",
        ] {
            assert_eq!(parse_notation(bad), None, "{bad:?} should be rejected");
        }
    }

    #[test]
    fn canonical_form_has_no_whitespace() {
        assert_eq!(parse_notation(" 2 D 6 - 1 ").unwrap().canonical(), "2d6-1");
        assert_eq!(parse_notation("1d20+0").unwrap().canonical(), "1d20+0");
        assert_eq!(parse_notation("3d4").unwrap().canonical(), "3d4");
    }

    #[test]
    fn rng_is_deterministic_and_never_stuck_at_zero() {
        let mut a = XorShift64Star::new(7);
        let mut b = XorShift64Star::new(7);
        assert_eq!(a.next_u64(), b.next_u64());
        let mut z = XorShift64Star::new(0);
        assert_ne!(z.next_u64(), 0);
    }

    #[test]
    fn dice_stay_in_range() {
        let mut rng = XorShift64Star::new(42);
        let notation = parse_notation("100d100").unwrap();
        let result = roll(
            &mut rng,
            &notation,
            RollMode::Normal,
            "r".into(),
            "u",
            String::new(),
        )
        .unwrap();
        assert_eq!(result.dice.len(), 100);
        assert!(result.dice.iter().all(|d| (1..=100).contains(&d.value)));
        assert_eq!(
            result.total,
            result.dice.iter().map(|d| d.value as i32).sum::<i32>()
        );
    }

    #[test]
    fn advantage_ignored_unless_single_d20() {
        let mut rng = XorShift64Star::new(1);
        let notation = parse_notation("2d20").unwrap();
        let result = roll(
            &mut rng,
            &notation,
            RollMode::Advantage,
            "r".into(),
            "u",
            String::new(),
        )
        .unwrap();
        assert_eq!(result.mode, RollMode::Normal as i32);
        assert_eq!(result.dice.len(), 2);
        assert!(result.dice.iter().all(|d| !d.dropped));
    }
}
