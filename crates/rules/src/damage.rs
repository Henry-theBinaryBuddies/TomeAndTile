//! Applying damage and healing to hit points (ADR 0005, ADR 0006).
//!
//! Pure functions: a token and its creature in, an updated token out. Hit
//! points live on the [`Token`] (`hp_current`, `hp_max`, `hp_temp`); the
//! [`Creature`] is only read, for its resistances, immunities and
//! vulnerabilities, and its own `hp_*` are never changed. Several tokens can
//! share one creature.
//!
//! # Damage, per part
//!
//! [`apply_damage`] takes a list of [`DamageRoll`]s (type, the roll's total,
//! whether a successful save halves it). Each part is worked out in this
//! order:
//!
//! 1. a negative total counts as 0;
//! 2. `halved`: divide by 2, rounding down;
//! 3. immune to the type: 0;
//! 4. resistant: divide by 2, rounding down;
//! 5. vulnerable: double.
//!
//! A creature that is resistant and vulnerable to a type gets both (step 4
//! then 5). A type listed twice on the creature counts once. Each part
//! yields one [`DamageApplied`] (`rolled` is the roll's total as given,
//! `applied` the result). The applied parts are added up, taken off `hp_temp`
//! first, and the rest off `hp_current`, which floors at 0.
//!
//! `dropped_to_zero` is true only when `hp_current` went from above 0 to 0.
//! Damage to a token already at 0 changes nothing and is not `dropped_to_zero`.
//!
//! # Healing
//!
//! [`apply_healing`] adds to `hp_current`, capped at `hp_max`; `hp_temp` is
//! untouched and a token at 0 can be healed (it stops being Down). The
//! amount reported is what was actually restored.
//!
//! # Health state
//!
//! [`health_state`] gives the [`HealthState`] for a current and maximum; the
//! returned token has it recomputed after every change, and nothing else on
//! the token changes.
//!
//! # No hit points
//!
//! A token with no `hp_max` (or an `hp_max` of 0 or less, which the contract
//! treats the same way) has no hit points to change. Nothing is applied and
//! the token comes back as it was, but the damage parts are still worked out
//! for the result card and the result says `no_hit_points`. A token with an
//! `hp_max` but no `hp_current` counts as full. A missing `hp_temp` is 0.
//!
//! # Not automated
//!
//! No death-save, instant-death or massive-damage rule runs at 0 HP: reaching
//! 0 sets Down and nothing else, and death saves are rolled by hand
//! (ADR 0005). Damage reduction such as Heavy Armor Master, and granting
//! temporary HP, are not modelled.

use tt_contracts::v1::{Creature, DamageApplied, DamageType, HealthState, Token};

/// One damage part before the target's modifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DamageRoll {
    pub damage_type: DamageType,
    /// The roll's total; a negative total counts as 0.
    pub rolled: i32,
    /// A successful save with `half_on_success`.
    pub halved: bool,
}

/// The result of [`apply_damage`].
#[derive(Debug, Clone, PartialEq)]
pub struct DamageResult {
    /// The token after the damage; equal to the input when it has no hit
    /// points or nothing got through.
    pub token: Token,
    /// One per input part, in order.
    pub damage: Vec<DamageApplied>,
    /// `hp_current` went from above 0 to 0.
    pub dropped_to_zero: bool,
    /// The token has no hit points, so nothing was applied.
    pub no_hit_points: bool,
}

/// The result of [`apply_healing`].
#[derive(Debug, Clone, PartialEq)]
pub struct HealingResult {
    pub token: Token,
    /// Hit points actually restored (0 at full health or with no hit points).
    pub restored: i32,
    /// The token has no hit points, so nothing was applied.
    pub no_hit_points: bool,
}

/// The health state for `hp_current` of `hp_max`, in integer arithmetic:
/// Down at 0, Near death at or below a quarter, Bloodied at or below half,
/// otherwise Healthy. `Unspecified` when `hp_max` is 0 or less.
pub fn health_state(hp_current: i32, hp_max: i32) -> HealthState {
    if hp_max <= 0 {
        return HealthState::Unspecified;
    }
    let (cur, max) = (i64::from(hp_current), i64::from(hp_max));
    if cur <= 0 {
        HealthState::Down
    } else if cur * 4 <= max {
        HealthState::NearDeath
    } else if cur * 2 <= max {
        HealthState::Bloodied
    } else {
        HealthState::Healthy
    }
}

/// The hit points of a token, if it has any: `(current, max, temp)`.
fn hit_points(token: &Token) -> Option<(i32, i32, i32)> {
    let max = token.hp_max.filter(|m| *m > 0)?;
    Some((
        token.hp_current.unwrap_or(max),
        max,
        token.hp_temp.unwrap_or(0).max(0),
    ))
}

/// What one part does to a creature with these modifiers.
fn applied_amount(creature: &Creature, part: &DamageRoll) -> i32 {
    let ty = part.damage_type as i32;
    let mut n = part.rolled.max(0);
    if part.halved {
        n /= 2;
    }
    if creature.immunities.contains(&ty) {
        return 0;
    }
    if creature.resistances.contains(&ty) {
        n /= 2;
    }
    if creature.vulnerabilities.contains(&ty) {
        n = n.saturating_mul(2);
    }
    n
}

/// Apply `parts` to `token`, whose creature is `creature`. See the module
/// docs for the order of the rules.
pub fn apply_damage(token: &Token, creature: &Creature, parts: &[DamageRoll]) -> DamageResult {
    let damage: Vec<DamageApplied> = parts
        .iter()
        .map(|p| DamageApplied {
            r#type: p.damage_type as i32,
            rolled: p.rolled,
            applied: applied_amount(creature, p),
        })
        .collect();
    let Some((current, max, temp)) = hit_points(token) else {
        return DamageResult {
            token: token.clone(),
            damage,
            dropped_to_zero: false,
            no_hit_points: true,
        };
    };
    let total = damage
        .iter()
        .fold(0_i32, |acc, d| acc.saturating_add(d.applied));
    let from_temp = total.min(temp);
    let new_current = (current - (total - from_temp)).max(0);
    let mut out = token.clone();
    if total > 0 {
        out.hp_temp = token.hp_temp.map(|_| temp - from_temp);
        out.hp_current = Some(new_current);
        out.health_state = health_state(new_current, max) as i32;
    }
    DamageResult {
        token: out,
        damage,
        dropped_to_zero: current > 0 && new_current == 0,
        no_hit_points: false,
    }
}

/// Heal `token` by up to `amount` (a negative amount counts as 0).
pub fn apply_healing(token: &Token, amount: i32) -> HealingResult {
    let Some((current, max, _)) = hit_points(token) else {
        return HealingResult {
            token: token.clone(),
            restored: 0,
            no_hit_points: true,
        };
    };
    let new_current = current.saturating_add(amount.max(0)).min(max).max(current);
    let mut out = token.clone();
    if new_current != current {
        out.hp_current = Some(new_current);
        out.health_state = health_state(new_current, max) as i32;
    }
    HealingResult {
        token: out,
        restored: new_current - current,
        no_hit_points: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tok(current: i32, max: i32, temp: i32) -> Token {
        Token {
            token_id: "t".to_string(),
            hp_current: Some(current),
            hp_max: Some(max),
            hp_temp: Some(temp),
            health_state: health_state(current, max) as i32,
            ..Default::default()
        }
    }

    fn cre(res: &[DamageType], imm: &[DamageType], vul: &[DamageType]) -> Creature {
        let ids = |v: &[DamageType]| v.iter().map(|d| *d as i32).collect();
        Creature {
            creature_id: "c".to_string(),
            resistances: ids(res),
            immunities: ids(imm),
            vulnerabilities: ids(vul),
            hp_max: 99,
            hp_current: 99,
            ..Default::default()
        }
    }

    fn roll(ty: DamageType, rolled: i32) -> DamageRoll {
        DamageRoll {
            damage_type: ty,
            rolled,
            halved: false,
        }
    }

    fn applied(r: &DamageResult) -> Vec<i32> {
        r.damage.iter().map(|d| d.applied).collect()
    }

    #[test]
    fn health_state_thresholds_for_9_and_28() {
        use HealthState::*;
        let want = [
            Down, NearDeath, NearDeath, Bloodied, Bloodied, Healthy, Healthy, Healthy, Healthy,
            Healthy,
        ];
        for (hp, w) in want.into_iter().enumerate() {
            assert_eq!(health_state(hp as i32, 9), w, "hp {hp} of 9");
        }
        assert_eq!(health_state(14, 28), Bloodied);
        assert_eq!(health_state(7, 28), NearDeath);
        assert_eq!(health_state(8, 28), Bloodied);
        assert_eq!(health_state(5, 0), Unspecified);
        assert_eq!(health_state(5, -3), Unspecified);
    }

    #[test]
    fn each_modifier_alone() {
        let c = cre(
            &[DamageType::Piercing],
            &[DamageType::Fire],
            &[DamageType::Cold],
        );
        let t = tok(50, 50, 0);
        let r = apply_damage(
            &t,
            &c,
            &[
                roll(DamageType::Piercing, 7),
                roll(DamageType::Fire, 9),
                roll(DamageType::Cold, 5),
                roll(DamageType::Slashing, 4),
            ],
        );
        assert_eq!(applied(&r), [3, 0, 10, 4]);
        assert_eq!(r.damage[0].rolled, 7);
        assert_eq!(r.damage[0].r#type, DamageType::Piercing as i32);
        assert_eq!(r.token.hp_current, Some(50 - 17));
    }

    #[test]
    fn resistance_with_vulnerability_halves_then_doubles() {
        let c = cre(&[DamageType::Fire], &[], &[DamageType::Fire]);
        let r = apply_damage(&tok(50, 50, 0), &c, &[roll(DamageType::Fire, 5)]);
        // 5 / 2 = 2, doubled = 4.
        assert_eq!(applied(&r), [4]);
    }

    #[test]
    fn duplicate_listing_counts_once() {
        let c = cre(
            &[DamageType::Fire, DamageType::Fire],
            &[],
            &[DamageType::Cold, DamageType::Cold],
        );
        let r = apply_damage(
            &tok(50, 50, 0),
            &c,
            &[roll(DamageType::Fire, 8), roll(DamageType::Cold, 3)],
        );
        assert_eq!(applied(&r), [4, 6]);
    }

    #[test]
    fn negative_total_counts_as_zero_and_halving_rounds_down() {
        let c = cre(&[], &[], &[]);
        let half = DamageRoll {
            damage_type: DamageType::Fire,
            rolled: 15,
            halved: true,
        };
        let r = apply_damage(&tok(20, 20, 0), &c, &[roll(DamageType::Fire, -4), half]);
        assert_eq!(applied(&r), [0, 7]);
        assert_eq!(r.damage[0].rolled, -4);
        assert_eq!(r.token.hp_current, Some(13));
        // Halved, then resistant: 15 -> 7 -> 3; halved then vulnerable: 14.
        let rc = cre(&[DamageType::Fire], &[], &[]);
        assert_eq!(applied(&apply_damage(&tok(20, 20, 0), &rc, &[half])), [3]);
        let vc = cre(&[], &[], &[DamageType::Fire]);
        assert_eq!(applied(&apply_damage(&tok(20, 20, 0), &vc, &[half])), [14]);
    }

    #[test]
    fn temporary_hp_absorbs_part_and_all_of_a_hit() {
        let c = cre(&[], &[], &[]);
        let part = apply_damage(&tok(10, 10, 4), &c, &[roll(DamageType::Fire, 6)]);
        assert_eq!(part.token.hp_temp, Some(0));
        assert_eq!(part.token.hp_current, Some(8));
        let all = apply_damage(&tok(10, 10, 4), &c, &[roll(DamageType::Fire, 3)]);
        assert_eq!(all.token.hp_temp, Some(1));
        assert_eq!(all.token.hp_current, Some(10));
        assert_eq!(all.token.health_state, HealthState::Healthy as i32);
        assert!(!all.dropped_to_zero);
    }

    #[test]
    fn floor_at_zero_and_drop_flag() {
        let c = cre(&[], &[], &[]);
        let r = apply_damage(&tok(3, 9, 0), &c, &[roll(DamageType::Fire, 20)]);
        assert_eq!(r.token.hp_current, Some(0));
        assert_eq!(r.token.health_state, HealthState::Down as i32);
        assert!(r.dropped_to_zero);
        // Already at 0: nothing changes, no drop.
        let again = apply_damage(&r.token, &c, &[roll(DamageType::Fire, 5)]);
        assert_eq!(again.token, r.token);
        assert!(!again.dropped_to_zero);
        assert_eq!(applied(&again), [5]);
        // Zero damage to a healthy token changes nothing.
        let t = tok(9, 9, 0);
        let none = apply_damage(&t, &c, &[roll(DamageType::Fire, 0)]);
        assert_eq!(none.token, t);
    }

    #[test]
    fn healing_caps_at_max_keeps_temp_and_raises_the_downed() {
        let capped = apply_healing(&tok(5, 9, 2), 10);
        assert_eq!(capped.restored, 4);
        assert_eq!(capped.token.hp_current, Some(9));
        assert_eq!(capped.token.hp_temp, Some(2));
        assert_eq!(capped.token.health_state, HealthState::Healthy as i32);
        let up = apply_healing(&tok(0, 9, 0), 1);
        assert_eq!(up.restored, 1);
        assert_eq!(up.token.health_state, HealthState::NearDeath as i32);
        let full = tok(9, 9, 0);
        let none = apply_healing(&full, 5);
        assert_eq!((none.restored, none.token), (0, full));
    }

    #[test]
    fn no_hp_max_applies_nothing_but_still_computes_applied() {
        let c = cre(&[DamageType::Fire], &[], &[]);
        let t = Token {
            token_id: "t".to_string(),
            health_state: HealthState::Healthy as i32,
            ..Default::default()
        };
        let r = apply_damage(&t, &c, &[roll(DamageType::Fire, 9)]);
        assert!(r.no_hit_points);
        assert_eq!(r.token, t);
        assert_eq!(applied(&r), [4]);
        assert!(!r.dropped_to_zero);
        let h = apply_healing(&t, 5);
        assert!(h.no_hit_points);
        assert_eq!((h.restored, h.token), (0, t));
    }
}
