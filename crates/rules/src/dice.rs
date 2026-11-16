//! Dice notation, parsing and rolling (ADR 0005, ADR 0006).
//!
//! Pure: no I/O, no clock, no global state. Randomness is an injected
//! [`Rng`]; this crate ships no production generator (the server supplies a
//! cryptographic one). Ids, labels, visibility and purpose of a roll belong to
//! the server and are not modelled here.

use std::fmt;
use std::str::FromStr;

use tt_contracts::v1::{Die, Modifier, RollMode};

/// Allowed die sizes.
pub const ALLOWED_SIDES: [u32; 9] = [2, 3, 4, 6, 8, 10, 12, 20, 100];
/// Most dice (counted before any keep rule) in one expression.
pub const MAX_DICE: u32 = 100;
/// Most terms in one expression.
pub const MAX_TERMS: usize = 16;
/// Largest absolute value of a constant term.
pub const MAX_CONSTANT: i32 = 100_000;
/// Longest accepted input, in bytes. Bounds all parsing work.
pub const MAX_INPUT_LEN: usize = 1_000;

// ---------------------------------------------------------------------------
// Randomness
// ---------------------------------------------------------------------------

/// A source of random 64-bit values. Injected by the caller.
pub trait Rng {
    fn next_u64(&mut self) -> u64;
}

/// Roll one die, uniform on `1..=sides`, using rejection sampling so there is
/// no modulo bias. `sides <= 1` is degenerate and yields 1 without drawing.
pub fn roll_die(rng: &mut dyn Rng, sides: u32) -> u32 {
    if sides <= 1 {
        return 1;
    }
    let s = u64::from(sides);
    // 2^64 mod s: the size of the incomplete final block that must be rejected.
    let rem = (u64::MAX % s + 1) % s;
    let max_accepted = u64::MAX - rem;
    loop {
        let x = rng.next_u64();
        if x <= max_accepted {
            // x % s < sides <= u32::MAX, so the cast is lossless.
            return (x % s) as u32 + 1;
        }
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Why a dice expression was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseError {
    /// Nothing but whitespace.
    Empty,
    /// Unexpected character or end of input; `position` is a byte offset.
    Syntax { position: usize },
    /// Die size not in [`ALLOWED_SIDES`].
    BadSides,
    /// Zero dice in a term.
    BadCount,
    /// More than [`MAX_DICE`] dice in total.
    TooManyDice,
    /// More than [`MAX_TERMS`] terms.
    TooManyTerms,
    /// Keep count not in `1..=N`.
    BadKeep,
    /// A number (or constant) beyond the allowed range.
    NumberTooLarge,
    /// Input longer than [`MAX_INPUT_LEN`] bytes.
    TooLong,
}

impl ParseError {
    /// Stable snake_case code for clients and logs.
    pub fn code(&self) -> &'static str {
        match self {
            ParseError::Empty => "dice_empty",
            ParseError::Syntax { .. } => "dice_syntax",
            ParseError::BadSides => "dice_bad_sides",
            ParseError::BadCount => "dice_bad_count",
            ParseError::TooManyDice => "dice_too_many_dice",
            ParseError::TooManyTerms => "dice_too_many_terms",
            ParseError::BadKeep => "dice_bad_keep",
            ParseError::NumberTooLarge => "dice_number_too_large",
            ParseError::TooLong => "dice_too_long",
        }
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::Empty => write!(f, "the dice expression is empty"),
            ParseError::Syntax { position } => {
                write!(f, "unexpected input at byte {position} in dice expression")
            }
            ParseError::BadSides => {
                write!(f, "die size must be one of 2, 3, 4, 6, 8, 10, 12, 20, 100")
            }
            ParseError::BadCount => write!(f, "a dice term needs at least one die"),
            ParseError::TooManyDice => write!(f, "at most {MAX_DICE} dice are allowed"),
            ParseError::TooManyTerms => write!(f, "at most {MAX_TERMS} terms are allowed"),
            ParseError::BadKeep => write!(f, "keep count must be between 1 and the dice count"),
            ParseError::NumberTooLarge => write!(f, "a number in the expression is too large"),
            ParseError::TooLong => {
                write!(
                    f,
                    "the dice expression is longer than {MAX_INPUT_LEN} bytes"
                )
            }
        }
    }
}

impl std::error::Error for ParseError {}

// ---------------------------------------------------------------------------
// Expressions
// ---------------------------------------------------------------------------

/// Keep rule on a dice term: keep the K highest or K lowest dice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Keep {
    Highest(u32),
    Lowest(u32),
}

impl Keep {
    fn count(self) -> u32 {
        match self {
            Keep::Highest(k) | Keep::Lowest(k) => k,
        }
    }
}

/// One signed term of an expression.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Term {
    /// `count` dice of `sides`, optionally keeping some; subtracted if
    /// `negative`.
    Dice {
        negative: bool,
        count: u32,
        sides: u32,
        keep: Option<Keep>,
    },
    /// A signed constant.
    Constant(i32),
}

/// A parsed dice expression: a list of signed terms. Only obtainable through
/// parsing (or [`Expr::doubled_dice`]), so the limits always hold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expr {
    terms: Vec<Term>,
}

impl FromStr for Expr {
    type Err = ParseError;
    fn from_str(s: &str) -> Result<Self, ParseError> {
        Expr::parse(s)
    }
}

struct Parser {
    /// Non-whitespace bytes, lowercased, with their original byte offsets.
    toks: Vec<(u8, usize)>,
    i: usize,
    end: usize,
}

impl Parser {
    fn peek(&self) -> Option<u8> {
        self.toks.get(self.i).map(|t| t.0)
    }

    fn pos(&self) -> usize {
        self.toks.get(self.i).map_or(self.end, |t| t.1)
    }

    fn syntax(&self) -> ParseError {
        ParseError::Syntax {
            position: self.pos(),
        }
    }

    fn number(&mut self) -> Result<u32, ParseError> {
        if !self.peek().is_some_and(|b| b.is_ascii_digit()) {
            return Err(self.syntax());
        }
        let mut value: u64 = 0;
        let mut too_large = false;
        while let Some(b) = self.peek().filter(u8::is_ascii_digit) {
            value = value * 10 + u64::from(b - b'0');
            if value > 1_000_000_000 {
                // Stop growing (no overflow) but keep consuming digits.
                too_large = true;
                value = 1_000_000_001;
            }
            self.i += 1;
        }
        if too_large {
            return Err(ParseError::NumberTooLarge);
        }
        // value <= 1_000_000_000 fits in u32.
        Ok(value as u32)
    }

    /// One term, with the sign already consumed.
    fn term(&mut self, negative: bool) -> Result<Term, ParseError> {
        let count = if self.peek() == Some(b'd') {
            1
        } else {
            let n = self.number()?;
            if self.peek() != Some(b'd') {
                if n > MAX_CONSTANT as u32 {
                    return Err(ParseError::NumberTooLarge);
                }
                let v = n as i32;
                return Ok(Term::Constant(if negative { -v } else { v }));
            }
            n
        };
        if count == 0 {
            return Err(ParseError::BadCount);
        }
        if count > MAX_DICE {
            return Err(ParseError::TooManyDice);
        }
        self.i += 1; // the 'd'
        let sides = self.number()?;
        if !ALLOWED_SIDES.contains(&sides) {
            return Err(ParseError::BadSides);
        }
        let keep = if self.peek() == Some(b'k') {
            self.i += 1;
            let highest = match self.peek() {
                Some(b'h') => true,
                Some(b'l') => false,
                _ => return Err(self.syntax()),
            };
            self.i += 1;
            let k = self.number()?;
            if k < 1 || k > count {
                return Err(ParseError::BadKeep);
            }
            Some(if highest {
                Keep::Highest(k)
            } else {
                Keep::Lowest(k)
            })
        } else {
            None
        };
        Ok(Term::Dice {
            negative,
            count,
            sides,
            keep,
        })
    }
}

impl Expr {
    /// Parse dice notation. See the module tests for accepted forms.
    pub fn parse(text: &str) -> Result<Expr, ParseError> {
        if text.len() > MAX_INPUT_LEN {
            return Err(ParseError::TooLong);
        }
        let mut toks = Vec::new();
        for (pos, b) in text.bytes().enumerate() {
            if !b.is_ascii() {
                return Err(ParseError::Syntax { position: pos });
            }
            if !b.is_ascii_whitespace() {
                toks.push((b.to_ascii_lowercase(), pos));
            }
        }
        if toks.is_empty() {
            return Err(ParseError::Empty);
        }
        let mut p = Parser {
            toks,
            i: 0,
            end: text.len(),
        };
        let mut terms: Vec<Term> = Vec::new();
        let mut total_dice: u32 = 0;
        let mut negative = false;
        if p.peek() == Some(b'-') {
            negative = true;
            p.i += 1;
        }
        loop {
            let term = p.term(negative)?;
            if let Term::Dice { count, .. } = term {
                total_dice += count; // each count <= 100, at most 16 terms
                if total_dice > MAX_DICE {
                    return Err(ParseError::TooManyDice);
                }
            }
            terms.push(term);
            if terms.len() > MAX_TERMS {
                return Err(ParseError::TooManyTerms);
            }
            match p.peek() {
                None => break,
                Some(b'+') => negative = false,
                Some(b'-') => negative = true,
                Some(_) => return Err(p.syntax()),
            }
            p.i += 1;
        }
        Ok(Expr { terms })
    }

    /// The terms in the order written.
    pub fn terms(&self) -> &[Term] {
        &self.terms
    }

    /// Canonical text: no whitespace, lowercase, explicit dice count, terms in
    /// written order (`d20` becomes `1d20`). Parses back to an equal `Expr`.
    pub fn canonical(&self) -> String {
        let mut out = String::new();
        for (i, term) in self.terms.iter().enumerate() {
            let (negative, body) = match *term {
                Term::Constant(v) => (v < 0, v.unsigned_abs().to_string()),
                Term::Dice {
                    negative,
                    count,
                    sides,
                    keep,
                } => {
                    let mut s = format!("{count}d{sides}");
                    match keep {
                        Some(Keep::Highest(k)) => s.push_str(&format!("kh{k}")),
                        Some(Keep::Lowest(k)) => s.push_str(&format!("kl{k}")),
                        None => {}
                    }
                    (negative, s)
                }
            };
            if negative {
                out.push('-');
            } else if i > 0 {
                out.push('+');
            }
            out.push_str(&body);
        }
        out
    }

    /// The 5e critical-hit rule: every dice term's count (and keep count) is
    /// doubled; constants are unchanged. `None` if that would exceed
    /// [`MAX_DICE`].
    pub fn doubled_dice(&self) -> Option<Expr> {
        let mut total: u32 = 0;
        let mut terms = Vec::with_capacity(self.terms.len());
        for term in &self.terms {
            terms.push(match *term {
                Term::Constant(v) => Term::Constant(v),
                Term::Dice {
                    negative,
                    count,
                    sides,
                    keep,
                } => {
                    total += count * 2;
                    if total > MAX_DICE {
                        return None;
                    }
                    Term::Dice {
                        negative,
                        count: count * 2,
                        sides,
                        keep: keep.map(|k| match k {
                            Keep::Highest(n) => Keep::Highest(n * 2),
                            Keep::Lowest(n) => Keep::Lowest(n * 2),
                        }),
                    }
                }
            });
        }
        Some(Expr { terms })
    }

    /// Smallest possible total. Cannot overflow under the parse limits.
    pub fn min_total(&self) -> i32 {
        self.bound(false)
    }

    /// Largest possible total. Cannot overflow under the parse limits.
    pub fn max_total(&self) -> i32 {
        self.bound(true)
    }

    fn bound(&self, upper: bool) -> i32 {
        self.terms
            .iter()
            .map(|t| match *t {
                Term::Constant(v) => v,
                Term::Dice {
                    negative,
                    count,
                    sides,
                    keep,
                } => {
                    let kept = keep.map_or(count, Keep::count) as i32;
                    let (lo, hi) = (kept, kept * sides as i32);
                    // A subtracted term swaps which end is the bound.
                    match (negative, upper) {
                        (false, false) => lo,
                        (false, true) => hi,
                        (true, false) => -hi,
                        (true, true) => -lo,
                    }
                }
            })
            .sum()
    }

    /// Expected total, or `None` if any term has a keep rule (the exact
    /// expectation of keep-highest/lowest is not a simple formula, so it is
    /// skipped rather than approximated).
    pub fn average(&self) -> Option<f64> {
        let mut sum = 0.0;
        for t in &self.terms {
            match *t {
                Term::Constant(v) => sum += f64::from(v),
                Term::Dice {
                    negative,
                    count,
                    sides,
                    keep,
                } => {
                    if keep.is_some() {
                        return None;
                    }
                    let avg = f64::from(count) * (f64::from(sides) + 1.0) / 2.0;
                    sum += if negative { -avg } else { avg };
                }
            }
        }
        Some(sum)
    }

    /// True for a "d20 test": exactly one dice term, a positive `1d20` with no
    /// keep suffix (constants allowed). Only these honour advantage and can be
    /// critical or fumble.
    pub fn is_d20_test(&self) -> bool {
        let mut dice = self.terms.iter().filter(|t| matches!(t, Term::Dice { .. }));
        let first = dice.next();
        let only_one = dice.next().is_none();
        only_one
            && matches!(
                first,
                Some(Term::Dice {
                    negative: false,
                    count: 1,
                    sides: 20,
                    keep: None,
                })
            )
    }
}

/// One modifier per constant term, in written order, with the generic source
/// `"modifier"`. Real sources ("+3 Strength") are attached by the server.
pub fn to_modifiers(expr: &Expr) -> Vec<Modifier> {
    expr.terms
        .iter()
        .filter_map(|t| match *t {
            Term::Constant(value) => Some(Modifier {
                source: "modifier".to_string(),
                value,
            }),
            Term::Dice { .. } => None,
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Rolling
// ---------------------------------------------------------------------------

/// The outcome of rolling an [`Expr`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rolled {
    /// Every physical die rolled, in roll order. `Die` carries no sign: a die
    /// belonging to a subtracted term (`- 1d4`) looks like any other, and its
    /// kept sum is subtracted from `total`. Callers who need the sign read it
    /// from the expression's terms.
    pub dice: Vec<Die>,
    /// Sum of all constant terms.
    pub constant: i32,
    /// Kept dice (signed per term) plus constants.
    pub total: i32,
    /// The effective mode: Normal unless the expression is a d20 test rolled
    /// with advantage or disadvantage.
    pub mode: RollMode,
    /// Kept d20 shows 20. Only ever true for a d20 test (see
    /// [`Expr::is_d20_test`]); `2d20kh1+5` is not one.
    pub critical: bool,
    /// Kept d20 shows 1. Same restriction as `critical`.
    pub fumble: bool,
}

/// Mark all but the kept dice in `dice` (one term's dice) as dropped. Ties
/// keep the earlier-rolled die.
fn apply_keep(dice: &mut [Die], keep: Keep) {
    let mut order: Vec<usize> = (0..dice.len()).collect();
    match keep {
        Keep::Highest(_) => {
            order.sort_by(|&a, &b| dice[b].value.cmp(&dice[a].value).then(a.cmp(&b)))
        }
        Keep::Lowest(_) => {
            order.sort_by(|&a, &b| dice[a].value.cmp(&dice[b].value).then(a.cmp(&b)))
        }
    }
    for &i in order.iter().skip(keep.count() as usize) {
        dice[i].dropped = true;
    }
}

/// Roll `expr`. Advantage and disadvantage are honoured only for a d20 test
/// (two d20s are rolled, the lower/higher is dropped, and on a tie the second
/// die is the dropped one); otherwise, and for `Unspecified`, the effective
/// mode is Normal. Arithmetic saturates; under the parse limits the extremes
/// fit an `i32`, so saturation is never reached (tested).
pub fn roll(expr: &Expr, mode: RollMode, rng: &mut dyn Rng) -> Rolled {
    let d20 = expr.is_d20_test();
    let effective = match mode {
        RollMode::Advantage | RollMode::Disadvantage if d20 => mode,
        _ => RollMode::Normal,
    };
    let two_d20 = d20 && effective != RollMode::Normal;

    let mut dice: Vec<Die> = Vec::new();
    let mut constant: i32 = 0;
    let mut total: i32 = 0;
    for term in &expr.terms {
        match *term {
            Term::Constant(v) => {
                constant = constant.saturating_add(v);
                total = total.saturating_add(v);
            }
            Term::Dice {
                negative,
                count,
                sides,
                keep,
            } => {
                let start = dice.len();
                let n = if two_d20 { 2 } else { count };
                for _ in 0..n {
                    dice.push(Die {
                        sides,
                        value: roll_die(rng, sides),
                        dropped: false,
                    });
                }
                if two_d20 {
                    let (first, second) = (dice[start].value, dice[start + 1].value);
                    let keep_first = if effective == RollMode::Advantage {
                        first >= second
                    } else {
                        first <= second
                    };
                    dice[start + usize::from(keep_first)].dropped = true;
                } else if let Some(k) = keep {
                    apply_keep(&mut dice[start..], k);
                }
                let sum: i32 = dice[start..]
                    .iter()
                    .filter(|d| !d.dropped)
                    .fold(0_i32, |acc, d| acc.saturating_add(d.value as i32));
                total = total.saturating_add(if negative { -sum } else { sum });
            }
        }
    }
    let kept_d20 = if d20 {
        dice.iter().find(|d| !d.dropped).map(|d| d.value)
    } else {
        None
    };
    Rolled {
        dice,
        constant,
        total,
        mode: effective,
        critical: kept_d20 == Some(20),
        fumble: kept_d20 == Some(1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// xorshift64*: deterministic test generator.
    struct XorShift64Star(u64);

    impl XorShift64Star {
        fn new(seed: u64) -> Self {
            Self(if seed == 0 {
                0x9E37_79B9_7F4A_7C15
            } else {
                seed
            })
        }
    }

    impl Rng for XorShift64Star {
        fn next_u64(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x >> 12;
            x ^= x << 25;
            x ^= x >> 27;
            self.0 = x;
            x.wrapping_mul(0x2545_F491_4F6C_DD1D)
        }
    }

    /// Plays back a fixed script; panics if it runs dry.
    struct Scripted {
        values: Vec<u64>,
        calls: usize,
    }

    impl Scripted {
        fn new(values: &[u64]) -> Self {
            Self {
                values: values.to_vec(),
                calls: 0,
            }
        }

        /// Script that makes `roll_die` produce these faces (face - 1).
        fn faces(faces: &[u64]) -> Self {
            Self::new(&faces.iter().map(|f| f - 1).collect::<Vec<_>>())
        }
    }

    impl Rng for Scripted {
        fn next_u64(&mut self) -> u64 {
            let v = self.values[self.calls];
            self.calls += 1;
            v
        }
    }

    fn p(s: &str) -> Expr {
        Expr::parse(s).unwrap_or_else(|e| panic!("{s:?} should parse: {e}"))
    }

    fn values(r: &Rolled) -> Vec<(u32, bool)> {
        r.dice.iter().map(|d| (d.value, d.dropped)).collect()
    }

    fn dice_count(e: &Expr) -> u32 {
        e.terms()
            .iter()
            .map(|t| match t {
                Term::Dice { count, .. } => *count,
                Term::Constant(_) => 0,
            })
            .sum()
    }

    #[test]
    fn examples_parse_and_round_trip() {
        let cases = [
            ("1d20", "1d20"),
            ("d20+5", "1d20+5"),
            ("2d6+3", "2d6+3"),
            ("2d6 + 1d4 + 3", "2d6+1d4+3"),
            ("1d8-1", "1d8-1"),
            ("2d20kh1+5", "2d20kh1+5"),
            ("4d6kl3", "4d6kl3"),
            ("7", "7"),
            ("1d10 - 1d4", "1d10-1d4"),
            ("-1d4+2", "-1d4+2"),
            ("-3", "-3"),
            (" 2 D 6 K H 1 ", "2d6kh1"),
            ("D100", "1d100"),
            ("1d3+1d2", "1d3+1d2"),
        ];
        for (input, canon) in cases {
            let e = p(input);
            assert_eq!(e.canonical(), canon, "{input:?}");
            assert_eq!(p(canon), e, "round trip of {input:?}");
            assert_eq!(canon.parse::<Expr>(), Ok(e));
        }
    }

    #[test]
    fn rejected_inputs() {
        let seventeen = vec!["1"; 17].join("+");
        let long = "1".repeat(10_000);
        let table: Vec<(&str, ParseError)> = vec![
            ("", ParseError::Empty),
            ("   ", ParseError::Empty),
            ("d", ParseError::Syntax { position: 1 }),
            ("2d", ParseError::Syntax { position: 2 }),
            ("2d7", ParseError::BadSides),
            ("0d6", ParseError::BadCount),
            ("101d6", ParseError::TooManyDice),
            ("50d6+51d6", ParseError::TooManyDice),
            ("2d6kh3", ParseError::BadKeep),
            ("2d6kh0", ParseError::BadKeep),
            ("2d6kx1", ParseError::Syntax { position: 4 }),
            ("1d20++5", ParseError::Syntax { position: 5 }),
            ("1d20+", ParseError::Syntax { position: 5 }),
            ("abc", ParseError::Syntax { position: 0 }),
            ("+1d20", ParseError::Syntax { position: 0 }),
            ("1d20 d4", ParseError::Syntax { position: 5 }),
            // Whitespace is ignored everywhere, so this reads as 1d205.
            ("1d20 5", ParseError::BadSides),
            ("100001", ParseError::NumberTooLarge),
            ("-100001", ParseError::NumberTooLarge),
            (
                "1d20+999999999999999999999999999999",
                ParseError::NumberTooLarge,
            ),
            (
                "99999999999999999999999999999d6",
                ParseError::NumberTooLarge,
            ),
            (&seventeen, ParseError::TooManyTerms),
            (&long, ParseError::TooLong),
            ("1d20+\u{e9}", ParseError::Syntax { position: 5 }),
            ("\u{1F3B2}", ParseError::Syntax { position: 0 }),
        ];
        for (input, expected) in table {
            assert_eq!(Expr::parse(input), Err(expected), "{input:.40?}");
        }
    }

    #[test]
    fn limits_accept_the_edge() {
        assert!(Expr::parse("100d100").is_ok());
        assert!(Expr::parse("100000").is_ok());
        assert!(Expr::parse("-100000").is_ok());
        assert!(Expr::parse(&vec!["1"; 16].join("+")).is_ok());
        assert!(Expr::parse("2d6kh2").is_ok());
        for s in ALLOWED_SIDES {
            assert!(Expr::parse(&format!("1d{s}")).is_ok());
        }
    }

    #[test]
    fn error_codes_and_display_are_stable() {
        assert_eq!(ParseError::BadSides.code(), "dice_bad_sides");
        assert_eq!(ParseError::Syntax { position: 3 }.code(), "dice_syntax");
        assert_eq!(ParseError::Empty.code(), "dice_empty");
        assert_eq!(ParseError::TooManyDice.code(), "dice_too_many_dice");
        assert_eq!(ParseError::TooManyTerms.code(), "dice_too_many_terms");
        assert_eq!(ParseError::BadKeep.code(), "dice_bad_keep");
        assert_eq!(ParseError::BadCount.code(), "dice_bad_count");
        assert_eq!(ParseError::NumberTooLarge.code(), "dice_number_too_large");
        assert_eq!(ParseError::TooLong.code(), "dice_too_long");
        assert!(ParseError::Syntax { position: 3 }.to_string().contains('3'));
        let _: &dyn std::error::Error = &ParseError::Empty;
    }

    #[test]
    fn parser_never_panics_on_noise() {
        let alphabet = b"0123456789dDkKhHlL+- x\t9d";
        let mut rng = XorShift64Star::new(99);
        for _ in 0..20_000 {
            let len = (rng.next_u64() % 24) as usize;
            let s: String = (0..len)
                .map(|_| alphabet[(rng.next_u64() % alphabet.len() as u64) as usize] as char)
                .collect();
            if let Ok(e) = Expr::parse(&s) {
                assert_eq!(Expr::parse(&e.canonical()), Ok(e));
            }
        }
    }

    #[test]
    fn roll_die_stays_in_range_and_covers_faces() {
        let mut rng = XorShift64Star::new(2024);
        for sides in ALLOWED_SIDES {
            for _ in 0..2_000 {
                let v = roll_die(&mut rng, sides);
                assert!((1..=sides).contains(&v), "d{sides} gave {v}");
            }
        }
        for sides in [6_u32, 20] {
            let mut seen = vec![false; sides as usize];
            for _ in 0..5_000 {
                seen[roll_die(&mut rng, sides) as usize - 1] = true;
            }
            assert!(seen.iter().all(|&s| s), "d{sides} missed a face");
        }
        assert_eq!(roll_die(&mut rng, 0), 1);
        assert_eq!(roll_die(&mut rng, 1), 1);
    }

    #[test]
    fn roll_die_rejects_the_biased_tail() {
        // For d6, 2^64 mod 6 = 4, so the top four u64 values are rejected.
        let mut rng = Scripted::new(&[u64::MAX, u64::MAX - 3, 7]);
        let v = roll_die(&mut rng, 6);
        assert_eq!(rng.calls, 3, "two draws must be rejected");
        assert_eq!(v, 2);
        // The largest accepted value is taken immediately.
        let mut rng = Scripted::new(&[u64::MAX - 4]);
        let v = roll_die(&mut rng, 6);
        assert_eq!(rng.calls, 1);
        assert!((1..=6).contains(&v));
        // Powers of two never reject.
        let mut rng = Scripted::new(&[u64::MAX]);
        assert_eq!(roll_die(&mut rng, 2), 2);
        assert_eq!(rng.calls, 1);
    }

    #[test]
    fn advantage_and_disadvantage_keep_the_right_die() {
        let e = p("1d20+5");
        let r = roll(&e, RollMode::Advantage, &mut Scripted::faces(&[7, 15]));
        assert_eq!(values(&r), [(7, true), (15, false)]);
        assert_eq!((r.total, r.constant, r.mode), (20, 5, RollMode::Advantage));

        let r = roll(&e, RollMode::Advantage, &mut Scripted::faces(&[15, 7]));
        assert_eq!(values(&r), [(15, false), (7, true)]);

        let r = roll(&e, RollMode::Disadvantage, &mut Scripted::faces(&[7, 15]));
        assert_eq!(values(&r), [(7, false), (15, true)]);
        assert_eq!((r.total, r.mode), (12, RollMode::Disadvantage));

        let r = roll(&e, RollMode::Disadvantage, &mut Scripted::faces(&[15, 7]));
        assert_eq!(values(&r), [(15, true), (7, false)]);

        // Ties: the second die is the dropped one.
        for mode in [RollMode::Advantage, RollMode::Disadvantage] {
            let r = roll(&e, mode, &mut Scripted::faces(&[9, 9]));
            assert_eq!(values(&r), [(9, false), (9, true)]);
            assert_eq!(r.total, 14);
        }
    }

    #[test]
    fn mode_falls_back_to_normal() {
        for src in [
            "2d6",
            "2d20",
            "1d20+1d4",
            "1d20kh1",
            "-1d20",
            "1d20-1d20",
            "5",
        ] {
            let e = p(src);
            for mode in [RollMode::Advantage, RollMode::Disadvantage] {
                let r = roll(&e, mode, &mut XorShift64Star::new(5));
                assert_eq!(r.mode, RollMode::Normal, "{src}");
                assert_eq!(r.dice.len() as u32, dice_count(&e), "{src}");
                assert!(!r.critical && !r.fumble, "{src}");
            }
        }
        let e = p("1d20");
        for mode in [RollMode::Normal, RollMode::Unspecified] {
            let r = roll(&e, mode, &mut XorShift64Star::new(5));
            assert_eq!(r.mode, RollMode::Normal);
            assert_eq!(r.dice.len(), 1);
        }
        // A constant before the d20 is still a d20 test.
        assert!(p("3+1d20").is_d20_test());
    }

    #[test]
    fn critical_and_fumble() {
        let e = p("1d20+5");
        let r = roll(&e, RollMode::Normal, &mut Scripted::faces(&[20]));
        assert!(r.critical && !r.fumble);
        let r = roll(&e, RollMode::Normal, &mut Scripted::faces(&[1]));
        assert!(r.fumble && !r.critical);
        let r = roll(&e, RollMode::Normal, &mut Scripted::faces(&[10]));
        assert!(!r.fumble && !r.critical);
        // Judged on the kept die only.
        let r = roll(&e, RollMode::Advantage, &mut Scripted::faces(&[20, 3]));
        assert!(r.critical);
        let r = roll(&e, RollMode::Disadvantage, &mut Scripted::faces(&[20, 3]));
        assert!(!r.critical && !r.fumble);
        let r = roll(&e, RollMode::Advantage, &mut Scripted::faces(&[1, 1]));
        assert!(r.fumble);
        // Not a d20 test by definition: no flags even on a natural 20.
        let r = roll(
            &p("2d20kh1+5"),
            RollMode::Normal,
            &mut Scripted::faces(&[20, 2]),
        );
        assert!(!r.critical && !r.fumble);
        let r = roll(
            &p("1d20+1d4"),
            RollMode::Normal,
            &mut Scripted::faces(&[20, 2]),
        );
        assert!(!r.critical);
    }

    #[test]
    fn keep_highest_and_lowest() {
        let n = RollMode::Normal;
        let r = roll(&p("4d6kh3"), n, &mut Scripted::faces(&[2, 6, 3, 5]));
        assert_eq!(values(&r), [(2, true), (6, false), (3, false), (5, false)]);
        assert_eq!(r.total, 14);

        let r = roll(&p("4d6kl3"), n, &mut Scripted::faces(&[2, 6, 3, 5]));
        assert_eq!(values(&r), [(2, false), (6, true), (3, false), (5, false)]);
        assert_eq!(r.total, 10);

        let r = roll(&p("2d20kh1+5"), n, &mut Scripted::faces(&[4, 17]));
        assert_eq!(values(&r), [(4, true), (17, false)]);
        assert_eq!((r.total, r.constant), (22, 5));

        // Ties keep the earlier die.
        let r = roll(&p("3d6kh2"), n, &mut Scripted::faces(&[4, 4, 4]));
        assert_eq!(values(&r), [(4, false), (4, false), (4, true)]);
        let r = roll(&p("3d6kl1"), n, &mut Scripted::faces(&[4, 4, 4]));
        assert_eq!(values(&r), [(4, false), (4, true), (4, true)]);

        // Keep applies per term.
        let r = roll(&p("2d6kh1+2d8kl1"), n, &mut Scripted::faces(&[3, 5, 7, 2]));
        assert_eq!(values(&r), [(3, true), (5, false), (7, true), (2, false)]);
        assert_eq!(r.total, 7);

        // Keeping everything drops nothing.
        let r = roll(&p("2d6kh2"), n, &mut Scripted::faces(&[1, 2]));
        assert!(r.dice.iter().all(|d| !d.dropped));
    }

    #[test]
    fn subtraction_terms() {
        let n = RollMode::Normal;
        let r = roll(&p("1d10-1d4"), n, &mut Scripted::faces(&[8, 3]));
        assert_eq!(r.total, 5);
        assert_eq!(values(&r), [(8, false), (3, false)]);

        let r = roll(&p("-1d4+10"), n, &mut Scripted::faces(&[4]));
        assert_eq!((r.total, r.constant), (6, 10));

        let r = roll(&p("5-2d6kh1-1"), n, &mut Scripted::faces(&[2, 6]));
        assert_eq!(r.total, 5 - 6 - 1);
        assert_eq!(r.constant, 4);

        let r = roll(&p("-7"), n, &mut Scripted::new(&[]));
        assert_eq!((r.total, r.constant), (-7, -7));
        assert!(r.dice.is_empty());
    }

    #[test]
    fn doubled_dice_follows_the_crit_rule() {
        assert_eq!(p("2d6+3").doubled_dice().unwrap().canonical(), "4d6+3");
        assert_eq!(
            p("1d8+1d6-2").doubled_dice().unwrap().canonical(),
            "2d8+2d6-2"
        );
        assert_eq!(p("-1d4").doubled_dice().unwrap().canonical(), "-2d4");
        assert_eq!(
            p("2d20kh1+5").doubled_dice().unwrap().canonical(),
            "4d20kh2+5"
        );
        assert_eq!(p("7").doubled_dice().unwrap().canonical(), "7");
        assert!(p("50d6").doubled_dice().is_some());
        assert!(p("51d6").doubled_dice().is_none());
        assert!(p("30d6+30d6").doubled_dice().is_none());
    }

    #[test]
    fn min_max_average() {
        let range = |s: &str| (p(s).min_total(), p(s).max_total());
        assert_eq!(range("2d6+3"), (5, 15));
        assert_eq!(range("1d10-1d4"), (-3, 9));
        assert_eq!(range("4d6kh3"), (3, 18));
        assert_eq!(range("-2d6-1"), (-13, -3));
        assert_eq!(p("2d6+3").average(), Some(10.0));
        assert_eq!(p("1d20-1d4+1").average(), Some(10.5 - 2.5 + 1.0));
        assert_eq!(p("7").average(), Some(7.0));
        assert_eq!(p("2d20kh1").average(), None);
    }

    #[test]
    fn totals_stay_inside_bounds() {
        let sources = [
            "1d20",
            "d20+5",
            "2d6+3",
            "2d6+1d4+3",
            "1d10-1d4",
            "2d20kh1+5",
            "4d6kl3",
            "-2d8-3",
            "100d100",
            "7",
        ];
        let mut rng = XorShift64Star::new(31337);
        for src in sources {
            let e = p(src);
            for i in 0..10_000 {
                let mode = match i % 3 {
                    0 => RollMode::Normal,
                    1 => RollMode::Advantage,
                    _ => RollMode::Disadvantage,
                };
                let r = roll(&e, mode, &mut rng);
                assert!(
                    (e.min_total()..=e.max_total()).contains(&r.total),
                    "{src}: {}",
                    r.total
                );
                assert!(r.dice.iter().all(|d| (1..=d.sides).contains(&d.value)));
            }
        }
    }

    #[test]
    fn extremes_cannot_overflow() {
        // Worst case the limits allow: every term at its extreme.
        let worst = format!("100d100+{}", vec!["100000"; 15].join("+"));
        let e = p(&worst);
        assert_eq!(e.max_total(), 10_000 + 1_500_000);
        let neg = format!("-100d100-{}", vec!["100000"; 15].join("-"));
        assert_eq!(p(&neg).min_total(), -(10_000 + 1_500_000));
        // Rolling every die at its top face lands exactly on the maximum,
        // so no saturation occurred.
        let mut max_rng = Scripted::new(&[99; 100]);
        let r = roll(&e, RollMode::Normal, &mut max_rng);
        assert_eq!(r.total, e.max_total());
        assert_eq!(r.constant, 1_500_000);
    }

    #[test]
    fn modifiers_follow_constant_terms() {
        let m = to_modifiers(&p("2d6+3-1d4-2"));
        assert_eq!(m.iter().map(|m| m.value).collect::<Vec<_>>(), [3, -2]);
        assert!(to_modifiers(&p("1d20")).is_empty());
    }
}
