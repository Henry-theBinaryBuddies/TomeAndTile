//! Distance in feet between two tokens on a square grid (ADR 0005).
//!
//! Pure geometry: no I/O, no clock. The function answers `None` whenever the
//! distance cannot be measured, and callers then skip range checks:
//!
//! - the grid kind is not `GRID_KIND_SQUARE` (hex, none, unspecified),
//! - `cell_size` or `feet_per_cell` is 0, negative or not finite, or the
//!   grid `offset` is not finite,
//! - either token has no `position`, or a position that is not finite.
//!
//! A token's `position` is its centre in scene units. It becomes cells with
//! `cell_size` and `offset`, using the same convention as the renderer's
//! `snap`: odd sizes centre in a cell (`floor`), even sizes sit on a grid
//! corner (`round`, halves away from zero). Tiny, Small, Medium (and an
//! unset size) cover 1 cell, Large 2x2, Huge 3x3, Gargantuan 4x4.
//!
//! Distance is measured between the nearest occupied cells: the gap, in
//! cells, along each axis is the difference between the closest cell
//! indexes (0 when the footprints overlap on that axis, 1 when they touch).
//! With `dx` and `dy` that gap:
//!
//! - `DIAGONAL_RULE_FIVE` (and `UNSPECIFIED`): `max(dx, dy)` cells.
//! - `DIAGONAL_RULE_FIVE_TEN`: `max(dx, dy) + min(dx, dy) / 2` cells (integer
//!   division), so every second diagonal step costs 10 ft.
//! - `DIAGONAL_RULE_EUCLIDEAN`: `sqrt(dx^2 + dy^2)` cells, not rounded
//!   (a one-cell diagonal is about 7.07 ft); callers compare it as is.
//!
//! The cell count is multiplied by `feet_per_cell`.

use tt_contracts::v1::{CreatureSize, DiagonalRule, Grid, GridKind, Token};

/// Inclusive cell-index box a token occupies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Cells {
    x_lo: i64,
    x_hi: i64,
    y_lo: i64,
    y_hi: i64,
}

/// Cells across for a token size; unset counts as one cell.
fn cells_across(size: i32) -> i64 {
    match CreatureSize::try_from(size) {
        Ok(CreatureSize::Large) => 2,
        Ok(CreatureSize::Huge) => 3,
        Ok(CreatureSize::Gargantuan) => 4,
        _ => 1,
    }
}

/// Inclusive range of cell indexes on one axis.
fn axis_range(pos: f64, offset: f64, cell: f64, n: i64) -> (i64, i64) {
    let u = (pos - offset) / cell;
    if n % 2 == 0 {
        // Centre on a corner: the corner index is k, cells k-n/2 .. k+n/2-1.
        // `as` saturates, so absurd coordinates cannot panic.
        let k = u.round() as i64;
        (k.saturating_sub(n / 2), k.saturating_add(n / 2 - 1))
    } else {
        let c = u.floor() as i64;
        let r = (n - 1) / 2;
        (c.saturating_sub(r), c.saturating_add(r))
    }
}

fn occupied(grid: &Grid, token: &Token) -> Option<Cells> {
    let pos = token.position.as_ref()?;
    let (ox, oy) = grid
        .offset
        .as_ref()
        .map_or((0.0, 0.0), |o| (f64::from(o.x), f64::from(o.y)));
    let (px, py) = (f64::from(pos.x), f64::from(pos.y));
    let cell = f64::from(grid.cell_size);
    if !(px.is_finite() && py.is_finite() && ox.is_finite() && oy.is_finite()) {
        return None;
    }
    if !(cell.is_finite() && cell > 0.0) {
        return None;
    }
    let n = cells_across(token.size);
    let (x_lo, x_hi) = axis_range(px, ox, cell, n);
    let (y_lo, y_hi) = axis_range(py, oy, cell, n);
    Some(Cells {
        x_lo,
        x_hi,
        y_lo,
        y_hi,
    })
}

/// Gap in cells between two inclusive ranges: 0 when they overlap, 1 when
/// adjacent.
fn gap(a: (i64, i64), b: (i64, i64)) -> i64 {
    a.0.saturating_sub(b.1).max(b.0.saturating_sub(a.1)).max(0)
}

/// Distance in feet between the nearest occupied cells of two tokens, or
/// `None` when range cannot be checked (see the module docs).
pub fn distance_feet(grid: &Grid, a: &Token, b: &Token) -> Option<f32> {
    if GridKind::try_from(grid.kind) != Ok(GridKind::Square) {
        return None;
    }
    let feet = f64::from(grid.feet_per_cell);
    if !(feet.is_finite() && feet > 0.0) {
        return None;
    }
    let ca = occupied(grid, a)?;
    let cb = occupied(grid, b)?;
    let dx = gap((ca.x_lo, ca.x_hi), (cb.x_lo, cb.x_hi));
    let dy = gap((ca.y_lo, ca.y_hi), (cb.y_lo, cb.y_hi));
    let (long, short) = (dx.max(dy), dx.min(dy));
    let cells = match DiagonalRule::try_from(grid.diagonal_rule) {
        Ok(DiagonalRule::FiveTen) => long.saturating_add(short / 2) as f64,
        Ok(DiagonalRule::Euclidean) => (dx as f64).hypot(dy as f64),
        _ => long as f64,
    };
    Some((cells * feet) as f32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tt_contracts::v1::Vec2;

    fn grid(rule: DiagonalRule) -> Grid {
        Grid {
            kind: GridKind::Square as i32,
            cell_size: 70.0,
            offset: None,
            feet_per_cell: 5.0,
            diagonal_rule: rule as i32,
        }
    }

    /// A Medium token centred in cell (cx, cy).
    fn medium(cx: f32, cy: f32) -> Token {
        Token {
            position: Some(Vec2 {
                x: (cx + 0.5) * 70.0,
                y: (cy + 0.5) * 70.0,
            }),
            size: CreatureSize::Medium as i32,
            ..Default::default()
        }
    }

    /// A token of `size` whose centre is on the grid corner (kx, ky).
    fn on_corner(kx: f32, ky: f32, size: CreatureSize) -> Token {
        Token {
            position: Some(Vec2 {
                x: kx * 70.0,
                y: ky * 70.0,
            }),
            size: size as i32,
            ..Default::default()
        }
    }

    fn d(rule: DiagonalRule, a: &Token, b: &Token) -> Option<f32> {
        distance_feet(&grid(rule), a, b)
    }

    #[test]
    fn straight_lines_and_same_cell() {
        let a = medium(0.0, 0.0);
        assert_eq!(d(DiagonalRule::Five, &a, &a), Some(0.0));
        assert_eq!(d(DiagonalRule::Five, &a, &medium(1.0, 0.0)), Some(5.0));
        assert_eq!(d(DiagonalRule::Five, &a, &medium(0.0, 6.0)), Some(30.0));
        assert_eq!(d(DiagonalRule::FiveTen, &a, &medium(-4.0, 0.0)), Some(20.0));
        assert_eq!(
            d(DiagonalRule::Euclidean, &a, &medium(0.0, 3.0)),
            Some(15.0)
        );
    }

    #[test]
    fn five_rule_every_diagonal_is_five() {
        let a = medium(0.0, 0.0);
        assert_eq!(d(DiagonalRule::Five, &a, &medium(1.0, 1.0)), Some(5.0));
        assert_eq!(d(DiagonalRule::Five, &a, &medium(4.0, 4.0)), Some(20.0));
        assert_eq!(d(DiagonalRule::Five, &a, &medium(3.0, -5.0)), Some(25.0));
    }

    #[test]
    fn unspecified_rule_counts_as_five() {
        let a = medium(0.0, 0.0);
        let b = medium(4.0, 4.0);
        assert_eq!(d(DiagonalRule::Unspecified, &a, &b), Some(20.0));
    }

    #[test]
    fn five_ten_rule_every_second_diagonal_costs_ten() {
        let a = medium(0.0, 0.0);
        assert_eq!(d(DiagonalRule::FiveTen, &a, &medium(1.0, 1.0)), Some(5.0));
        assert_eq!(d(DiagonalRule::FiveTen, &a, &medium(2.0, 2.0)), Some(15.0));
        assert_eq!(d(DiagonalRule::FiveTen, &a, &medium(3.0, 3.0)), Some(20.0));
        assert_eq!(d(DiagonalRule::FiveTen, &a, &medium(4.0, 4.0)), Some(30.0));
        // 5 across, 2 down: 5 + 2/2 = 6 cells.
        assert_eq!(d(DiagonalRule::FiveTen, &a, &medium(5.0, 2.0)), Some(30.0));
        // Symmetric.
        assert_eq!(d(DiagonalRule::FiveTen, &medium(5.0, 2.0), &a), Some(30.0));
    }

    #[test]
    fn euclidean_is_straight_line_unrounded() {
        let a = medium(0.0, 0.0);
        let one = d(DiagonalRule::Euclidean, &a, &medium(1.0, 1.0)).unwrap();
        assert!((one - 7.071_068).abs() < 1e-4, "{one}");
        // 3-4-5 triangle: 5 cells = 25 ft exactly.
        assert_eq!(
            d(DiagonalRule::Euclidean, &a, &medium(3.0, 4.0)),
            Some(25.0)
        );
    }

    #[test]
    fn feet_per_cell_scales_the_result() {
        let mut g = grid(DiagonalRule::Five);
        g.feet_per_cell = 10.0;
        assert_eq!(
            distance_feet(&g, &medium(0.0, 0.0), &medium(3.0, 1.0)),
            Some(30.0)
        );
    }

    #[test]
    fn grid_offset_shifts_the_cells() {
        let mut g = grid(DiagonalRule::Five);
        g.offset = Some(Vec2 { x: 35.0, y: 0.0 });
        let at = |x: f32| Token {
            position: Some(Vec2 { x, y: 35.0 }),
            size: CreatureSize::Medium as i32,
            ..Default::default()
        };
        // With the offset, cell 0 spans x 35..105 and cell 3 spans 245..315.
        assert_eq!(distance_feet(&g, &at(70.0), &at(280.0)), Some(15.0));
        // Off-centre inside the same cell is the same cell.
        assert_eq!(distance_feet(&g, &at(70.0), &at(100.0)), Some(0.0));
    }

    #[test]
    fn small_and_tiny_occupy_one_cell() {
        let a = medium(0.0, 0.0);
        for size in [
            CreatureSize::Tiny,
            CreatureSize::Small,
            CreatureSize::Unspecified,
        ] {
            let mut b = medium(3.0, 0.0);
            b.size = size as i32;
            assert_eq!(d(DiagonalRule::Five, &a, &b), Some(15.0));
        }
    }

    #[test]
    fn large_token_measures_from_its_nearest_cell() {
        // Large on corner (1, 1) occupies cells 0..=1 on both axes.
        let large = on_corner(1.0, 1.0, CreatureSize::Large);
        assert_eq!(d(DiagonalRule::Five, &large, &medium(4.0, 0.0)), Some(15.0));
        assert_eq!(d(DiagonalRule::Five, &large, &medium(1.0, 1.0)), Some(0.0));
        assert_eq!(d(DiagonalRule::Five, &large, &medium(2.0, 2.0)), Some(5.0));
        assert_eq!(
            d(DiagonalRule::Five, &medium(-2.0, 0.0), &large),
            Some(10.0)
        );
    }

    #[test]
    fn an_even_size_token_off_a_corner_snaps_to_the_nearest_corner() {
        // Centre at (1.4, 0.6) cells: rounds to corner (1, 1), so it occupies
        // cells 0..=1 on both axes. floor would give corner (1, 0) and ceil
        // corner (2, 1); each assertion below fails under one of them.
        let large = on_corner(1.4, 0.6, CreatureSize::Large);
        assert_eq!(d(DiagonalRule::Five, &large, &medium(4.0, 0.0)), Some(15.0));
        assert_eq!(d(DiagonalRule::Five, &large, &medium(0.0, 3.0)), Some(10.0));
    }

    #[test]
    fn huge_and_gargantuan_footprints() {
        // Huge centres in a cell: centre cell (5, 5) occupies 4..=6.
        let mut huge = medium(5.0, 5.0);
        huge.size = CreatureSize::Huge as i32;
        assert_eq!(d(DiagonalRule::Five, &huge, &medium(9.0, 5.0)), Some(15.0));
        assert_eq!(d(DiagonalRule::Five, &huge, &medium(7.0, 5.0)), Some(5.0));
        // Gargantuan on corner (4, 4) occupies 2..=5.
        let garg = on_corner(4.0, 4.0, CreatureSize::Gargantuan);
        assert_eq!(d(DiagonalRule::Five, &garg, &medium(8.0, 2.0)), Some(15.0));
        assert_eq!(d(DiagonalRule::Five, &garg, &medium(5.0, 5.0)), Some(0.0));
    }

    #[test]
    fn two_large_tokens_use_nearest_cells() {
        let a = on_corner(1.0, 1.0, CreatureSize::Large); // cells 0..=1
        let b = on_corner(7.0, 1.0, CreatureSize::Large); // cells 6..=7
        assert_eq!(d(DiagonalRule::Five, &a, &b), Some(25.0));
    }

    #[test]
    fn range_is_not_checked_without_a_measurable_square_grid() {
        let a = medium(0.0, 0.0);
        let b = medium(3.0, 0.0);
        for kind in [
            GridKind::Unspecified,
            GridKind::HexRows,
            GridKind::HexColumns,
            GridKind::None,
        ] {
            let mut g = grid(DiagonalRule::Five);
            g.kind = kind as i32;
            assert_eq!(distance_feet(&g, &a, &b), None);
        }
        let mut g = grid(DiagonalRule::Five);
        g.cell_size = 0.0;
        assert_eq!(distance_feet(&g, &a, &b), None);
        g.cell_size = f32::NAN;
        assert_eq!(distance_feet(&g, &a, &b), None);
        let mut g = grid(DiagonalRule::Five);
        g.feet_per_cell = 0.0;
        assert_eq!(distance_feet(&g, &a, &b), None);
        g.feet_per_cell = -5.0;
        assert_eq!(distance_feet(&g, &a, &b), None);
    }

    #[test]
    fn missing_or_non_finite_positions_are_not_measured() {
        let a = medium(0.0, 0.0);
        let mut b = medium(3.0, 0.0);
        b.position = None;
        assert_eq!(d(DiagonalRule::Five, &a, &b), None);
        b.position = Some(Vec2 {
            x: f32::INFINITY,
            y: 0.0,
        });
        assert_eq!(d(DiagonalRule::Five, &a, &b), None);
    }

    #[test]
    fn huge_coordinates_do_not_overflow() {
        let a = medium(0.0, 0.0);
        let mut b = medium(0.0, 0.0);
        b.position = Some(Vec2 {
            x: f32::MAX,
            y: -f32::MAX,
        });
        for rule in [
            DiagonalRule::Five,
            DiagonalRule::FiveTen,
            DiagonalRule::Euclidean,
        ] {
            assert!(d(rule, &a, &b).is_some());
        }
    }
}
