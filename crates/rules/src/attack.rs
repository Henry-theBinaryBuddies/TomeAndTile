//! Attack resolution: an attack roll against armor class, and damage dice on
//! a hit (ADR 0005, ADR 0006), with targeting checks that warn and never
//! block.
//!
//! Pure: no I/O, no clock, no id minting. Randomness is an injected
//! [`Rng`]. Nothing here changes hit points: damage is rolled but not applied
//! (that is a later milestone), so `TargetOutcome.damage` stays empty,
//! `dropped_to_zero` stays false and no target creature is modified. The only
//! change is to the source creature's resources (see below). `roll_id` and
//! `resolution_id` are left empty for the server to fill.
//!
//! # Targeting checks
//!
//! Targets are (token, creature) pairs; the token gives the id, position and
//! size. The source token and the scene's [`Grid`] are passed too. Problems
//! become [`Warning`]s in `Resolution.warnings`; the ability still resolves.
//! Warnings come in this order: `not_enough_resource`, then
//! `too_many_targets` or `extra_targets_ignored`, then one `out_of_range`
//! per out-of-range target in target order.
//!
//! | code                    | given when |
//! |-------------------------|------------|
//! | `out_of_range`          | a target is beyond long range, or beyond `range_feet` when there is no long range. One per such target; the message names the token id and the distance. The attack is still rolled. |
//! | `too_many_targets`      | an ability with one attack step lists more targets than `max_targets` (0 means no limit). At most once; every target is still resolved. |
//! | `not_enough_resource`   | `resource_key` names a resource the creature lacks, or one with `current` below `resource_amount`. Once per use. |
//! | `extra_targets_ignored` | an ability with several attack steps lists more targets than it has steps. At most once; never together with `too_many_targets`. |
//!
//! Range, per target (each attack uses its own target):
//!
//! - The limit is `range_feet`; a `range_feet` of 0 (touch) counts as 5 feet.
//!   `long_range_feet` of 0 (or not above the limit) means no long range.
//! - Distance comes from [`distance_feet`](crate::distance::distance_feet).
//!   Where it is `None` (not a square grid, zero `cell_size` or
//!   `feet_per_cell`, no token position) range is not checked at all.
//! - Farther than the limit but within long range: that attack only has
//!   disadvantage. Normal becomes Disadvantage, Advantage becomes Normal
//!   (they cancel), Disadvantage stays Disadvantage.
//! - Beyond the limit with no long range, or beyond long range: an
//!   `out_of_range` warning, and the attack is rolled with the mode it would
//!   otherwise have.
//!
//! Targeting shape:
//!
//! - `TARGET_SHAPE_CREATURES` with no targets is [`AttackError::NoTargets`].
//! - `SPHERE`, `CONE`, `LINE` and `CUBE` are [`AttackError::NotImplemented`].
//! - `TARGET_SHAPE_UNSPECIFIED` (with a `Targeting` present) is checked like
//!   `CREATURES` for range and count, but an empty target list is allowed.
//! - `TARGET_SHAPE_SELF`, or an ability with no `Targeting` at all, gets no
//!   range or `too_many_targets` check; listed targets are resolved as given,
//!   except that an ability with several attack steps still ignores entries
//!   past its last step and gives `extra_targets_ignored`, whatever its
//!   `Targeting`.
//!
//! Resources: when `resource_key` is set, `resource_amount` is spent once per
//! use (not per target) from the matching `Resource.current` on the returned
//! source creature, flooring at 0, before any die is rolled. A missing
//! resource or too few uses warns and the ability still resolves. An amount
//! of 0 or less spends nothing. Validation errors are returned before
//! anything is spent or rolled.

use std::fmt;

use tt_contracts::v1::{
    effect_step, Ability, AttackStep, Creature, Grid, HitOutcome, Resolution, ResolutionState,
    RollMode, RollPurpose, RollResult, TargetOutcome, TargetShape, Targeting, Token, Warning,
};

use crate::dice::{self, Expr, ParseError, Rng};
use crate::distance::distance_feet;

/// Why an ability could not be resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttackError {
    /// The ability id is not on the source creature.
    UnknownAbility,
    /// The ability has a step this engine cannot resolve yet: a save, flat,
    /// apply-effect or plugin step (or an empty step).
    NotImplemented,
    /// The ability targets creatures but none were listed.
    NoTargets,
    /// A dice expression (or the attack's `1d20` expression) was rejected.
    Dice(ParseError),
}

impl AttackError {
    /// Stable snake_case code for clients and logs. The server maps
    /// `unknown_ability`, `not_implemented` and `no_targets` to the rejections
    /// of the same name; dice errors carry the [`ParseError::code`].
    pub fn code(&self) -> &'static str {
        match self {
            AttackError::UnknownAbility => "unknown_ability",
            AttackError::NotImplemented => "not_implemented",
            AttackError::NoTargets => "no_targets",
            AttackError::Dice(e) => e.code(),
        }
    }
}

impl fmt::Display for AttackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AttackError::UnknownAbility => write!(f, "the creature has no such ability"),
            AttackError::NotImplemented => {
                write!(f, "the ability has a step that is not implemented yet")
            }
            AttackError::NoTargets => write!(f, "the ability needs at least one target"),
            AttackError::Dice(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for AttackError {}

impl From<ParseError> for AttackError {
    fn from(e: ParseError) -> Self {
        AttackError::Dice(e)
    }
}

/// One attack step with its dice parsed ahead of any rolling, so bad dice
/// text fails the whole resolution before a die is thrown.
pub(crate) struct Prepared {
    pub(crate) step_index: u32,
    pub(crate) attack: Expr,
    /// Each damage part as written, and with its dice doubled for a critical.
    pub(crate) damage: Vec<(Expr, Expr)>,
}

/// Parse one attack step's dice (the attack roll, each damage part and its
/// critical double) without rolling anything.
pub(crate) fn prepare_attack(step_index: usize, a: &AttackStep) -> Result<Prepared, ParseError> {
    let attack = Expr::parse(&format!("1d20{:+}", a.to_hit_bonus))?;
    let mut damage = Vec::with_capacity(a.damage.len());
    for part in &a.damage {
        let normal = Expr::parse(&part.dice)?;
        // A critical that would exceed the dice limit is an error found
        // now, not only when a critical happens to be rolled.
        let doubled = normal.doubled_dice().ok_or(ParseError::TooManyDice)?;
        damage.push((normal, doubled));
    }
    Ok(Prepared {
        // A step list is far shorter than u32::MAX; saturate regardless.
        step_index: u32::try_from(step_index).unwrap_or(u32::MAX),
        attack,
        damage,
    })
}

fn prepare(ability: &Ability) -> Result<Vec<Prepared>, AttackError> {
    let mut out = Vec::new();
    for (i, step) in ability.steps.iter().enumerate() {
        let Some(effect_step::Step::Attack(a)) = &step.step else {
            return Err(AttackError::NotImplemented);
        };
        out.push(prepare_attack(i, a)?);
    }
    Ok(out)
}

pub(crate) fn roll_result(
    expr: &Expr,
    mode: RollMode,
    rng: &mut dyn Rng,
    purpose: RollPurpose,
    roller_creature_id: &str,
    label: String,
) -> RollResult {
    let rolled = dice::roll(expr, mode, rng);
    RollResult {
        roll_id: String::new(),
        notation: expr.canonical(),
        purpose: purpose as i32,
        mode: rolled.mode as i32,
        dice: rolled.dice,
        modifiers: dice::to_modifiers(expr),
        total: rolled.total,
        critical: rolled.critical,
        fumble: rolled.fumble,
        visibility: 0,
        roller_user_id: String::new(),
        roller_creature_id: roller_creature_id.to_string(),
        label,
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn resolve_one(
    source: &Creature,
    ability: &Ability,
    step: &Prepared,
    target_token_id: &str,
    target: &Creature,
    mode: RollMode,
    auto_crit: bool,
    rng: &mut dyn Rng,
) -> TargetOutcome {
    let attack_roll = roll_result(
        &step.attack,
        mode,
        rng,
        RollPurpose::Attack,
        &source.creature_id,
        format!("{} attack", ability.name),
    );
    let outcome = if attack_roll.critical {
        HitOutcome::CriticalHit
    } else if attack_roll.fumble || attack_roll.total < target.armor_class {
        HitOutcome::Miss
    } else if auto_crit {
        // Paralyzed or unconscious target within 5 feet (conditions.rs).
        HitOutcome::CriticalHit
    } else {
        HitOutcome::Hit
    };
    let mut damage_rolls = Vec::new();
    if outcome != HitOutcome::Miss {
        for (normal, doubled) in &step.damage {
            let expr = if outcome == HitOutcome::CriticalHit {
                doubled
            } else {
                normal
            };
            damage_rolls.push(roll_result(
                expr,
                RollMode::Normal,
                rng,
                RollPurpose::Damage,
                &source.creature_id,
                format!("{} damage", ability.name),
            ));
        }
    }
    TargetOutcome {
        token_id: target_token_id.to_string(),
        outcome: outcome as i32,
        roll: Some(attack_roll),
        damage_rolls,
        damage: Vec::new(),
        healing: 0,
        effects_applied: Vec::new(),
        dropped_to_zero: false,
        step_index: step.step_index,
    }
}

/// The result of [`resolve_attacks`].
#[derive(Debug, Clone, PartialEq)]
pub struct AttackResolution {
    /// Outcomes and warnings, with `state` done.
    pub resolution: Resolution,
    /// The source creature after the ability's resource was spent. Equal to
    /// the input when the ability costs nothing.
    pub source: Creature,
}

pub(crate) fn warning(code: &str, message: String) -> Warning {
    Warning {
        code: code.to_string(),
        message,
    }
}

/// Range limit in feet: 0 (touch) counts as 5, and so does anything that is
/// not a positive number.
fn range_limit(t: &Targeting) -> f64 {
    let r = f64::from(t.range_feet);
    if r > 0.0 {
        r
    } else {
        5.0
    }
}

/// Apply the range check for one target: the attack's mode, and a warning
/// when the target is out of range.
pub(crate) fn check_range(
    targeting: &Targeting,
    grid: &Grid,
    source: &Token,
    target: &Token,
    mode: RollMode,
    warnings: &mut Vec<Warning>,
) -> RollMode {
    let Some(dist) = distance_feet(grid, source, target) else {
        return mode;
    };
    let dist = f64::from(dist);
    let limit = range_limit(targeting);
    if dist <= limit {
        return mode;
    }
    let long = f64::from(targeting.long_range_feet);
    if long > limit && dist <= long {
        return match mode {
            RollMode::Advantage => RollMode::Normal,
            _ => RollMode::Disadvantage,
        };
    }
    warnings.push(warning(
        "out_of_range",
        format!(
            "target {} is {} ft away, beyond the {} ft limit",
            target.token_id,
            dist,
            if long > limit { long } else { limit }
        ),
    ));
    mode
}

/// Spend `ability`'s resource once from `updated`, warning (never blocking)
/// when it is missing or too low. Does nothing when no `resource_key` is set.
pub(crate) fn spend_resource(
    updated: &mut Creature,
    ability: &Ability,
    warnings: &mut Vec<Warning>,
) {
    if !ability.resource_key.is_empty() {
        let amount = ability.resource_amount.max(0);
        match updated
            .resources
            .iter_mut()
            .find(|r| r.key == ability.resource_key)
        {
            Some(r) => {
                if r.current < amount {
                    warnings.push(warning(
                        "not_enough_resource",
                        format!(
                            "{} needs {} {} but only {} left",
                            ability.name, amount, ability.resource_key, r.current
                        ),
                    ));
                }
                r.current = r.current.saturating_sub(amount).max(0);
            }
            None => warnings.push(warning(
                "not_enough_resource",
                format!(
                    "{} needs {} {} but the creature has none",
                    ability.name, amount, ability.resource_key
                ),
            )),
        }
    }
}

/// Resolve `ability_id` on `source`, an ability made only of attack steps.
///
/// `targets` are (token, creature) pairs in the order listed. With one
/// attack step every entry is a target of that step. With several, entry `n`
/// is the target of the `n`th attack step: entries past the last step are
/// ignored and steps past the last entry are not rolled. Each outcome carries
/// its `step_index`. An ability with no steps resolves to a done `Resolution`
/// with no outcomes.
///
/// Range, target count and resource checks give warnings and never block;
/// the crate docs above list each code. The returned source creature has
/// the ability's resource spent.
///
/// Errors: [`AttackError::UnknownAbility`], [`AttackError::NotImplemented`]
/// for an area shape or any non-attack step, [`AttackError::NoTargets`], or
/// [`AttackError::Dice`] for dice that do not parse. All are detected before
/// anything is spent or any die is rolled.
pub fn resolve_attacks(
    source_token: &Token,
    source: &Creature,
    ability_id: &str,
    targets: &[(Token, Creature)],
    grid: &Grid,
    mode: RollMode,
    rng: &mut dyn Rng,
) -> Result<AttackResolution, AttackError> {
    let ability = source
        .abilities
        .iter()
        .find(|a| a.ability_id == ability_id)
        .ok_or(AttackError::UnknownAbility)?;

    // Shape: which targeting (if any) the range and count checks use.
    let shape = ability
        .targeting
        .as_ref()
        .map(|t| TargetShape::try_from(t.shape).unwrap_or(TargetShape::Unspecified));
    let checked = match shape {
        Some(TargetShape::Sphere | TargetShape::Cone | TargetShape::Line | TargetShape::Cube) => {
            return Err(AttackError::NotImplemented)
        }
        Some(TargetShape::Creatures) if targets.is_empty() => return Err(AttackError::NoTargets),
        Some(TargetShape::Creatures | TargetShape::Unspecified) => ability.targeting.as_ref(),
        Some(TargetShape::Self_) | None => None,
    };
    let steps = prepare(ability)?;

    let mut warnings = Vec::new();
    let mut updated = source.clone();

    // Resource: spent once per use, before rolling.
    spend_resource(&mut updated, ability, &mut warnings);

    // Which (step, target) pairs are rolled.
    let pairs: Vec<(&Prepared, &(Token, Creature))> = if let [only] = steps.as_slice() {
        targets.iter().map(|t| (only, t)).collect()
    } else {
        steps.iter().zip(targets).collect()
    };

    if steps.len() == 1 {
        if let Some(t) = checked {
            if t.max_targets > 0 && targets.len() > t.max_targets as usize {
                warnings.push(warning(
                    "too_many_targets",
                    format!(
                        "{} lists {} targets but allows {}",
                        ability.name,
                        targets.len(),
                        t.max_targets
                    ),
                ));
            }
        }
    } else if steps.len() > 1 && targets.len() > steps.len() {
        warnings.push(warning(
            "extra_targets_ignored",
            format!(
                "{} has {} attacks but {} targets were listed; the extra were ignored",
                ability.name,
                steps.len(),
                targets.len()
            ),
        ));
    }

    let mut outcomes = Vec::with_capacity(pairs.len());
    for (step, (token, creature)) in pairs {
        let attack_mode = match checked {
            Some(t) => check_range(t, grid, source_token, token, mode, &mut warnings),
            None => mode,
        };
        outcomes.push(resolve_one(
            source,
            ability,
            step,
            &token.token_id,
            creature,
            attack_mode,
            false,
            rng,
        ));
    }

    Ok(AttackResolution {
        resolution: Resolution {
            resolution_id: String::new(),
            source_token_id: source_token.token_id.clone(),
            ability_id: ability.ability_id.clone(),
            ability_name: ability.name.clone(),
            state: ResolutionState::Done as i32,
            outcomes,
            warnings,
        },
        source: updated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tt_contracts::v1::{
        ApplyEffectStep, AttackStep, CreatureSize, DamagePart, DamageType, DiagonalRule,
        EffectStep, FlatStep, GridKind, PluginStep, Resource, SaveStep, Vec2,
    };

    struct Scripted {
        values: Vec<u64>,
        calls: usize,
    }

    impl Scripted {
        /// Script that makes `roll_die` produce these faces (face - 1).
        fn faces(faces: &[u64]) -> Self {
            Self {
                values: faces.iter().map(|f| f - 1).collect(),
                calls: 0,
            }
        }
    }

    impl Rng for Scripted {
        fn next_u64(&mut self) -> u64 {
            let v = self.values[self.calls];
            self.calls += 1;
            v
        }
    }

    fn attack_step(bonus: i32, dice: &[&str], ty: DamageType) -> EffectStep {
        EffectStep {
            step: Some(effect_step::Step::Attack(AttackStep {
                to_hit_bonus: bonus,
                damage: dice
                    .iter()
                    .map(|d| DamagePart {
                        dice: (*d).to_string(),
                        r#type: ty as i32,
                    })
                    .collect(),
            })),
        }
    }

    fn creature(id: &str, ac: i32, abilities: Vec<Ability>) -> Creature {
        Creature {
            creature_id: id.to_string(),
            armor_class: ac,
            abilities,
            ..Default::default()
        }
    }

    fn ability(id: &str, name: &str, steps: Vec<EffectStep>) -> Ability {
        Ability {
            ability_id: id.to_string(),
            name: name.to_string(),
            steps,
            ..Default::default()
        }
    }

    /// Brannoch's longsword: +5, 1d8+3 slashing.
    fn brannoch() -> Creature {
        creature(
            "brannoch",
            18,
            vec![ability(
                "longsword",
                "Longsword",
                vec![attack_step(5, &["1d8+3"], DamageType::Slashing)],
            )],
        )
    }

    /// Ilvaren's Scorching Ray: three +5 rays of 2d6 fire.
    fn ilvaren() -> Creature {
        creature(
            "ilvaren",
            12,
            vec![ability(
                "scorching-ray",
                "Scorching Ray",
                vec![attack_step(5, &["2d6"], DamageType::Fire); 3],
            )],
        )
    }

    /// A Medium token centred in cell (cx, cy) of the 70-unit test grid.
    fn token_at(id: &str, cx: f32, cy: f32) -> Token {
        Token {
            token_id: id.to_string(),
            position: Some(Vec2 {
                x: (cx + 0.5) * 70.0,
                y: (cy + 0.5) * 70.0,
            }),
            size: CreatureSize::Medium as i32,
            ..Default::default()
        }
    }

    /// Standard 5e square grid: 70 units a cell, 5 ft, every diagonal 5 ft.
    fn grid() -> Grid {
        Grid {
            kind: GridKind::Square as i32,
            cell_size: 70.0,
            offset: None,
            feet_per_cell: 5.0,
            diagonal_rule: DiagonalRule::Five as i32,
        }
    }

    /// The source token, in cell (0, 0).
    fn src_token() -> Token {
        token_at("tok-src", 0.0, 0.0)
    }

    /// A target one cell (5 ft) east of the source, with the given AC.
    fn tgt(token: &str, ac: i32) -> (Token, Creature) {
        tgt_at(token, ac, 1.0)
    }

    /// A target `cells` cells east of the source (5 ft each).
    fn tgt_at(token: &str, ac: i32, cells: f32) -> (Token, Creature) {
        (
            token_at(token, cells, 0.0),
            creature(&format!("c-{token}"), ac, Vec::new()),
        )
    }

    /// Run `resolve_attacks` and `resolve_ability` on the same input and
    /// script, and require them to agree on the result (apart from applied hit points) (or the error code)
    /// and on how many dice they drew. Returns the old result and the number
    /// of draws, so every M2 test also exercises `resolve_ability`.
    fn dual(
        source: &Creature,
        id: &str,
        targets: &[(Token, Creature)],
        grid: &Grid,
        mode: RollMode,
        faces: &[u64],
    ) -> (Result<AttackResolution, AttackError>, usize) {
        let mut old_rng = Scripted::faces(faces);
        let mut new_rng = Scripted::faces(faces);
        let old = resolve_attacks(&src_token(), source, id, targets, grid, mode, &mut old_rng);
        let new = crate::ability::resolve_ability(
            &src_token(),
            source,
            id,
            targets,
            grid,
            mode,
            None,
            &mut new_rng,
        );
        assert_eq!(old_rng.calls, new_rng.calls, "dice drawn differ");
        match (&old, &new) {
            (Ok(o), Ok(n)) => {
                // resolve_ability also applies hit points (#98): compare
                // without what that adds, which resolve_attacks never fills.
                let mut applied = n.resolution.clone();
                applied.warnings.retain(|w| w.code != "no_hit_points");
                for out in &mut applied.outcomes {
                    out.damage.clear();
                    out.healing = 0;
                    out.dropped_to_zero = false;
                }
                assert_eq!(o.resolution, applied);
                assert_eq!(o.source, n.source);
                assert_eq!(n.pending.len(), n.resolution.outcomes.len());
            }
            (Err(o), Err(n)) => assert_eq!(o.code(), n.code()),
            _ => panic!("resolve_attacks and resolve_ability disagree: {old:?} vs {new:?}"),
        }
        (old, old_rng.calls)
    }

    fn run_full(
        source: &Creature,
        id: &str,
        targets: &[(Token, Creature)],
        mode: RollMode,
        faces: &[u64],
    ) -> Result<AttackResolution, AttackError> {
        dual(source, id, targets, &grid(), mode, faces).0
    }

    /// `resolve_attacks` alone, for the cases where `resolve_ability` now
    /// differs on purpose (area shapes, save/flat/apply-effect steps).
    fn run_old(
        source: &Creature,
        id: &str,
        targets: &[(Token, Creature)],
        mode: RollMode,
        faces: &[u64],
    ) -> Result<Resolution, AttackError> {
        resolve_attacks(
            &src_token(),
            source,
            id,
            targets,
            &grid(),
            mode,
            &mut Scripted::faces(faces),
        )
        .map(|r| r.resolution)
    }

    fn resolve_old_full(
        source: &Creature,
        id: &str,
        targets: &[(Token, Creature)],
        mode: RollMode,
        faces: &[u64],
    ) -> Result<AttackResolution, AttackError> {
        resolve_attacks(
            &src_token(),
            source,
            id,
            targets,
            &grid(),
            mode,
            &mut Scripted::faces(faces),
        )
    }

    fn run(
        source: &Creature,
        id: &str,
        targets: &[(Token, Creature)],
        mode: RollMode,
        faces: &[u64],
    ) -> Result<Resolution, AttackError> {
        run_full(source, id, targets, mode, faces).map(|r| r.resolution)
    }

    fn codes(r: &Resolution) -> Vec<&str> {
        r.warnings.iter().map(|w| w.code.as_str()).collect()
    }

    fn roll_mode(o: &TargetOutcome) -> RollMode {
        RollMode::try_from(o.roll.as_ref().unwrap().mode).unwrap()
    }

    fn targeting(shape: TargetShape, range: f32, long: f32, max: u32) -> Targeting {
        Targeting {
            shape: shape as i32,
            range_feet: range,
            long_range_feet: long,
            max_targets: max,
            ..Default::default()
        }
    }

    /// Korvath's longbow from the fixture: +7, 1d8+3, 150/600 ft, one target.
    fn korvath() -> Creature {
        let mut a = ability(
            "longbow",
            "Longbow",
            vec![attack_step(7, &["1d8+3"], DamageType::Piercing)],
        );
        a.targeting = Some(targeting(TargetShape::Creatures, 150.0, 600.0, 1));
        creature("korvath", 16, vec![a])
    }

    /// Ilvaren's Fire Bolt from the fixture: +5, 1d10, 120 ft, one target.
    fn ilvaren_fire_bolt() -> Creature {
        let mut a = ability(
            "cantrip-fire-bolt",
            "Fire Bolt",
            vec![attack_step(5, &["1d10"], DamageType::Fire)],
        );
        a.targeting = Some(targeting(TargetShape::Creatures, 120.0, 0.0, 1));
        creature("ilvaren", 12, vec![a])
    }

    /// Scorching Ray as in the fixture: spell_slot_2 x1, 120 ft, 3 rays, and
    /// two slots to spend.
    fn ilvaren_with_slots(current: i32) -> Creature {
        let mut c = ilvaren();
        c.abilities[0].resource_key = "spell_slot_2".to_string();
        c.abilities[0].resource_amount = 1;
        c.abilities[0].targeting = Some(targeting(TargetShape::Creatures, 120.0, 0.0, 3));
        c.resources = vec![
            Resource {
                key: "spell_slot_1".to_string(),
                label: String::new(),
                current: 4,
                max: 4,
            },
            Resource {
                key: "spell_slot_2".to_string(),
                label: String::new(),
                current,
                max: 2,
            },
        ];
        c
    }

    fn brannoch_longsword() -> Creature {
        let mut c = brannoch();
        c.abilities[0].targeting = Some(targeting(TargetShape::Creatures, 5.0, 0.0, 1));
        c
    }

    fn out_kind(o: &TargetOutcome) -> HitOutcome {
        HitOutcome::try_from(o.outcome).unwrap()
    }

    #[test]
    fn hit_rolls_damage_and_envelope_is_filled() {
        let src = brannoch();
        let r = run(
            &src,
            "longsword",
            &[tgt("t1", 15)],
            RollMode::Normal,
            &[10, 5],
        )
        .unwrap();
        assert_eq!(r.state, ResolutionState::Done as i32);
        assert_eq!(r.source_token_id, "tok-src");
        assert_eq!(r.ability_id, "longsword");
        assert_eq!(r.ability_name, "Longsword");
        assert!(r.resolution_id.is_empty());
        assert!(r.warnings.is_empty());
        let o = &r.outcomes[0];
        assert_eq!(out_kind(o), HitOutcome::Hit);
        assert_eq!(o.token_id, "t1");
        assert_eq!(o.step_index, 0);
        let a = o.roll.as_ref().unwrap();
        assert_eq!(a.purpose, RollPurpose::Attack as i32);
        assert_eq!(a.notation, "1d20+5");
        assert_eq!(a.total, 15);
        assert_eq!(a.mode, RollMode::Normal as i32);
        assert_eq!(a.roller_creature_id, "brannoch");
        assert_eq!(a.label, "Longsword attack");
        assert!(a.roll_id.is_empty());
        assert_eq!(a.modifiers.len(), 1);
        assert_eq!(a.modifiers[0].value, 5);
        assert_eq!(o.damage_rolls.len(), 1);
        let d = &o.damage_rolls[0];
        assert_eq!(d.purpose, RollPurpose::Damage as i32);
        assert_eq!(d.notation, "1d8+3");
        assert_eq!(d.total, 8);
        assert_eq!(d.label, "Longsword damage");
        assert_eq!(d.roller_creature_id, "brannoch");
        assert!(o.damage.is_empty());
        assert!(!o.dropped_to_zero);
    }

    #[test]
    fn total_equal_to_ac_hits_and_lower_misses_without_damage() {
        let src = brannoch();
        let r = run(
            &src,
            "longsword",
            &[tgt("t", 15)],
            RollMode::Normal,
            &[10, 1],
        )
        .unwrap();
        assert_eq!(out_kind(&r.outcomes[0]), HitOutcome::Hit);
        // Miss: only the d20 is drawn, so a one-value script must suffice.
        let r = run(&src, "longsword", &[tgt("t", 16)], RollMode::Normal, &[10]).unwrap();
        let o = &r.outcomes[0];
        assert_eq!(out_kind(o), HitOutcome::Miss);
        assert!(o.damage_rolls.is_empty());
        assert!(o.roll.is_some());
    }

    #[test]
    fn natural_20_crit_doubles_dice_not_modifiers() {
        let src = brannoch();
        // d20 = 20, then 2d8: 4 and 6, +3 constant.
        let r = run(
            &src,
            "longsword",
            &[tgt("t", 30)],
            RollMode::Normal,
            &[20, 4, 6],
        )
        .unwrap();
        let o = &r.outcomes[0];
        assert_eq!(out_kind(o), HitOutcome::CriticalHit);
        assert!(o.roll.as_ref().unwrap().critical);
        let d = &o.damage_rolls[0];
        assert_eq!(d.notation, "2d8+3");
        assert_eq!(d.dice.len(), 2);
        assert_eq!(d.modifiers.len(), 1);
        assert_eq!(d.modifiers[0].value, 3);
        assert_eq!(d.total, 13);
    }

    #[test]
    fn natural_1_misses_despite_high_bonus() {
        let mut src = brannoch();
        src.abilities[0].steps = vec![attack_step(50, &["1d8+3"], DamageType::Slashing)];
        let r = run(&src, "longsword", &[tgt("t", 10)], RollMode::Normal, &[1]).unwrap();
        let o = &r.outcomes[0];
        assert_eq!(out_kind(o), HitOutcome::Miss);
        assert!(o.roll.as_ref().unwrap().fumble);
        assert_eq!(o.roll.as_ref().unwrap().total, 51);
        assert!(o.damage_rolls.is_empty());
    }

    #[test]
    fn natural_20_hits_regardless_of_ac() {
        let mut src = brannoch();
        src.abilities[0].steps = vec![attack_step(-5, &["1d8"], DamageType::Slashing)];
        let r = run(
            &src,
            "longsword",
            &[tgt("t", 30)],
            RollMode::Normal,
            &[20, 2, 3],
        )
        .unwrap();
        assert_eq!(out_kind(&r.outcomes[0]), HitOutcome::CriticalHit);
    }

    #[test]
    fn advantage_and_disadvantage_come_from_the_mode() {
        let src = brannoch();
        let r = run(
            &src,
            "longsword",
            &[tgt("t", 15)],
            RollMode::Advantage,
            &[4, 12, 5],
        )
        .unwrap();
        let a = r.outcomes[0].roll.as_ref().unwrap();
        assert_eq!(a.mode, RollMode::Advantage as i32);
        assert_eq!(a.dice.len(), 2);
        assert!(a.dice[0].dropped);
        assert_eq!(a.total, 17);
        assert_eq!(out_kind(&r.outcomes[0]), HitOutcome::Hit);
        // Damage is never rolled with the attack's mode.
        assert_eq!(r.outcomes[0].damage_rolls[0].mode, RollMode::Normal as i32);

        let r = run(
            &src,
            "longsword",
            &[tgt("t", 15)],
            RollMode::Disadvantage,
            &[4, 12],
        )
        .unwrap();
        let a = r.outcomes[0].roll.as_ref().unwrap();
        assert_eq!(a.mode, RollMode::Disadvantage as i32);
        assert!(a.dice[1].dropped);
        assert_eq!(a.total, 9);
        assert_eq!(out_kind(&r.outcomes[0]), HitOutcome::Miss);
    }

    #[test]
    fn negative_and_zero_bonus_notation() {
        let mut src = brannoch();
        src.abilities[0].steps = vec![attack_step(-1, &["1d4"], DamageType::Piercing)];
        let r = run(
            &src,
            "longsword",
            &[tgt("t", 5)],
            RollMode::Normal,
            &[10, 2],
        )
        .unwrap();
        let a = r.outcomes[0].roll.as_ref().unwrap();
        assert_eq!((a.notation.as_str(), a.total), ("1d20-1", 9));
        src.abilities[0].steps = vec![attack_step(0, &["1d4"], DamageType::Piercing)];
        let r = run(
            &src,
            "longsword",
            &[tgt("t", 5)],
            RollMode::Normal,
            &[10, 2],
        )
        .unwrap();
        let a = r.outcomes[0].roll.as_ref().unwrap();
        assert_eq!((a.notation.as_str(), a.total), ("1d20+0", 10));
    }

    #[test]
    fn several_damage_parts_each_get_a_roll() {
        let mut src = brannoch();
        src.abilities[0].steps = vec![attack_step(5, &["1d8+3", "1d6"], DamageType::Slashing)];
        let r = run(
            &src,
            "longsword",
            &[tgt("t", 10)],
            RollMode::Normal,
            &[10, 2, 3],
        )
        .unwrap();
        assert_eq!(r.outcomes[0].damage_rolls.len(), 2);
        assert_eq!(r.outcomes[0].damage_rolls[1].notation, "1d6");
    }

    #[test]
    fn one_step_several_targets_gives_one_outcome_each_in_order() {
        let src = brannoch();
        let targets = [tgt("a", 10), tgt("b", 30), tgt("c", 10)];
        // a: hit (d20 10, dmg 4); b: miss (d20 10 vs 30); c: hit (d20 12, dmg 1).
        let r = run(
            &src,
            "longsword",
            &targets,
            RollMode::Normal,
            &[10, 4, 10, 12, 1],
        )
        .unwrap();
        let ids: Vec<_> = r.outcomes.iter().map(|o| o.token_id.as_str()).collect();
        assert_eq!(ids, ["a", "b", "c"]);
        let kinds: Vec<_> = r.outcomes.iter().map(out_kind).collect();
        assert_eq!(kinds, [HitOutcome::Hit, HitOutcome::Miss, HitOutcome::Hit]);
        assert!(r.outcomes.iter().all(|o| o.step_index == 0));
    }

    #[test]
    fn brannoch_longsword_against_a_monster() {
        // A goblin-ish monster: AC 15. Roll 11 + 5 = 16 hits; 1d8 shows 6.
        let r = run(
            &brannoch(),
            "longsword",
            &[tgt("goblin", 15)],
            RollMode::Normal,
            &[11, 6],
        )
        .unwrap();
        let o = &r.outcomes[0];
        assert_eq!(out_kind(o), HitOutcome::Hit);
        assert_eq!(o.roll.as_ref().unwrap().total, 16);
        assert_eq!(o.damage_rolls[0].total, 9);
    }

    #[test]
    fn scorching_ray_three_rays_at_one_target() {
        let src = ilvaren();
        let t = tgt("ogre", 11);
        let targets = [t.clone(), t.clone(), t];
        // ray 0 hits (10+5=15, 2d6: 3,4); ray 1 misses (1); ray 2 crit (20, 4d6).
        let faces = [10, 3, 4, 1, 20, 1, 2, 3, 4];
        let r = run(&src, "scorching-ray", &targets, RollMode::Normal, &faces).unwrap();
        assert_eq!(r.outcomes.len(), 3);
        let idx: Vec<_> = r.outcomes.iter().map(|o| o.step_index).collect();
        assert_eq!(idx, [0, 1, 2]);
        assert!(r.outcomes.iter().all(|o| o.token_id == "ogre"));
        let kinds: Vec<_> = r.outcomes.iter().map(out_kind).collect();
        assert_eq!(
            kinds,
            [HitOutcome::Hit, HitOutcome::Miss, HitOutcome::CriticalHit]
        );
        assert_eq!(r.outcomes[0].damage_rolls[0].total, 7);
        assert!(r.outcomes[1].damage_rolls.is_empty());
        assert_eq!(r.outcomes[2].damage_rolls[0].notation, "4d6");
        assert_eq!(r.outcomes[2].damage_rolls[0].total, 10);
    }

    #[test]
    fn scorching_ray_three_rays_at_three_targets() {
        let src = ilvaren();
        let targets = [tgt("a", 10), tgt("b", 10), tgt("c", 10)];
        let faces = [10, 1, 1, 10, 1, 1, 10, 1, 1];
        let r = run(&src, "scorching-ray", &targets, RollMode::Normal, &faces).unwrap();
        let ids: Vec<_> = r.outcomes.iter().map(|o| o.token_id.as_str()).collect();
        assert_eq!(ids, ["a", "b", "c"]);
        let idx: Vec<_> = r.outcomes.iter().map(|o| o.step_index).collect();
        assert_eq!(idx, [0, 1, 2]);
    }

    #[test]
    fn multi_step_extra_entries_ignored_and_missing_entries_not_rolled() {
        let src = ilvaren();
        // Five entries, three steps: the last two are ignored.
        let five: Vec<_> = (0..5).map(|i| tgt(&format!("t{i}"), 30)).collect();
        let r = run(&src, "scorching-ray", &five, RollMode::Normal, &[5, 5, 5]).unwrap();
        assert_eq!(r.outcomes.len(), 3);
        assert_eq!(codes(&r), ["extra_targets_ignored"]);
        // Two entries, three steps: step 2 is not rolled (script has 2 draws).
        let two = [tgt("x", 30), tgt("y", 30)];
        let r = run(&src, "scorching-ray", &two, RollMode::Normal, &[5, 5]).unwrap();
        let idx: Vec<_> = r.outcomes.iter().map(|o| o.step_index).collect();
        assert_eq!(idx, [0, 1]);
        // No entries at all.
        let r = run(&src, "scorching-ray", &[], RollMode::Normal, &[]).unwrap();
        assert!(r.outcomes.is_empty());
    }

    #[test]
    fn no_steps_is_done_with_no_outcomes() {
        let mut src = brannoch();
        src.abilities[0].steps.clear();
        let r = run(&src, "longsword", &[tgt("t", 10)], RollMode::Normal, &[]).unwrap();
        assert_eq!(r.state, ResolutionState::Done as i32);
        assert!(r.outcomes.is_empty());
        assert_eq!(r.ability_name, "Longsword");
        // Zero steps is not "several attack steps": no extra_targets_ignored.
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);
    }

    #[test]
    fn unknown_ability_is_a_typed_error() {
        let e = run(&brannoch(), "fireball", &[], RollMode::Normal, &[]).unwrap_err();
        assert_eq!(e, AttackError::UnknownAbility);
        assert_eq!(e.code(), "unknown_ability");
    }

    #[test]
    fn non_attack_steps_are_not_implemented() {
        let save = EffectStep {
            step: Some(effect_step::Step::Save(SaveStep::default())),
        };
        let flat = EffectStep {
            step: Some(effect_step::Step::Flat(FlatStep::default())),
        };
        let apply = EffectStep {
            step: Some(effect_step::Step::ApplyEffect(ApplyEffectStep::default())),
        };
        let plugin = EffectStep {
            step: Some(effect_step::Step::Plugin(PluginStep::default())),
        };
        let empty = EffectStep { step: None };
        for odd in [save, flat, apply, plugin, empty] {
            // Even after a valid attack step, and even with no targets.
            let mut src = brannoch();
            src.abilities[0].steps = vec![attack_step(5, &["1d8"], DamageType::Slashing), odd];
            let e = run_old(&src, "longsword", &[], RollMode::Normal, &[]).unwrap_err();
            assert_eq!(e, AttackError::NotImplemented);
            assert_eq!(e.code(), "not_implemented");
        }
    }

    #[test]
    fn a_lone_non_attack_step_is_not_implemented() {
        let apply = EffectStep {
            step: Some(effect_step::Step::ApplyEffect(ApplyEffectStep::default())),
        };
        let plugin = EffectStep {
            step: Some(effect_step::Step::Plugin(PluginStep::default())),
        };
        for odd in [apply, plugin] {
            let mut src = brannoch();
            src.abilities[0].steps = vec![odd];
            let e = run_old(&src, "longsword", &[], RollMode::Normal, &[]).unwrap_err();
            assert_eq!(e, AttackError::NotImplemented);
        }
    }

    #[test]
    fn bad_dice_text_carries_the_dice_code_before_any_roll() {
        let mut src = brannoch();
        src.abilities[0].steps = vec![attack_step(5, &["1d7"], DamageType::Slashing)];
        let (e, calls) = dual(
            &src,
            "longsword",
            &[tgt("t", 10)],
            &grid(),
            RollMode::Normal,
            &[],
        );
        let e = e.unwrap_err();
        assert_eq!(e, AttackError::Dice(ParseError::BadSides));
        assert_eq!(e.code(), "dice_bad_sides");
        assert_eq!(calls, 0);

        src.abilities[0].steps = vec![attack_step(5, &[""], DamageType::Slashing)];
        let e = run(&src, "longsword", &[], RollMode::Normal, &[]).unwrap_err();
        assert_eq!(e.code(), "dice_empty");
    }

    #[test]
    fn crit_that_cannot_double_is_an_error() {
        let mut src = brannoch();
        src.abilities[0].steps = vec![attack_step(5, &["51d6"], DamageType::Slashing)];
        let e = run(&src, "longsword", &[], RollMode::Normal, &[]).unwrap_err();
        assert_eq!(e.code(), "dice_too_many_dice");
    }

    #[test]
    fn no_creature_is_modified_and_hp_stay_empty() {
        let src = brannoch();
        let before = src.clone();
        let targets = [tgt("t", 5)];
        let target_before = targets[0].1.clone();
        let r = run(&src, "longsword", &targets, RollMode::Normal, &[15, 3]).unwrap();
        assert_eq!(src, before);
        assert_eq!(targets[0].1, target_before);
        assert!(r.outcomes[0].damage.is_empty());
        assert!(!r.outcomes[0].dropped_to_zero);
    }

    #[test]
    fn error_display_and_codes_are_stable() {
        assert_eq!(AttackError::UnknownAbility.code(), "unknown_ability");
        assert_eq!(AttackError::NotImplemented.code(), "not_implemented");
        assert_eq!(AttackError::NoTargets.code(), "no_targets");
        assert_eq!(AttackError::Dice(ParseError::Empty).code(), "dice_empty");
        assert!(!AttackError::NotImplemented.to_string().is_empty());
        assert!(!AttackError::NoTargets.to_string().is_empty());
        let _: &dyn std::error::Error = &AttackError::UnknownAbility;
    }

    // ---------------------------------------------------------------
    // Targeting checks (issue 61)
    // ---------------------------------------------------------------

    #[test]
    fn longbow_at_normal_range_has_no_warning_and_normal_mode() {
        // 30 cells = 150 ft, exactly the normal range. Miss: one d20.
        let r = run_full(
            &korvath(),
            "longbow",
            &[tgt_at("t", 30, 30.0)],
            RollMode::Normal,
            &[10],
        )
        .unwrap();
        assert!(r.resolution.warnings.is_empty());
        assert_eq!(roll_mode(&r.resolution.outcomes[0]), RollMode::Normal);
    }

    #[test]
    fn longbow_at_long_range_attacks_with_disadvantage() {
        // 155 ft: past 150, within 600. Disadvantage draws two d20s.
        let r = run(
            &korvath(),
            "longbow",
            &[tgt_at("t", 30, 31.0)],
            RollMode::Normal,
            &[15, 3],
        )
        .unwrap();
        assert!(r.warnings.is_empty());
        let a = r.outcomes[0].roll.as_ref().unwrap();
        assert_eq!(a.mode, RollMode::Disadvantage as i32);
        assert_eq!(a.dice.len(), 2);
        assert_eq!(a.total, 10);
        // 600 ft exactly is still long range.
        let r = run(
            &korvath(),
            "longbow",
            &[tgt_at("t", 30, 120.0)],
            RollMode::Normal,
            &[15, 3],
        )
        .unwrap();
        assert!(r.warnings.is_empty());
        assert_eq!(roll_mode(&r.outcomes[0]), RollMode::Disadvantage);
    }

    #[test]
    fn longbow_beyond_long_range_warns_and_still_rolls() {
        // 605 ft.
        let r = run(
            &korvath(),
            "longbow",
            &[tgt_at("far", 30, 121.0)],
            RollMode::Normal,
            &[10],
        )
        .unwrap();
        assert_eq!(codes(&r), ["out_of_range"]);
        let w = &r.warnings[0];
        assert!(w.message.contains("far"), "{}", w.message);
        assert!(w.message.contains("605"), "{}", w.message);
        // Still rolled, with the mode it would otherwise have.
        assert_eq!(r.outcomes.len(), 1);
        assert_eq!(roll_mode(&r.outcomes[0]), RollMode::Normal);
        // Advantage stays advantage out there.
        let r = run(
            &korvath(),
            "longbow",
            &[tgt_at("far", 30, 121.0)],
            RollMode::Advantage,
            &[10, 4],
        )
        .unwrap();
        assert_eq!(codes(&r), ["out_of_range"]);
        assert_eq!(roll_mode(&r.outcomes[0]), RollMode::Advantage);
    }

    #[test]
    fn advantage_cancels_long_range_disadvantage() {
        // One d20 drawn: advantage and disadvantage cancel to Normal.
        let r = run(
            &korvath(),
            "longbow",
            &[tgt_at("t", 30, 40.0)],
            RollMode::Advantage,
            &[10],
        )
        .unwrap();
        assert!(r.warnings.is_empty());
        let a = r.outcomes[0].roll.as_ref().unwrap();
        assert_eq!(a.mode, RollMode::Normal as i32);
        assert_eq!(a.dice.len(), 1);
    }

    #[test]
    fn disadvantage_mode_stays_disadvantage_at_long_range() {
        let r = run(
            &korvath(),
            "longbow",
            &[tgt_at("t", 30, 40.0)],
            RollMode::Disadvantage,
            &[10, 12],
        )
        .unwrap();
        assert_eq!(roll_mode(&r.outcomes[0]), RollMode::Disadvantage);
        assert_eq!(r.outcomes[0].roll.as_ref().unwrap().dice.len(), 2);
    }

    #[test]
    fn fire_bolt_at_exactly_120_feet_is_fine_and_beyond_warns() {
        let src = ilvaren_fire_bolt();
        let r = run(
            &src,
            "cantrip-fire-bolt",
            &[tgt_at("t", 30, 24.0)],
            RollMode::Normal,
            &[3],
        )
        .unwrap();
        assert!(r.warnings.is_empty());
        assert_eq!(roll_mode(&r.outcomes[0]), RollMode::Normal);
        // 125 ft: no long range, so out_of_range, still rolled normally.
        let r = run(
            &src,
            "cantrip-fire-bolt",
            &[tgt_at("t", 30, 25.0)],
            RollMode::Normal,
            &[3],
        )
        .unwrap();
        assert_eq!(codes(&r), ["out_of_range"]);
        assert_eq!(r.outcomes.len(), 1);
        assert_eq!(roll_mode(&r.outcomes[0]), RollMode::Normal);
    }

    #[test]
    fn touch_range_counts_as_five_feet() {
        let mut src = brannoch();
        src.abilities[0].targeting = Some(targeting(TargetShape::Creatures, 0.0, 0.0, 1));
        let r = run(
            &src,
            "longsword",
            &[tgt_at("t", 30, 1.0)],
            RollMode::Normal,
            &[10],
        )
        .unwrap();
        assert!(r.warnings.is_empty());
        let r = run(
            &src,
            "longsword",
            &[tgt_at("t", 30, 2.0)],
            RollMode::Normal,
            &[10],
        )
        .unwrap();
        assert_eq!(codes(&r), ["out_of_range"]);
    }

    #[test]
    fn range_is_checked_per_target() {
        // Longbow with two targets (max 1): near and long range.
        let near = tgt_at("near", 30, 10.0);
        let long = tgt_at("long", 30, 40.0);
        // near: one d20; long: two d20s.
        let r = run(
            &korvath(),
            "longbow",
            &[near, long],
            RollMode::Normal,
            &[10, 10, 10],
        )
        .unwrap();
        assert_eq!(codes(&r), ["too_many_targets"]);
        assert_eq!(roll_mode(&r.outcomes[0]), RollMode::Normal);
        assert_eq!(roll_mode(&r.outcomes[1]), RollMode::Disadvantage);
    }

    #[test]
    fn each_out_of_range_target_gets_its_own_warning() {
        let src = ilvaren_with_slots(2);
        let targets = [
            tgt_at("tok-alpha", 30, 30.0),
            tgt_at("tok-beta", 30, 2.0),
            tgt_at("tok-gamma", 30, 40.0),
        ];
        let r = run(
            &src,
            "scorching-ray",
            &targets,
            RollMode::Normal,
            &[5, 5, 5],
        )
        .unwrap();
        assert_eq!(codes(&r), ["out_of_range", "out_of_range"]);
        let (w0, w1) = (&r.warnings[0].message, &r.warnings[1].message);
        assert!(w0.contains("tok-alpha") && w0.contains("150"), "{w0}");
        assert!(
            !w0.contains("tok-gamma") && !w0.contains("tok-beta"),
            "{w0}"
        );
        assert!(w1.contains("tok-gamma") && w1.contains("200"), "{w1}");
        assert!(
            !w1.contains("tok-alpha") && !w1.contains("tok-beta"),
            "{w1}"
        );
        assert_eq!(r.outcomes.len(), 3);
    }

    #[test]
    fn longsword_with_two_targets_resolves_both_and_warns_once() {
        let src = brannoch_longsword();
        // a: miss; b: miss.
        let r = run(
            &src,
            "longsword",
            &[tgt_at("a", 30, 1.0), tgt_at("b", 30, 1.0)],
            RollMode::Normal,
            &[10, 10],
        )
        .unwrap();
        assert_eq!(r.outcomes.len(), 2);
        assert_eq!(codes(&r), ["too_many_targets"]);
        // Three targets still give one warning.
        let r = run(
            &src,
            "longsword",
            &[
                tgt_at("a", 30, 1.0),
                tgt_at("b", 30, 1.0),
                tgt_at("c", 30, 1.0),
            ],
            RollMode::Normal,
            &[10, 10, 10],
        )
        .unwrap();
        assert_eq!(r.outcomes.len(), 3);
        assert_eq!(codes(&r), ["too_many_targets"]);
    }

    #[test]
    fn max_targets_zero_means_no_limit() {
        let mut src = brannoch();
        src.abilities[0].targeting = Some(targeting(TargetShape::Creatures, 5.0, 0.0, 0));
        let r = run(
            &src,
            "longsword",
            &[tgt_at("a", 30, 1.0), tgt_at("b", 30, 1.0)],
            RollMode::Normal,
            &[10, 10],
        )
        .unwrap();
        assert!(r.warnings.is_empty());
    }

    #[test]
    fn scorching_ray_with_four_targets_ignores_the_extra() {
        let src = ilvaren_with_slots(2);
        let targets: Vec<_> = (0..4).map(|i| tgt_at(&format!("t{i}"), 30, 3.0)).collect();
        let r = run(
            &src,
            "scorching-ray",
            &targets,
            RollMode::Normal,
            &[5, 5, 5],
        )
        .unwrap();
        assert_eq!(r.outcomes.len(), 3);
        assert_eq!(codes(&r), ["extra_targets_ignored"]);
        let ids: Vec<_> = r.outcomes.iter().map(|o| o.token_id.as_str()).collect();
        assert_eq!(ids, ["t0", "t1", "t2"]);
    }

    #[test]
    fn extra_targets_ignored_is_given_once_for_many_extras() {
        let src = ilvaren_with_slots(2);
        let targets: Vec<_> = (0..6).map(|i| tgt_at(&format!("t{i}"), 30, 3.0)).collect();
        let r = run(
            &src,
            "scorching-ray",
            &targets,
            RollMode::Normal,
            &[5, 5, 5],
        )
        .unwrap();
        assert_eq!(codes(&r), ["extra_targets_ignored"]);
    }

    #[test]
    fn scorching_ray_with_one_target_rolls_one_ray() {
        let src = ilvaren_with_slots(2);
        let r = run(
            &src,
            "scorching-ray",
            &[tgt_at("t", 30, 3.0)],
            RollMode::Normal,
            &[5],
        )
        .unwrap();
        assert_eq!(r.outcomes.len(), 1);
        assert_eq!(r.outcomes[0].step_index, 0);
        assert!(r.warnings.is_empty());
    }

    #[test]
    fn spell_slot_is_spent_once_per_use() {
        let src = ilvaren_with_slots(2);
        let targets = [
            tgt_at("a", 30, 3.0),
            tgt_at("b", 30, 3.0),
            tgt_at("c", 30, 3.0),
        ];
        let r = run_full(
            &src,
            "scorching-ray",
            &targets,
            RollMode::Normal,
            &[5, 5, 5],
        )
        .unwrap();
        assert!(r.resolution.warnings.is_empty());
        let slot = |c: &Creature, k: &str| c.resources.iter().find(|x| x.key == k).unwrap().current;
        assert_eq!(slot(&r.source, "spell_slot_2"), 1);
        assert_eq!(slot(&r.source, "spell_slot_1"), 4);
        // The input is untouched.
        assert_eq!(slot(&src, "spell_slot_2"), 2);
        // Spending the last one leaves 0 and no warning.
        let r = run_full(
            &r.source,
            "scorching-ray",
            &[tgt_at("a", 30, 3.0)],
            RollMode::Normal,
            &[5],
        )
        .unwrap();
        assert_eq!(slot(&r.source, "spell_slot_2"), 0);
        assert!(r.resolution.warnings.is_empty());
    }

    #[test]
    fn empty_slot_warns_floors_at_zero_and_still_resolves() {
        let src = ilvaren_with_slots(0);
        let r = run_full(
            &src,
            "scorching-ray",
            &[tgt_at("a", 30, 3.0)],
            RollMode::Normal,
            &[5],
        )
        .unwrap();
        assert_eq!(codes(&r.resolution), ["not_enough_resource"]);
        assert_eq!(r.resolution.outcomes.len(), 1);
        let cur = r
            .source
            .resources
            .iter()
            .find(|x| x.key == "spell_slot_2")
            .unwrap()
            .current;
        assert_eq!(cur, 0);
    }

    #[test]
    fn missing_slot_warns_once_and_still_resolves() {
        let mut src = ilvaren_with_slots(2);
        src.resources.retain(|r| r.key != "spell_slot_2");
        let before = src.clone();
        let targets = [
            tgt_at("a", 30, 3.0),
            tgt_at("b", 30, 3.0),
            tgt_at("c", 30, 3.0),
        ];
        let r = run_full(
            &src,
            "scorching-ray",
            &targets,
            RollMode::Normal,
            &[5, 5, 5],
        )
        .unwrap();
        assert_eq!(codes(&r.resolution), ["not_enough_resource"]);
        assert_eq!(r.resolution.outcomes.len(), 3);
        assert_eq!(r.source, before);
    }

    #[test]
    fn amount_larger_than_current_warns_and_floors() {
        let mut src = ilvaren_with_slots(1);
        src.abilities[0].resource_amount = 3;
        let r = run_full(
            &src,
            "scorching-ray",
            &[tgt_at("a", 30, 3.0)],
            RollMode::Normal,
            &[5],
        )
        .unwrap();
        assert_eq!(codes(&r.resolution), ["not_enough_resource"]);
        let cur = r
            .source
            .resources
            .iter()
            .find(|x| x.key == "spell_slot_2")
            .unwrap()
            .current;
        assert_eq!(cur, 0);
    }

    #[test]
    fn warnings_come_in_a_fixed_order() {
        let mut src = ilvaren_with_slots(0);
        src.abilities[0].targeting = Some(targeting(TargetShape::Creatures, 120.0, 0.0, 3));
        let targets: Vec<_> = (0..4).map(|i| tgt_at(&format!("t{i}"), 30, 40.0)).collect();
        let r = run(
            &src,
            "scorching-ray",
            &targets,
            RollMode::Normal,
            &[5, 5, 5],
        )
        .unwrap();
        assert_eq!(
            codes(&r),
            [
                "not_enough_resource",
                "extra_targets_ignored",
                "out_of_range",
                "out_of_range",
                "out_of_range"
            ]
        );
    }

    #[test]
    fn creatures_shape_with_no_targets_is_an_error_and_spends_nothing() {
        let src = ilvaren_with_slots(2);
        let (e, calls) = dual(&src, "scorching-ray", &[], &grid(), RollMode::Normal, &[]);
        let e = e.unwrap_err();
        assert_eq!(e, AttackError::NoTargets);
        assert_eq!(e.code(), "no_targets");
        assert_eq!(calls, 0);
    }

    #[test]
    fn area_shapes_are_not_implemented() {
        for shape in [
            TargetShape::Sphere,
            TargetShape::Cone,
            TargetShape::Line,
            TargetShape::Cube,
        ] {
            let mut src = ilvaren_with_slots(2);
            src.abilities[0].targeting = Some(targeting(shape, 60.0, 0.0, 0));
            let e = resolve_old_full(
                &src,
                "scorching-ray",
                &[tgt_at("a", 30, 3.0)],
                RollMode::Normal,
                &[],
            )
            .unwrap_err();
            assert_eq!(e, AttackError::NotImplemented);
            // Also with no targets: the shape error comes first.
            let e =
                resolve_old_full(&src, "scorching-ray", &[], RollMode::Normal, &[]).unwrap_err();
            assert_eq!(e.code(), "not_implemented");
        }
    }

    #[test]
    fn unset_and_self_targeting_skip_range_and_count_checks() {
        // No Targeting at all (the issue-60 abilities): far away, many targets.
        let src = brannoch();
        let r = run(
            &src,
            "longsword",
            &[tgt_at("a", 30, 100.0), tgt_at("b", 30, 100.0)],
            RollMode::Normal,
            &[10, 10],
        )
        .unwrap();
        assert!(r.warnings.is_empty());
        assert_eq!(roll_mode(&r.outcomes[0]), RollMode::Normal);
        // Self: the same, and no targets is allowed.
        let mut src = brannoch();
        src.abilities[0].targeting = Some(targeting(TargetShape::Self_, 0.0, 0.0, 0));
        let r = run(&src, "longsword", &[], RollMode::Normal, &[]).unwrap();
        assert!(r.outcomes.is_empty());
        let r = run(
            &src,
            "longsword",
            &[tgt_at("a", 30, 100.0), tgt_at("b", 30, 100.0)],
            RollMode::Normal,
            &[10, 10],
        )
        .unwrap();
        assert!(r.warnings.is_empty());
    }

    #[test]
    fn unspecified_shape_is_checked_like_creatures_but_allows_no_targets() {
        let mut src = brannoch();
        src.abilities[0].targeting = Some(targeting(TargetShape::Unspecified, 5.0, 0.0, 1));
        let r = run(&src, "longsword", &[], RollMode::Normal, &[]).unwrap();
        assert!(r.outcomes.is_empty());
        let r = run(
            &src,
            "longsword",
            &[tgt_at("a", 30, 3.0), tgt_at("b", 30, 1.0)],
            RollMode::Normal,
            &[10, 10],
        )
        .unwrap();
        assert_eq!(codes(&r), ["too_many_targets", "out_of_range"]);
    }

    #[test]
    fn range_is_not_checked_on_a_non_square_grid_or_zero_cell_size() {
        let src = korvath();
        let far = [tgt_at("far", 30, 500.0)];
        for g in [
            Grid {
                kind: GridKind::HexRows as i32,
                ..grid()
            },
            Grid {
                kind: GridKind::None as i32,
                ..grid()
            },
            Grid {
                cell_size: 0.0,
                ..grid()
            },
            Grid {
                feet_per_cell: 0.0,
                ..grid()
            },
        ] {
            let r = dual(&src, "longbow", &far, &g, RollMode::Normal, &[10])
                .0
                .unwrap();
            assert!(r.resolution.warnings.is_empty());
            assert_eq!(roll_mode(&r.resolution.outcomes[0]), RollMode::Normal);
        }
    }

    #[test]
    fn validation_errors_leave_the_rng_and_resources_alone() {
        let mut src = ilvaren_with_slots(2);
        src.abilities[0].steps[1] = attack_step(5, &["1d7"], DamageType::Fire);
        let (e, calls) = dual(
            &src,
            "scorching-ray",
            &[tgt_at("a", 30, 3.0)],
            &grid(),
            RollMode::Normal,
            &[],
        );
        let e = e.unwrap_err();
        assert_eq!(e.code(), "dice_bad_sides");
        assert_eq!(calls, 0);
    }

    #[test]
    fn resolution_source_token_id_comes_from_the_token() {
        let r = run(
            &brannoch(),
            "longsword",
            &[tgt("t", 15)],
            RollMode::Normal,
            &[10, 5],
        )
        .unwrap();
        assert_eq!(r.source_token_id, "tok-src");
    }
}
