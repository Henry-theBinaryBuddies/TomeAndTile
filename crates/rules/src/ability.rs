//! Ability resolution: attack, save and flat steps (ADR 0005, ADR 0013).
//!
//! [`resolve_ability`] is the general resolver. It takes the same inputs as
//! [`resolve_attacks`](crate::attack::resolve_attacks) (which stays as it was
//! for `crates/server`) and also resolves save and flat steps. Pure: no I/O,
//! no clock, no id minting; randomness is an injected [`Rng`]. `roll_id` and
//! `resolution_id` are left empty for the server to fill. Conditions are
//! #99's job.
//!
//! # Output and applying hit points
//!
//! The result is an [`AbilityResolution`]: the wire [`Resolution`], the
//! updated source creature, the updated target tokens, and
//! [`AbilityResolution::pending`], a plain Rust list with exactly one
//! [`Pending`] per `Resolution.outcomes` entry, in the same order. Each
//! [`Pending`] records what that outcome asked for: the damage parts (type,
//! the roll's total, and whether a successful save halved it) and the
//! healing. [`crate::damage`] then applies them. `pending` is kept for the
//! caller (a result card or log); the applied numbers are in the outcomes.
//! Rules for `pending`:
//!
//! - save step: a failed save has every part, not halved; a successful save
//!   has each part with `halved` set when `half_on_success`, and no parts
//!   otherwise;
//! - flat step: every part, not halved, and `healing` is the healing total;
//! - attack step: every part rolled (already doubled on a critical) on a hit
//!   or critical hit, nothing on a miss;
//! - `rolled` is the roll's total as rolled, which can be negative.
//!
//! Outcomes are applied in order, each to its target's token: damage first
//! (resistance, immunity, vulnerability, temporary HP, floor at 0; see
//! [`crate::damage`]), then healing. The outcome's `damage`, `healing` (the
//! amount actually restored) and `dropped_to_zero` are filled in; the last is
//! true only for the outcome that took the token from above 0 to 0, so of
//! three hits on one token only the one that reaches 0 has it.
//! [`AbilityResolution::targets`] has one updated [`Token`] per token that
//! changed, in the order each was first touched by an outcome. A token that
//! is both source and target (Second Wind) appears once with every change.
//! Tokens are matched by `token_id`; if the same id is listed twice, the
//! first listed copy is the one updated. The target's creature (for
//! resistances) is the one it was listed with. A target token with no
//! hit points gets nothing applied and one `no_hit_points` warning, however
//! many outcomes touch it, but its outcomes still show `applied` amounts.
//! Creature hit points are never changed. No death-save, instant-death or
//! massive-damage rule runs (ADR 0005).
//!
//! # Targets per step
//!
//! - An attack step: with one attack step every listed target; with several,
//!   entry `n` is the target of the `n`th attack step (extra entries are
//!   ignored, steps past the last entry are not rolled), as in
//!   [`attack`](crate::attack).
//! - Every other step (save, flat, apply-effect) uses every listed target.
//! - A `TARGET_SHAPE_SELF` ability, or one with no `Targeting`, that has no
//!   attack step targets the source token (and its creature) and ignores the
//!   listed targets. With an attack step it behaves as in M2.
//! - An area shape (`SPHERE`, `CONE`, `LINE`, `CUBE`): the listed tokens are
//!   the targets; range and `max_targets` are not checked.
//!   `TARGET_SHAPE_CREATURES` with no targets is [`ResolveError::NoTargets`].
//! - `TARGET_SHAPE_UNSPECIFIED` is checked like `CREATURES` but may have no
//!   targets.
//!
//! Outcomes come step by step in step order, and within a step in target
//! order. A save or flat step with no targets (an area with none listed)
//! rolls nothing.
//!
//! # Steps
//!
//! - Save: per target a d20 (always `Normal`; the roll mode is for attacks)
//!   plus the target's ability modifier (`(score - 10)` divided by 2, rounded
//!   down; a missing `scores` counts as 10s) plus `proficiency_bonus` when
//!   the ability is in `save_proficiencies`. The `RollResult` has purpose
//!   `SAVE`, label `<Ability> save` (e.g. "Dexterity save"), modifiers named
//!   by source (`Dexterity`, `proficiency`) and the target creature as
//!   roller. A total below `dc` is `SAVE_FAILED`, otherwise `SAVE_SUCCEEDED`;
//!   a natural 20 or 1 means nothing. Roll order: every target's save, then
//!   the damage parts once. The damage `RollResult`s (purpose `DAMAGE`) are
//!   rolled once per step, even if every save succeeds and none halves, and a
//!   copy goes in `damage_rolls` of every outcome of that step.
//! - Flat: one `AUTOMATIC` outcome per target. Damage parts are rolled once
//!   and copied to every outcome as above, then `healing_dice` is rolled once
//!   (purpose `HEALING`) and its total, floored at 0, is `TargetOutcome.healing`.
//! - Apply-effect: accepted, no outcome, nothing happens until #99.
//!
//! # Recharge and resources
//!
//! An ability with `recharge` set returns the source with `recharge_spent`
//! true. Resources are spent as in [`attack`](crate::attack).
//!
//! # Warnings
//!
//! In this order: `ability_recharging`, `not_enough_resource`,
//! `area_not_checked`, then `too_many_targets` or `extra_targets_ignored`,
//! then one `out_of_range` per out-of-range target, then one `no_hit_points`
//! per target token without hit points (in first-touched order).
//!
//! | code                    | given when |
//! |-------------------------|------------|
//! | `ability_recharging`    | the ability has `recharge` and `recharge_spent` was already true. It still resolves. |
//! | `not_enough_resource`   | as in [`attack`](crate::attack). Once per use. |
//! | `area_not_checked`      | the shape is `SPHERE`, `CONE`, `LINE` or `CUBE`. Once per use; no template is placed. |
//! | `too_many_targets`      | an ability with at most one attack step lists more targets than a non-zero `max_targets` (not for area shapes, `SELF` or no `Targeting`). |
//! | `extra_targets_ignored` | an ability with several attack steps lists more targets than it has steps. |
//! | `out_of_range`          | a target is beyond range, as in [`attack`](crate::attack); not for area shapes, and once per target however many steps use it. |
//! | `no_hit_points`         | a target token has no `hp_max`; its damage and healing are shown, not applied. Once per token. |
//!
//! All dice text (attack, save damage, flat damage and healing, and each
//! target's save roll) is parsed before anything is spent or rolled, so a
//! bad expression is an error with nothing changed.
//!
//! Errors ([`ResolveError`]): `unknown_ability`, `no_targets`,
//! `not_implemented` (a plugin step, an empty step, a save step with no
//! ability, or several attack steps together with a save or flat step) and
//! the dice codes.

use std::collections::HashSet;
use std::fmt;

use tt_contracts::v1::{
    effect_step, Ability, AbilityScore, AbilityScores, Creature, DamagePart, DamageType, Grid,
    HitOutcome, Modifier, Resolution, ResolutionState, RollMode, RollPurpose, RollResult,
    TargetOutcome, TargetShape, Token,
};

use crate::attack::{
    check_range, prepare_attack, resolve_one, roll_result, spend_resource, warning, Prepared,
};
use crate::damage::{apply_damage, apply_healing, DamageRoll};
use crate::dice::{Expr, ParseError, Rng};

/// Why an ability could not be resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolveError {
    /// The ability id is not on the source creature.
    UnknownAbility,
    /// A plugin step, an empty step, a save step with no ability, or several
    /// attack steps together with a save or flat step.
    NotImplemented,
    /// The ability targets creatures but none were listed.
    NoTargets,
    /// A dice expression was rejected.
    Dice(ParseError),
}

impl ResolveError {
    /// Stable snake_case code: `unknown_ability`, `no_targets`,
    /// `not_implemented`, or the [`ParseError::code`].
    pub fn code(&self) -> &'static str {
        match self {
            ResolveError::UnknownAbility => "unknown_ability",
            ResolveError::NotImplemented => "not_implemented",
            ResolveError::NoTargets => "no_targets",
            ResolveError::Dice(e) => e.code(),
        }
    }
}

impl fmt::Display for ResolveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ResolveError::UnknownAbility => write!(f, "the creature has no such ability"),
            ResolveError::NotImplemented => {
                write!(f, "the ability has a step that is not implemented yet")
            }
            ResolveError::NoTargets => write!(f, "the ability needs at least one target"),
            ResolveError::Dice(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for ResolveError {}

impl From<ParseError> for ResolveError {
    fn from(e: ParseError) -> Self {
        ResolveError::Dice(e)
    }
}

/// What one outcome asked of its target's hit points; see the module docs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    /// Same as the outcome's `token_id`.
    pub token_id: String,
    /// Same as the outcome's `step_index`.
    pub step_index: u32,
    /// Damage parts in step order; empty when the target takes none.
    pub damage: Vec<DamageRoll>,
    /// Hit points the ability offered to restore (before the cap at the
    /// maximum); never negative.
    pub healing: i32,
}

/// The result of [`resolve_ability`].
#[derive(Debug, Clone, PartialEq)]
pub struct AbilityResolution {
    /// Outcomes and warnings, with `state` done.
    pub resolution: Resolution,
    /// The source creature after resources were spent and `recharge_spent`
    /// set. Equal to the input when the ability costs nothing.
    pub source: Creature,
    /// The target tokens after damage and healing: one per token that
    /// changed, in the order outcomes first touched them. The source token
    /// appears here only if it was hit or healed.
    pub targets: Vec<Token>,
    /// One entry per `resolution.outcomes` entry, same order.
    pub pending: Vec<Pending>,
}

type Parts = Vec<(Expr, DamageType)>;

/// One step with all its dice parsed ahead of any rolling.
enum Plan {
    Attack {
        step: Prepared,
        types: Vec<DamageType>,
    },
    Save {
        step_index: u32,
        ability: AbilityScore,
        dc: i32,
        half: bool,
        damage: Parts,
        /// The save roll expression per effective target, in target order.
        rolls: Vec<(Expr, Vec<Modifier>)>,
    },
    Flat {
        step_index: u32,
        damage: Parts,
        healing: Option<Expr>,
    },
    Effect,
}

fn damage_type(raw: i32) -> DamageType {
    DamageType::try_from(raw).unwrap_or(DamageType::Unspecified)
}

fn parse_parts(parts: &[DamagePart]) -> Result<Parts, ParseError> {
    parts
        .iter()
        .map(|p| Ok((Expr::parse(&p.dice)?, damage_type(p.r#type))))
        .collect()
}

fn score_of(scores: Option<&AbilityScores>, ability: AbilityScore) -> i32 {
    let Some(s) = scores else { return 10 };
    match ability {
        AbilityScore::Strength => s.strength,
        AbilityScore::Dexterity => s.dexterity,
        AbilityScore::Constitution => s.constitution,
        AbilityScore::Intelligence => s.intelligence,
        AbilityScore::Wisdom => s.wisdom,
        AbilityScore::Charisma => s.charisma,
        AbilityScore::Unspecified => 10,
    }
}

fn ability_name(ability: AbilityScore) -> &'static str {
    match ability {
        AbilityScore::Strength => "Strength",
        AbilityScore::Dexterity => "Dexterity",
        AbilityScore::Constitution => "Constitution",
        AbilityScore::Intelligence => "Intelligence",
        AbilityScore::Wisdom => "Wisdom",
        AbilityScore::Charisma => "Charisma",
        AbilityScore::Unspecified => "Unspecified",
    }
}

/// The saving throw expression and named modifiers for `target`.
fn save_roll(
    target: &Creature,
    ability: AbilityScore,
) -> Result<(Expr, Vec<Modifier>), ParseError> {
    let score = i64::from(score_of(target.scores.as_ref(), ability));
    let ability_mod = (score - 10).div_euclid(2) as i32;
    let mut modifiers = vec![Modifier {
        source: ability_name(ability).to_string(),
        value: ability_mod,
    }];
    let mut total = ability_mod;
    if target.save_proficiencies.contains(&(ability as i32)) {
        modifiers.push(Modifier {
            source: "proficiency".to_string(),
            value: target.proficiency_bonus,
        });
        total = total.saturating_add(target.proficiency_bonus);
    }
    Ok((Expr::parse(&format!("1d20{total:+}"))?, modifiers))
}

fn pending_for(outcome: &TargetOutcome, damage: Vec<DamageRoll>, healing: i32) -> Pending {
    Pending {
        token_id: outcome.token_id.clone(),
        step_index: outcome.step_index,
        damage,
        healing,
    }
}

fn damage_parts(rolls: &[RollResult], types: &[DamageType], halved: bool) -> Vec<DamageRoll> {
    rolls
        .iter()
        .zip(types)
        .map(|(r, &damage_type)| DamageRoll {
            damage_type,
            rolled: r.total,
            halved,
        })
        .collect()
}

fn full_damage(rolls: &[RollResult], types: &[DamageType]) -> Vec<DamageRoll> {
    damage_parts(rolls, types, false)
}

fn half_damage(rolls: &[RollResult], types: &[DamageType]) -> Vec<DamageRoll> {
    damage_parts(rolls, types, true)
}

fn roll_parts(
    parts: &Parts,
    rng: &mut dyn Rng,
    source: &Creature,
    ability: &Ability,
) -> Vec<RollResult> {
    parts
        .iter()
        .map(|(expr, _)| {
            roll_result(
                expr,
                RollMode::Normal,
                rng,
                RollPurpose::Damage,
                &source.creature_id,
                format!("{} damage", ability.name),
            )
        })
        .collect()
}

/// Resolve `ability_id` on `source`: attack, save, flat and apply-effect
/// steps. See the module docs for the target rules, warnings and the
/// `pending` damage handed to #98.
///
/// Errors are detected before anything is spent or rolled.
pub fn resolve_ability(
    source_token: &Token,
    source: &Creature,
    ability_id: &str,
    targets: &[(Token, Creature)],
    grid: &Grid,
    mode: RollMode,
    rng: &mut dyn Rng,
) -> Result<AbilityResolution, ResolveError> {
    let ability = source
        .abilities
        .iter()
        .find(|a| a.ability_id == ability_id)
        .ok_or(ResolveError::UnknownAbility)?;

    let shape = ability
        .targeting
        .as_ref()
        .map(|t| TargetShape::try_from(t.shape).unwrap_or(TargetShape::Unspecified));
    let area = matches!(
        shape,
        Some(TargetShape::Sphere | TargetShape::Cone | TargetShape::Line | TargetShape::Cube)
    );
    if shape == Some(TargetShape::Creatures) && targets.is_empty() {
        return Err(ResolveError::NoTargets);
    }
    // Range and count checks apply to CREATURES and UNSPECIFIED only.
    let checked = match shape {
        Some(TargetShape::Creatures | TargetShape::Unspecified) => ability.targeting.as_ref(),
        _ => None,
    };

    let attacks = ability
        .steps
        .iter()
        .filter(|s| matches!(s.step, Some(effect_step::Step::Attack(_))))
        .count();
    let multi = attacks > 1;
    let self_targeted = matches!(shape, Some(TargetShape::Self_) | None) && attacks == 0;
    let effective: Vec<(&Token, &Creature)> = if self_targeted {
        vec![(source_token, source)]
    } else {
        targets.iter().map(|(t, c)| (t, c)).collect()
    };

    // Parse everything (dice, per-target save rolls) before anything happens.
    let mut plans = Vec::with_capacity(ability.steps.len());
    for (i, step) in ability.steps.iter().enumerate() {
        let step_index = u32::try_from(i).unwrap_or(u32::MAX);
        let plan = match &step.step {
            Some(effect_step::Step::Attack(a)) => Plan::Attack {
                step: prepare_attack(i, a)?,
                types: a.damage.iter().map(|p| damage_type(p.r#type)).collect(),
            },
            Some(effect_step::Step::Save(s)) => {
                if multi {
                    return Err(ResolveError::NotImplemented);
                }
                let ability_score = AbilityScore::try_from(s.ability)
                    .ok()
                    .filter(|a| *a != AbilityScore::Unspecified)
                    .ok_or(ResolveError::NotImplemented)?;
                let damage = parse_parts(&s.damage)?;
                let rolls = effective
                    .iter()
                    .map(|(_, c)| save_roll(c, ability_score))
                    .collect::<Result<Vec<_>, _>>()?;
                Plan::Save {
                    step_index,
                    ability: ability_score,
                    dc: s.dc,
                    half: s.half_on_success,
                    damage,
                    rolls,
                }
            }
            Some(effect_step::Step::Flat(f)) => {
                if multi {
                    return Err(ResolveError::NotImplemented);
                }
                let healing = if f.healing_dice.is_empty() {
                    None
                } else {
                    Some(Expr::parse(&f.healing_dice)?)
                };
                Plan::Flat {
                    step_index,
                    damage: parse_parts(&f.damage)?,
                    healing,
                }
            }
            Some(effect_step::Step::ApplyEffect(_)) => Plan::Effect,
            Some(effect_step::Step::Plugin(_)) | None => return Err(ResolveError::NotImplemented),
        };
        plans.push(plan);
    }

    let mut warnings = Vec::new();
    let mut updated = source.clone();

    if ability.recharge.is_some() {
        if ability.recharge_spent {
            warnings.push(warning(
                "ability_recharging",
                format!("{} has not recharged yet", ability.name),
            ));
        }
        if let Some(a) = updated
            .abilities
            .iter_mut()
            .find(|a| a.ability_id == ability.ability_id)
        {
            a.recharge_spent = true;
        }
    }
    spend_resource(&mut updated, ability, &mut warnings);

    if area {
        warnings.push(warning(
            "area_not_checked",
            format!(
                "{} is an area ability: the listed tokens are the targets and no template was checked",
                ability.name
            ),
        ));
    }
    if multi {
        if targets.len() > attacks {
            warnings.push(warning(
                "extra_targets_ignored",
                format!(
                    "{} has {} attacks but {} targets were listed; the extra were ignored",
                    ability.name,
                    attacks,
                    targets.len()
                ),
            ));
        }
    } else if !plans.is_empty() {
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
    }

    // Range, once per paired target; the mode only matters to attack steps.
    let paired = if multi {
        attacks.min(effective.len())
    } else {
        effective.len()
    };
    let modes: Vec<RollMode> = effective[..paired]
        .iter()
        .map(|(token, _)| match checked {
            Some(t) => check_range(t, grid, source_token, token, mode, &mut warnings),
            None => mode,
        })
        .collect();

    let mut outcomes = Vec::new();
    let mut pending = Vec::new();
    // Index into `effective` of each outcome's target.
    let mut owners: Vec<usize> = Vec::new();
    let mut attack_ordinal = 0;
    for plan in &plans {
        // With nobody to affect, a save or flat step rolls nothing.
        if effective.is_empty() && matches!(plan, Plan::Save { .. } | Plan::Flat { .. }) {
            continue;
        }
        match plan {
            Plan::Attack { step, types } => {
                let range = if multi {
                    let n = attack_ordinal;
                    attack_ordinal += 1;
                    n..(n + 1).min(paired)
                } else {
                    0..paired
                };
                for n in range {
                    let (token, creature) = effective[n];
                    let o = resolve_one(
                        source,
                        ability,
                        step,
                        &token.token_id,
                        creature,
                        modes[n],
                        rng,
                    );
                    let damage = if o.outcome == HitOutcome::Miss as i32 {
                        Vec::new()
                    } else {
                        full_damage(&o.damage_rolls, types)
                    };
                    pending.push(pending_for(&o, damage, 0));
                    outcomes.push(o);
                    owners.push(n);
                }
            }
            Plan::Save {
                step_index,
                ability: score,
                dc,
                half,
                damage,
                rolls,
            } => {
                let saves: Vec<RollResult> = rolls
                    .iter()
                    .zip(&effective)
                    .map(|((expr, mods), (_, creature))| {
                        let mut r = roll_result(
                            expr,
                            RollMode::Normal,
                            rng,
                            RollPurpose::Save,
                            &creature.creature_id,
                            format!("{} save", ability_name(*score)),
                        );
                        r.modifiers = mods.clone();
                        // A save has no critical or fumble.
                        r.critical = false;
                        r.fumble = false;
                        r
                    })
                    .collect();
                let damage_rolls = roll_parts(damage, rng, source, ability);
                let types: Vec<DamageType> = damage.iter().map(|(_, t)| *t).collect();
                for (n, (save, (token, _))) in saves.into_iter().zip(&effective).enumerate() {
                    let (kind, taken) = if save.total < *dc {
                        (HitOutcome::SaveFailed, full_damage(&damage_rolls, &types))
                    } else if *half {
                        (
                            HitOutcome::SaveSucceeded,
                            half_damage(&damage_rolls, &types),
                        )
                    } else {
                        (HitOutcome::SaveSucceeded, Vec::new())
                    };
                    let o = TargetOutcome {
                        token_id: token.token_id.clone(),
                        outcome: kind as i32,
                        roll: Some(save),
                        damage_rolls: damage_rolls.clone(),
                        damage: Vec::new(),
                        healing: 0,
                        effects_applied: Vec::new(),
                        dropped_to_zero: false,
                        step_index: *step_index,
                    };
                    pending.push(pending_for(&o, taken, 0));
                    outcomes.push(o);
                    owners.push(n);
                }
            }
            Plan::Flat {
                step_index,
                damage,
                healing,
            } => {
                let damage_rolls = roll_parts(damage, rng, source, ability);
                let healed = healing.as_ref().map_or(0, |expr| {
                    roll_result(
                        expr,
                        RollMode::Normal,
                        rng,
                        RollPurpose::Healing,
                        &source.creature_id,
                        format!("{} healing", ability.name),
                    )
                    .total
                    .max(0)
                });
                let types: Vec<DamageType> = damage.iter().map(|(_, t)| *t).collect();
                for (n, (token, _)) in effective.iter().enumerate() {
                    let o = TargetOutcome {
                        token_id: token.token_id.clone(),
                        outcome: HitOutcome::Automatic as i32,
                        roll: None,
                        damage_rolls: damage_rolls.clone(),
                        damage: Vec::new(),
                        healing: healed,
                        effects_applied: Vec::new(),
                        dropped_to_zero: false,
                        step_index: *step_index,
                    };
                    pending.push(pending_for(&o, full_damage(&damage_rolls, &types), healed));
                    outcomes.push(o);
                    owners.push(n);
                }
            }
            Plan::Effect => {}
        }
    }

    // Apply every outcome in order to its target's token.
    let mut touched: Vec<Token> = Vec::new();
    let mut no_hp: Vec<String> = Vec::new();
    for ((o, p), &n) in outcomes.iter_mut().zip(&pending).zip(&owners) {
        let (listed, creature) = effective[n];
        let at = match touched.iter().position(|t| t.token_id == listed.token_id) {
            Some(i) => i,
            None => {
                touched.push(listed.clone());
                touched.len() - 1
            }
        };
        let hit = apply_damage(&touched[at], creature, &p.damage);
        let healed = apply_healing(&hit.token, p.healing);
        o.damage = hit.damage;
        o.dropped_to_zero = hit.dropped_to_zero;
        o.healing = healed.restored;
        touched[at] = healed.token;
        if (hit.no_hit_points || healed.no_hit_points) && !no_hp.contains(&listed.token_id) {
            no_hp.push(listed.token_id.clone());
        }
    }
    for id in no_hp {
        warnings.push(warning(
            "no_hit_points",
            format!("token {id} has no hit points: damage and healing are shown, not applied"),
        ));
    }
    // Only tokens that changed go back, in first-touched order.
    let mut seen = HashSet::new();
    let targets: Vec<Token> = touched
        .into_iter()
        .filter(|t| seen.insert(t.token_id.clone()))
        .filter(|t| {
            effective
                .iter()
                .find(|(l, _)| l.token_id == t.token_id)
                .is_some_and(|(l, _)| *l != t)
        })
        .collect();

    Ok(AbilityResolution {
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
        targets,
        pending,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tt_contracts::v1::{
        ApplyEffectStep, AttackStep, CreatureSize, DiagonalRule, EffectStep, FlatStep, GridKind,
        HealthState, PluginStep, Recharge, Resource, SaveStep, Targeting, Vec2,
    };

    struct Scripted {
        values: Vec<u64>,
        calls: usize,
    }

    impl Scripted {
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

    fn part(dice: &str, ty: DamageType) -> DamagePart {
        DamagePart {
            dice: dice.to_string(),
            r#type: ty as i32,
        }
    }

    fn save_step(ab: AbilityScore, dc: i32, dice: &str, ty: DamageType, half: bool) -> EffectStep {
        EffectStep {
            step: Some(effect_step::Step::Save(SaveStep {
                ability: ab as i32,
                dc,
                damage: vec![part(dice, ty)],
                half_on_success: half,
            })),
        }
    }

    fn flat_step(damage: Vec<DamagePart>, healing: &str) -> EffectStep {
        EffectStep {
            step: Some(effect_step::Step::Flat(FlatStep {
                damage,
                healing_dice: healing.to_string(),
            })),
        }
    }

    fn attack_step(bonus: i32, dice: &str) -> EffectStep {
        EffectStep {
            step: Some(effect_step::Step::Attack(AttackStep {
                to_hit_bonus: bonus,
                damage: vec![part(dice, DamageType::Slashing)],
            })),
        }
    }

    fn effect_only(id: &str) -> EffectStep {
        EffectStep {
            step: Some(effect_step::Step::ApplyEffect(ApplyEffectStep {
                effect_id: id.to_string(),
                ..Default::default()
            })),
        }
    }

    fn targeting(shape: TargetShape, range: f32, max: u32) -> Targeting {
        Targeting {
            shape: shape as i32,
            range_feet: range,
            max_targets: max,
            ..Default::default()
        }
    }

    fn ability(id: &str, name: &str, tg: Option<Targeting>, steps: Vec<EffectStep>) -> Ability {
        Ability {
            ability_id: id.to_string(),
            name: name.to_string(),
            targeting: tg,
            steps,
            ..Default::default()
        }
    }

    fn scores(str_: i32, dex: i32, con: i32) -> AbilityScores {
        AbilityScores {
            strength: str_,
            dexterity: dex,
            constitution: con,
            intelligence: 10,
            wisdom: 10,
            charisma: 10,
        }
    }

    /// Brannoch (fixture): Dex 14, Con 14, proficient in Str and Con saves.
    fn brannoch() -> Creature {
        Creature {
            creature_id: "brannoch".to_string(),
            scores: Some(scores(17, 14, 14)),
            proficiency_bonus: 2,
            save_proficiencies: vec![
                AbilityScore::Strength as i32,
                AbilityScore::Constitution as i32,
            ],
            armor_class: 18,
            abilities: vec![ability(
                "second-wind",
                "Second Wind",
                Some(targeting(TargetShape::Self_, 0.0, 0)),
                vec![flat_step(Vec::new(), "1d10+3")],
            )],
            resources: vec![Resource {
                key: "second_wind".to_string(),
                current: 2,
                max: 2,
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    /// Second Wind needs its resource key, as in the fixture.
    fn brannoch_with_second_wind() -> Creature {
        let mut c = brannoch();
        c.abilities[0].resource_key = "second_wind".to_string();
        c.abilities[0].resource_amount = 1;
        c
    }

    /// Ember Wisp (#95 seed): Dex 14, Con 12, proficient in Dex saves.
    fn ember_wisp() -> Creature {
        Creature {
            creature_id: "creature-ember-wisp".to_string(),
            scores: Some(scores(4, 14, 12)),
            proficiency_bonus: 2,
            save_proficiencies: vec![AbilityScore::Dexterity as i32],
            armor_class: 12,
            abilities: vec![ability(
                "choking-ash",
                "Choking Ash",
                Some(targeting(TargetShape::Creatures, 10.0, 1)),
                vec![
                    save_step(
                        AbilityScore::Constitution,
                        11,
                        "2d4",
                        DamageType::Poison,
                        true,
                    ),
                    effect_only("blinded"),
                ],
            )],
            ..Default::default()
        }
    }

    fn caster(id: &str, abilities: Vec<Ability>) -> Creature {
        Creature {
            creature_id: id.to_string(),
            abilities,
            ..Default::default()
        }
    }

    fn token_at(id: &str, cx: f32) -> Token {
        Token {
            token_id: id.to_string(),
            position: Some(Vec2 {
                x: (cx + 0.5) * 70.0,
                y: 35.0,
            }),
            size: CreatureSize::Medium as i32,
            hp_current: Some(50),
            hp_max: Some(100),
            hp_temp: Some(0),
            health_state: HealthState::Bloodied as i32,
            ..Default::default()
        }
    }

    fn grid() -> Grid {
        Grid {
            kind: GridKind::Square as i32,
            cell_size: 70.0,
            offset: None,
            feet_per_cell: 5.0,
            diagonal_rule: DiagonalRule::Five as i32,
        }
    }

    fn src_token() -> Token {
        token_at("tok-src", 0.0)
    }

    fn at(token: &str, cells: f32, c: Creature) -> (Token, Creature) {
        (token_at(token, cells), c)
    }

    fn run(
        source: &Creature,
        id: &str,
        targets: &[(Token, Creature)],
        faces: &[u64],
    ) -> Result<AbilityResolution, ResolveError> {
        resolve_ability(
            &src_token(),
            source,
            id,
            targets,
            &grid(),
            RollMode::Normal,
            &mut Scripted::faces(faces),
        )
    }

    fn codes(r: &AbilityResolution) -> Vec<&str> {
        r.resolution
            .warnings
            .iter()
            .map(|w| w.code.as_str())
            .collect()
    }

    fn kind(o: &TargetOutcome) -> HitOutcome {
        HitOutcome::try_from(o.outcome).unwrap()
    }

    fn dmg(ty: DamageType, rolled: i32) -> DamageRoll {
        DamageRoll {
            damage_type: ty,
            rolled,
            halved: false,
        }
    }

    fn halved(ty: DamageType, rolled: i32) -> DamageRoll {
        DamageRoll {
            damage_type: ty,
            rolled,
            halved: true,
        }
    }

    /// Maribel's Sacred Flame from the fixture: Dex DC 13, 1d8 radiant.
    fn sacred_flame() -> Creature {
        caster(
            "maribel",
            vec![ability(
                "cantrip-sacred-flame",
                "Sacred Flame",
                Some(targeting(TargetShape::Creatures, 60.0, 1)),
                vec![save_step(
                    AbilityScore::Dexterity,
                    13,
                    "1d8",
                    DamageType::Radiant,
                    false,
                )],
            )],
        )
    }

    #[test]
    fn sacred_flame_failed_save_takes_full_damage() {
        // d20 10 + Dex 2 = 12 < 13; then 1d8 = 5.
        let r = run(
            &sacred_flame(),
            "cantrip-sacred-flame",
            &[at("t1", 3.0, brannoch())],
            &[10, 5],
        )
        .unwrap();
        assert!(r.resolution.warnings.is_empty());
        let o = &r.resolution.outcomes[0];
        assert_eq!(kind(o), HitOutcome::SaveFailed);
        assert_eq!(o.token_id, "t1");
        assert_eq!(o.step_index, 0);
        let s = o.roll.as_ref().unwrap();
        assert_eq!(s.purpose, RollPurpose::Save as i32);
        assert_eq!(s.notation, "1d20+2");
        assert_eq!(s.total, 12);
        assert_eq!(s.label, "Dexterity save");
        assert_eq!(s.roller_creature_id, "brannoch");
        assert_eq!(s.modifiers.len(), 1);
        assert_eq!(s.modifiers[0].source, "Dexterity");
        assert_eq!(s.modifiers[0].value, 2);
        assert!(s.roll_id.is_empty());
        assert_eq!(o.damage_rolls.len(), 1);
        assert_eq!(o.damage_rolls[0].purpose, RollPurpose::Damage as i32);
        assert_eq!(o.damage_rolls[0].total, 5);
        assert_eq!(o.damage_rolls[0].roller_creature_id, "maribel");
        assert_eq!(o.damage.len(), 1);
        assert_eq!((o.damage[0].rolled, o.damage[0].applied), (5, 5));
        assert_eq!(r.targets[0].hp_current, Some(45));
        assert_eq!(
            r.pending,
            vec![Pending {
                token_id: "t1".to_string(),
                step_index: 0,
                damage: vec![dmg(DamageType::Radiant, 5)],
                healing: 0,
            }]
        );
    }

    #[test]
    fn sacred_flame_successful_save_takes_nothing_but_damage_is_still_rolled() {
        // 11 + 2 = 13 meets the DC.
        let r = run(
            &sacred_flame(),
            "cantrip-sacred-flame",
            &[at("t1", 3.0, brannoch())],
            &[11, 5],
        )
        .unwrap();
        let o = &r.resolution.outcomes[0];
        assert_eq!(kind(o), HitOutcome::SaveSucceeded);
        assert_eq!(o.damage_rolls[0].total, 5);
        assert!(r.pending[0].damage.is_empty());
    }

    #[test]
    fn natural_20_and_1_mean_nothing_on_a_save() {
        let mut src = sacred_flame();
        let t = [at("t", 3.0, brannoch())];
        // Natural 1 with +2 is 3: fails because it is below 13, not the 1.
        let r = run(&src, "cantrip-sacred-flame", &t, &[1, 4]).unwrap();
        assert_eq!(kind(&r.resolution.outcomes[0]), HitOutcome::SaveFailed);
        assert!(!r.resolution.outcomes[0].roll.as_ref().unwrap().fumble);
        // Natural 20 against an unreachable DC still fails.
        src.abilities[0].steps = vec![save_step(
            AbilityScore::Dexterity,
            30,
            "1d8",
            DamageType::Radiant,
            false,
        )];
        let r = run(&src, "cantrip-sacred-flame", &t, &[20, 4]).unwrap();
        let o = &r.resolution.outcomes[0];
        assert_eq!(kind(o), HitOutcome::SaveFailed);
        assert!(!o.roll.as_ref().unwrap().critical);
        // Natural 1 still succeeds against a low DC.
        src.abilities[0].steps = vec![save_step(
            AbilityScore::Dexterity,
            3,
            "1d8",
            DamageType::Radiant,
            false,
        )];
        let r = run(&src, "cantrip-sacred-flame", &t, &[1, 4]).unwrap();
        assert_eq!(kind(&r.resolution.outcomes[0]), HitOutcome::SaveSucceeded);
    }

    #[test]
    fn burning_hands_two_targets_share_one_damage_roll() {
        let mut ilvaren = caster(
            "ilvaren",
            vec![ability(
                "burning-hands",
                "Burning Hands",
                Some(Targeting {
                    shape: TargetShape::Cone as i32,
                    size_feet: 15.0,
                    ..Default::default()
                }),
                vec![save_step(
                    AbilityScore::Dexterity,
                    13,
                    "3d6",
                    DamageType::Fire,
                    true,
                )],
            )],
        );
        ilvaren.abilities[0].resource_key = "spell_slot_1".to_string();
        ilvaren.abilities[0].resource_amount = 1;
        ilvaren.resources = vec![Resource {
            key: "spell_slot_1".to_string(),
            current: 2,
            max: 4,
            ..Default::default()
        }];
        // Far away: area shapes check no range. Brannoch: 10 + 2 = 12 fails.
        // Ember Wisp: 10 + 2 + 2 proficiency = 14 succeeds. Damage 6+5+4 = 15.
        let r = run(
            &ilvaren,
            "burning-hands",
            &[at("a", 40.0, brannoch()), at("b", 41.0, ember_wisp())],
            &[10, 10, 6, 5, 4],
        )
        .unwrap();
        assert_eq!(codes(&r), ["area_not_checked"]);
        assert_eq!(r.source.resources[0].current, 1);
        let (a, b) = (&r.resolution.outcomes[0], &r.resolution.outcomes[1]);
        assert_eq!(kind(a), HitOutcome::SaveFailed);
        assert_eq!(kind(b), HitOutcome::SaveSucceeded);
        assert_eq!(a.damage_rolls.len(), 1);
        assert_eq!(a.damage_rolls, b.damage_rolls);
        assert_eq!(a.damage_rolls[0].total, 15);
        assert_eq!(b.roll.as_ref().unwrap().notation, "1d20+4");
        assert_eq!(r.pending[0].damage, vec![dmg(DamageType::Fire, 15)]);
        // Half of 15 rounds down.
        assert_eq!(r.pending[1].damage, vec![halved(DamageType::Fire, 15)]);
        assert_eq!(r.resolution.outcomes[1].damage[0].applied, 7);
    }

    #[test]
    fn choking_ash_against_a_proficient_brannoch_adds_proficiency() {
        let wisp = ember_wisp();
        let t = [at("brannoch-tok", 1.0, brannoch())];
        // 8 + 2 (Con) + 2 (proficiency) = 12 >= 11: success, half of 3+4=7.
        let r = run(&wisp, "choking-ash", &t, &[8, 3, 4]).unwrap();
        assert!(r.resolution.warnings.is_empty());
        // The apply-effect step does nothing and makes no outcome.
        assert_eq!(r.resolution.outcomes.len(), 1);
        let o = &r.resolution.outcomes[0];
        assert_eq!(kind(o), HitOutcome::SaveSucceeded);
        let s = o.roll.as_ref().unwrap();
        assert_eq!(s.notation, "1d20+4");
        assert_eq!(s.total, 12);
        assert_eq!(s.label, "Constitution save");
        let mods: Vec<(&str, i32)> = s
            .modifiers
            .iter()
            .map(|m| (m.source.as_str(), m.value))
            .collect();
        assert_eq!(mods, [("Constitution", 2), ("proficiency", 2)]);
        assert_eq!(r.pending[0].damage, vec![halved(DamageType::Poison, 7)]);
        assert_eq!(r.resolution.outcomes[0].damage[0].applied, 3);
        // 6 + 4 = 10 < 11 fails and takes all 7.
        let r = run(&wisp, "choking-ash", &t, &[6, 3, 4]).unwrap();
        assert_eq!(kind(&r.resolution.outcomes[0]), HitOutcome::SaveFailed);
        assert_eq!(r.pending[0].damage, vec![dmg(DamageType::Poison, 7)]);
    }

    #[test]
    fn ability_modifier_rounds_down_for_low_scores() {
        let mut weak = brannoch();
        weak.scores = Some(scores(9, 14, 14));
        weak.save_proficiencies.clear();
        let mut src = sacred_flame();
        src.abilities[0].steps = vec![save_step(
            AbilityScore::Strength,
            10,
            "1d8",
            DamageType::Radiant,
            false,
        )];
        let r = run(
            &src,
            "cantrip-sacred-flame",
            &[at("w", 1.0, weak)],
            &[10, 1],
        )
        .unwrap();
        let s = r.resolution.outcomes[0].roll.as_ref().unwrap();
        // (9 - 10) / 2 rounded down is -1.
        assert_eq!(s.notation, "1d20-1");
        assert_eq!(s.total, 9);
        assert_eq!(s.modifiers[0].value, -1);
    }

    fn magic_missile() -> Creature {
        caster(
            "ilvaren",
            vec![ability(
                "magic-missile",
                "Magic Missile",
                Some(targeting(TargetShape::Creatures, 120.0, 3)),
                vec![flat_step(vec![part("3d4+3", DamageType::Force)], "")],
            )],
        )
    }

    #[test]
    fn magic_missile_deals_the_full_amount_to_each_listed_target() {
        // 3d4 = 1 + 2 + 3, +3 = 9, rolled once.
        let r = run(
            &magic_missile(),
            "magic-missile",
            &[at("a", 5.0, ember_wisp()), at("b", 6.0, brannoch())],
            &[1, 2, 3],
        )
        .unwrap();
        assert!(r.resolution.warnings.is_empty());
        assert_eq!(r.resolution.outcomes.len(), 2);
        for (o, p) in r.resolution.outcomes.iter().zip(&r.pending) {
            assert_eq!(kind(o), HitOutcome::Automatic);
            assert!(o.roll.is_none());
            assert_eq!(o.healing, 0);
            assert_eq!(o.damage_rolls.len(), 1);
            assert_eq!(o.damage_rolls[0].total, 9);
            assert_eq!(o.damage_rolls[0].notation, "3d4+3");
            assert_eq!(p.damage, vec![dmg(DamageType::Force, 9)]);
            assert_eq!(p.token_id, o.token_id);
        }
    }

    #[test]
    fn cure_wounds_rolls_healing_once_into_every_outcome() {
        let maribel = caster(
            "maribel",
            vec![ability(
                "cure-wounds",
                "Cure Wounds",
                Some(targeting(TargetShape::Creatures, 0.0, 2)),
                vec![flat_step(Vec::new(), "2d8+6")],
            )],
        );
        // 3 + 4 + 6 = 13, shared by both targets.
        let r = run(
            &maribel,
            "cure-wounds",
            &[at("t", 1.0, brannoch()), at("u", 1.0, ember_wisp())],
            &[3, 4],
        )
        .unwrap();
        for (o, p) in r.resolution.outcomes.iter().zip(&r.pending) {
            assert_eq!(kind(o), HitOutcome::Automatic);
            assert_eq!(o.healing, 13);
            assert!(o.damage_rolls.is_empty());
            assert_eq!(p.healing, 13);
            assert!(p.damage.is_empty());
        }
    }

    #[test]
    fn second_wind_self_targets_the_source_and_ignores_listed_targets() {
        let src = brannoch_with_second_wind();
        let r = run(&src, "second-wind", &[], &[4]).unwrap();
        assert!(r.resolution.warnings.is_empty());
        assert_eq!(r.resolution.outcomes.len(), 1);
        let o = &r.resolution.outcomes[0];
        assert_eq!(o.token_id, "tok-src");
        assert_eq!(kind(o), HitOutcome::Automatic);
        assert_eq!(o.healing, 7);
        assert_eq!(r.pending[0].healing, 7);
        assert_eq!(r.source.resources[0].current, 1);
        // Listed targets are ignored, and no range or count warnings appear.
        let r = run(
            &src,
            "second-wind",
            &[at("x", 50.0, ember_wisp()), at("y", 60.0, ember_wisp())],
            &[4],
        )
        .unwrap();
        assert_eq!(r.resolution.outcomes.len(), 1);
        assert_eq!(r.resolution.outcomes[0].token_id, "tok-src");
        assert!(r.resolution.warnings.is_empty());
    }

    #[test]
    fn no_targeting_with_a_save_step_saves_for_the_source() {
        let mut src = sacred_flame();
        src.abilities[0].targeting = None;
        src.scores = Some(scores(10, 18, 10));
        let r = run(
            &src,
            "cantrip-sacred-flame",
            &[at("x", 2.0, brannoch())],
            &[10, 2],
        )
        .unwrap();
        assert_eq!(r.resolution.outcomes.len(), 1);
        let o = &r.resolution.outcomes[0];
        assert_eq!(o.token_id, "tok-src");
        let s = o.roll.as_ref().unwrap();
        assert_eq!(s.notation, "1d20+4");
        assert_eq!(s.roller_creature_id, "maribel");
    }

    #[test]
    fn recharge_ability_used_twice_warns_the_second_time() {
        let mut wisp = ember_wisp();
        wisp.abilities[0].recharge = Some(Recharge { min_roll: 5 });
        let t = [at("t", 1.0, brannoch())];
        let first = run(&wisp, "choking-ash", &t, &[10, 1, 1]).unwrap();
        assert!(first.resolution.warnings.is_empty());
        assert!(first.source.abilities[0].recharge_spent);
        // The input is untouched.
        assert!(!wisp.abilities[0].recharge_spent);
        let second = run(&first.source, "choking-ash", &t, &[10, 1, 1]).unwrap();
        assert_eq!(codes(&second), ["ability_recharging"]);
        assert_eq!(second.resolution.outcomes.len(), 1);
        assert!(second.source.abilities[0].recharge_spent);
    }

    #[test]
    fn several_attacks_with_a_save_or_flat_step_are_not_implemented() {
        for extra in [
            save_step(AbilityScore::Dexterity, 10, "1d6", DamageType::Fire, false),
            flat_step(Vec::new(), "1d4"),
        ] {
            let src = caster(
                "x",
                vec![ability(
                    "multi",
                    "Multi",
                    Some(targeting(TargetShape::Creatures, 5.0, 0)),
                    vec![attack_step(5, "1d8"), attack_step(5, "1d8"), extra],
                )],
            );
            let e = run(&src, "multi", &[at("t", 1.0, brannoch())], &[]).unwrap_err();
            assert_eq!(e, ResolveError::NotImplemented);
            assert_eq!(e.code(), "not_implemented");
        }
    }

    #[test]
    fn plugin_empty_and_unspecified_save_steps_are_not_implemented() {
        let plugin = EffectStep {
            step: Some(effect_step::Step::Plugin(PluginStep::default())),
        };
        let empty = EffectStep { step: None };
        let no_ability = EffectStep {
            step: Some(effect_step::Step::Save(SaveStep::default())),
        };
        for odd in [plugin, empty, no_ability] {
            let mut src = sacred_flame();
            src.abilities[0].steps.push(odd);
            let t = [at("t", 1.0, brannoch())];
            let e = run(&src, "cantrip-sacred-flame", &t, &[]).unwrap_err();
            assert_eq!(e, ResolveError::NotImplemented);
        }
    }

    #[test]
    fn apply_effect_only_abilities_resolve_with_no_outcomes() {
        let mut src = sacred_flame();
        src.abilities[0].steps = vec![effect_only("bless")];
        let t = [at("t", 1.0, brannoch())];
        let r = run(&src, "cantrip-sacred-flame", &t, &[]).unwrap();
        assert!(r.resolution.outcomes.is_empty());
        assert!(r.pending.is_empty());
    }

    #[test]
    fn creatures_with_no_targets_errors_but_area_does_not() {
        let src = sacred_flame();
        let e = run(&src, "cantrip-sacred-flame", &[], &[]).unwrap_err();
        assert_eq!(e, ResolveError::NoTargets);
        assert_eq!(e.code(), "no_targets");
        for shape in [
            TargetShape::Sphere,
            TargetShape::Cone,
            TargetShape::Line,
            TargetShape::Cube,
        ] {
            let mut area = sacred_flame();
            area.abilities[0].targeting = Some(targeting(shape, 0.0, 0));
            let r = run(&area, "cantrip-sacred-flame", &[], &[]).unwrap();
            assert!(r.resolution.outcomes.is_empty());
            assert_eq!(codes(&r), ["area_not_checked"]);
        }
    }

    #[test]
    fn unknown_ability_is_an_error() {
        let e = run(&brannoch(), "fireball", &[], &[]).unwrap_err();
        assert_eq!(e, ResolveError::UnknownAbility);
    }

    #[test]
    fn bad_dice_anywhere_fail_before_anything_is_rolled() {
        let t = [at("t", 1.0, brannoch())];
        let cases: Vec<Vec<EffectStep>> = vec![
            vec![save_step(
                AbilityScore::Dexterity,
                10,
                "1d7",
                DamageType::Fire,
                false,
            )],
            vec![flat_step(vec![part("1d7", DamageType::Fire)], "")],
            vec![flat_step(Vec::new(), "2d")],
            vec![
                save_step(AbilityScore::Dexterity, 10, "1d6", DamageType::Fire, false),
                flat_step(Vec::new(), ""),
                flat_step(Vec::new(), "x"),
            ],
        ];
        for steps in cases {
            let mut src = sacred_flame();
            src.abilities[0].steps = steps;
            let mut rng = Scripted::faces(&[]);
            let e = resolve_ability(
                &src_token(),
                &src,
                "cantrip-sacred-flame",
                &t,
                &grid(),
                RollMode::Normal,
                &mut rng,
            )
            .unwrap_err();
            assert!(matches!(e, ResolveError::Dice(_)), "{e:?}");
            assert_eq!(rng.calls, 0);
        }
    }

    #[test]
    fn a_save_roll_that_cannot_be_parsed_is_found_before_rolling() {
        let mut src = sacred_flame();
        src.abilities[0].steps = vec![save_step(
            AbilityScore::Strength,
            10,
            "1d8",
            DamageType::Radiant,
            false,
        )];
        let mut huge = brannoch();
        huge.proficiency_bonus = 1_000_000;
        let mut rng = Scripted::faces(&[]);
        let e = resolve_ability(
            &src_token(),
            &src,
            "cantrip-sacred-flame",
            &[at("t", 1.0, huge)],
            &grid(),
            RollMode::Normal,
            &mut rng,
        )
        .unwrap_err();
        assert_eq!(e, ResolveError::Dice(ParseError::NumberTooLarge));
        assert_eq!(rng.calls, 0);
    }

    #[test]
    fn save_and_flat_steps_get_range_and_count_warnings() {
        // Four targets with a limit of three; one beyond 120 ft (30 cells).
        let r = run(
            &magic_missile(),
            "magic-missile",
            &[
                at("a", 1.0, brannoch()),
                at("b", 2.0, brannoch()),
                at("c", 30.0, brannoch()),
                at("d", 3.0, brannoch()),
            ],
            &[1, 1, 1],
        )
        .unwrap();
        assert_eq!(codes(&r), ["too_many_targets", "out_of_range"]);
        assert_eq!(r.resolution.outcomes.len(), 4);
    }

    #[test]
    fn warnings_come_in_a_fixed_order_and_range_is_once_per_target() {
        let mut src = magic_missile();
        src.abilities[0].recharge = Some(Recharge { min_roll: 6 });
        src.abilities[0].recharge_spent = true;
        src.abilities[0].resource_key = "slot".to_string();
        src.abilities[0].resource_amount = 1;
        // Two steps, both using the far target: one range warning only.
        src.abilities[0].steps = vec![
            flat_step(vec![part("1d4", DamageType::Force)], ""),
            save_step(AbilityScore::Dexterity, 10, "1d4", DamageType::Force, false),
        ];
        let r = run(
            &src,
            "magic-missile",
            &[at("far", 30.0, brannoch())],
            &[1, 2, 3, 1],
        )
        .unwrap();
        assert_eq!(
            codes(&r),
            ["ability_recharging", "not_enough_resource", "out_of_range"]
        );
    }

    #[test]
    fn attack_plus_save_resolves_in_step_order_with_pending_damage() {
        let src = caster(
            "x",
            vec![ability(
                "bite",
                "Venom Bite",
                Some(targeting(TargetShape::Creatures, 5.0, 0)),
                vec![
                    attack_step(5, "1d6"),
                    save_step(
                        AbilityScore::Constitution,
                        12,
                        "2d6",
                        DamageType::Poison,
                        true,
                    ),
                ],
            )],
        );
        // Attack 10+5 = 15 vs AC 18 misses (no damage die). Save 10+4 = 14
        // succeeds against DC 15? Use DC 12 above: 14 succeeds, so halve 3+4.
        let r = run(&src, "bite", &[at("t", 1.0, brannoch())], &[10, 10, 3, 4]).unwrap();
        let o = &r.resolution.outcomes;
        assert_eq!(o.len(), 2);
        assert_eq!((kind(&o[0]), o[0].step_index), (HitOutcome::Miss, 0));
        assert_eq!(
            (kind(&o[1]), o[1].step_index),
            (HitOutcome::SaveSucceeded, 1)
        );
        assert!(r.pending[0].damage.is_empty());
        assert_eq!(r.pending[1].damage, vec![halved(DamageType::Poison, 7)]);
    }

    #[test]
    fn attack_pending_damage_follows_hit_and_critical() {
        let src = caster(
            "x",
            vec![ability(
                "stab",
                "Stab",
                Some(targeting(TargetShape::Creatures, 5.0, 0)),
                vec![attack_step(5, "1d8+3")],
            )],
        );
        let t = [at("t", 1.0, brannoch())];
        let r = run(&src, "stab", &t, &[15, 5]).unwrap();
        assert_eq!(r.pending[0].damage, vec![dmg(DamageType::Slashing, 8)]);
        // Critical: 2d8 = 5 + 6, +3.
        let r = run(&src, "stab", &t, &[20, 5, 6]).unwrap();
        assert_eq!(r.pending[0].damage, vec![dmg(DamageType::Slashing, 14)]);
    }

    #[test]
    fn negative_totals_count_as_zero_and_each_part_halves_alone() {
        let mut src = sacred_flame();
        src.abilities[0].steps = vec![EffectStep {
            step: Some(effect_step::Step::Save(SaveStep {
                ability: AbilityScore::Dexterity as i32,
                dc: 5,
                damage: vec![
                    part("1d4-5", DamageType::Fire),
                    part("1d8", DamageType::Cold),
                    part("1d6", DamageType::Acid),
                ],
                half_on_success: true,
            })),
        }];
        // Save 15 + 2 succeeds. Parts: 1d4-5 = -4 -> 0; 1d8 = 5 -> 2; 1d6 = 1 -> 0.
        let r = run(
            &src,
            "cantrip-sacred-flame",
            &[at("t", 1.0, brannoch())],
            &[15, 1, 5, 1],
        )
        .unwrap();
        assert_eq!(
            r.pending[0].damage,
            vec![
                halved(DamageType::Fire, -4),
                halved(DamageType::Cold, 5),
                halved(DamageType::Acid, 1)
            ]
        );
        let applied: Vec<i32> = r.resolution.outcomes[0]
            .damage
            .iter()
            .map(|d| d.applied)
            .collect();
        assert_eq!(applied, [0, 2, 0]);
        assert_eq!(r.resolution.outcomes[0].damage_rolls.len(), 3);
    }

    #[test]
    fn no_creature_is_modified_by_a_save() {
        let src = sacred_flame();
        let before = src.clone();
        let t = [at("t", 1.0, brannoch())];
        let t_before = t[0].1.clone();
        let r = run(&src, "cantrip-sacred-flame", &t, &[10, 5]).unwrap();
        assert_eq!(src, before);
        assert_eq!(t[0].1, t_before);
        assert_eq!(r.source, before);
        assert!(!r.resolution.outcomes[0].dropped_to_zero);
    }

    #[test]
    fn error_codes_and_display_are_stable() {
        assert_eq!(ResolveError::UnknownAbility.code(), "unknown_ability");
        assert_eq!(ResolveError::NoTargets.code(), "no_targets");
        assert_eq!(ResolveError::NotImplemented.code(), "not_implemented");
        assert_eq!(
            ResolveError::from(ParseError::BadSides).code(),
            "dice_bad_sides"
        );
        assert!(!ResolveError::NoTargets.to_string().is_empty());
        assert!(!ResolveError::NotImplemented.to_string().is_empty());
        assert!(!ResolveError::UnknownAbility.to_string().is_empty());
        let _: &dyn std::error::Error = &ResolveError::UnknownAbility;
    }

    // ---- Applying hit points (#98) ----

    fn hp_token(id: &str, cx: f32, current: i32, max: i32, temp: i32) -> Token {
        Token {
            hp_current: Some(current),
            hp_max: Some(max),
            hp_temp: Some(temp),
            health_state: crate::damage::health_state(current, max) as i32,
            ..token_at(id, cx)
        }
    }

    fn typed_attack(bonus: i32, dice: &str, ty: DamageType) -> EffectStep {
        EffectStep {
            step: Some(effect_step::Step::Attack(AttackStep {
                to_hit_bonus: bonus,
                damage: vec![part(dice, ty)],
            })),
        }
    }

    fn monster(id: &str, ac: i32, res: &[DamageType], imm: &[DamageType]) -> Creature {
        Creature {
            creature_id: id.to_string(),
            armor_class: ac,
            resistances: res.iter().map(|d| *d as i32).collect(),
            immunities: imm.iter().map(|d| *d as i32).collect(),
            ..Default::default()
        }
    }

    fn health(t: &Token) -> HealthState {
        HealthState::try_from(t.health_state).unwrap()
    }

    #[test]
    fn longsword_takes_the_goblin_to_bloodied_then_to_zero() {
        let src = caster(
            "brannoch",
            vec![ability(
                "longsword",
                "Longsword",
                Some(targeting(TargetShape::Creatures, 5.0, 1)),
                vec![typed_attack(5, "1d8+3", DamageType::Slashing)],
            )],
        );
        let goblin = monster("creature-goblin", 15, &[], &[]);
        // d20 12 + 5 = 17 hits AC 15; 1d8 = 3 + 3 = 6 of 9.
        let t = [(hp_token("goblin", 1.0, 9, 9, 0), goblin.clone())];
        let r = run(&src, "longsword", &t, &[12, 3]).unwrap();
        assert!(r.resolution.warnings.is_empty());
        let o = &r.resolution.outcomes[0];
        assert_eq!(o.damage.len(), 1);
        assert_eq!(
            (o.damage[0].rolled, o.damage[0].applied, o.dropped_to_zero),
            (6, 6, false)
        );
        assert_eq!(r.targets.len(), 1);
        assert_eq!(r.targets[0].hp_current, Some(3));
        assert_eq!(health(&r.targets[0]), HealthState::Bloodied);
        // The input token is untouched.
        assert_eq!(t[0].0.hp_current, Some(9));
        // 1d8 = 4 + 3 = 7 against 3 HP: floors at 0.
        let t = [(r.targets[0].clone(), goblin)];
        let r = run(&src, "longsword", &t, &[12, 4]).unwrap();
        assert!(r.resolution.outcomes[0].dropped_to_zero);
        assert_eq!(r.targets[0].hp_current, Some(0));
        assert_eq!(health(&r.targets[0]), HealthState::Down);
    }

    #[test]
    fn rapier_is_resisted_and_fire_bolt_is_ignored_by_the_ember_wisp() {
        let wisp = monster(
            "creature-ember-wisp",
            12,
            &[DamageType::Piercing],
            &[DamageType::Fire],
        );
        let t = [(hp_token("wisp", 1.0, 11, 11, 0), wisp)];
        let tamsin = caster(
            "tamsin",
            vec![ability(
                "rapier",
                "Rapier",
                Some(targeting(TargetShape::Creatures, 5.0, 1)),
                vec![typed_attack(5, "1d8+3", DamageType::Piercing)],
            )],
        );
        // d20 10 + 5 = 15 hits AC 12; 4 + 3 = 7, halved to 3.
        let r = run(&tamsin, "rapier", &t, &[10, 4]).unwrap();
        let d = &r.resolution.outcomes[0].damage[0];
        assert_eq!((d.rolled, d.applied), (7, 3));
        assert_eq!(r.targets[0].hp_current, Some(8));
        let ilvaren = caster(
            "ilvaren",
            vec![ability(
                "fire-bolt",
                "Fire Bolt",
                Some(targeting(TargetShape::Creatures, 120.0, 1)),
                vec![typed_attack(5, "1d10", DamageType::Fire)],
            )],
        );
        let r = run(&ilvaren, "fire-bolt", &t, &[10, 7]).unwrap();
        let d = &r.resolution.outcomes[0].damage[0];
        assert_eq!((d.rolled, d.applied), (7, 0));
        // Nothing changed, so no token comes back.
        assert!(r.targets.is_empty());
    }

    #[test]
    fn burning_hands_halves_on_a_success_and_updates_each_target() {
        let ilvaren = caster(
            "ilvaren",
            vec![ability(
                "burning-hands",
                "Burning Hands",
                Some(targeting(TargetShape::Cone, 15.0, 0)),
                vec![save_step(
                    AbilityScore::Dexterity,
                    13,
                    "3d6",
                    DamageType::Fire,
                    true,
                )],
            )],
        );
        let t = [
            (hp_token("a", 1.0, 50, 100, 0), brannoch()),
            (hp_token("b", 2.0, 50, 100, 0), monster("m", 10, &[], &[])),
        ];
        // a: 11 + 2 = 13 succeeds; b: 5 + 0 = 5 fails; 3d6 = 5, 5, 5 = 15.
        let r = run(&ilvaren, "burning-hands", &t, &[11, 5, 5, 5, 5]).unwrap();
        assert_eq!(r.resolution.outcomes[0].damage[0].applied, 7);
        assert_eq!(r.resolution.outcomes[1].damage[0].applied, 15);
        let hp: Vec<_> = r
            .targets
            .iter()
            .map(|t| (t.token_id.as_str(), t.hp_current))
            .collect();
        assert_eq!(hp, [("a", Some(43)), ("b", Some(35))]);
    }

    #[test]
    fn choking_ash_is_halved_by_maribels_poison_resistance() {
        let maribel = monster("maribel", 18, &[DamageType::Poison], &[]);
        let t = [(hp_token("maribel-tok", 1.0, 27, 27, 0), maribel)];
        // Con save 5 + 0 = 5 < 11 fails; 2d4 = 4 + 4 = 8, resisted to 4.
        let r = run(&ember_wisp(), "choking-ash", &t, &[5, 4, 4]).unwrap();
        let d = &r.resolution.outcomes[0].damage[0];
        assert_eq!((d.rolled, d.applied), (8, 4));
        assert_eq!(r.targets[0].hp_current, Some(23));
        assert_eq!(health(&r.targets[0]), HealthState::Healthy);
    }

    #[test]
    fn three_scorching_ray_hits_on_one_token_apply_in_order() {
        let src = caster(
            "ilvaren",
            vec![ability(
                "scorching-ray",
                "Scorching Ray",
                Some(targeting(TargetShape::Creatures, 120.0, 3)),
                vec![
                    typed_attack(5, "1d6", DamageType::Fire),
                    typed_attack(5, "1d6", DamageType::Fire),
                    typed_attack(5, "1d6", DamageType::Fire),
                ],
            )],
        );
        let one = (hp_token("t", 1.0, 10, 10, 0), monster("m", 10, &[], &[]));
        let t = [one.clone(), one.clone(), one];
        let r = run(&src, "scorching-ray", &t, &[15, 4, 15, 4, 15, 4]).unwrap();
        let dropped: Vec<bool> = r
            .resolution
            .outcomes
            .iter()
            .map(|o| o.dropped_to_zero)
            .collect();
        assert_eq!(dropped, [false, false, true]);
        assert_eq!(r.targets.len(), 1);
        assert_eq!(r.targets[0].hp_current, Some(0));
        assert_eq!(health(&r.targets[0]), HealthState::Down);
    }

    #[test]
    fn a_token_that_is_source_and_target_updates_once_with_every_change() {
        // Takes 3 fire first, then heals 1d10+3 = 8, on the source's own token.
        let src = caster(
            "brannoch",
            vec![ability(
                "second-wind",
                "Second Wind",
                Some(targeting(TargetShape::Self_, 0.0, 0)),
                vec![flat_step(vec![part("1d4", DamageType::Fire)], "1d10+3")],
            )],
        );
        let source_token = hp_token("tok-src", 0.0, 20, 28, 0);
        let r = resolve_ability(
            &source_token,
            &src,
            "second-wind",
            &[],
            &grid(),
            RollMode::Normal,
            &mut Scripted::faces(&[3, 5]),
        )
        .unwrap();
        assert_eq!(r.targets.len(), 1);
        assert_eq!(r.targets[0].token_id, "tok-src");
        assert_eq!(r.targets[0].hp_current, Some(25));
        let o = &r.resolution.outcomes[0];
        assert_eq!((o.damage[0].applied, o.healing), (3, 8));
    }

    #[test]
    fn healing_is_capped_and_reports_what_was_restored() {
        let r = resolve_ability(
            &hp_token("tok-src", 0.0, 25, 28, 4),
            &brannoch_with_second_wind(),
            "second-wind",
            &[],
            &grid(),
            RollMode::Normal,
            &mut Scripted::faces(&[9]),
        )
        .unwrap();
        // 9 + 3 = 12 offered, 3 restored; temporary HP untouched.
        assert_eq!(r.resolution.outcomes[0].healing, 3);
        assert_eq!(r.pending[0].healing, 12);
        assert_eq!(r.targets[0].hp_current, Some(28));
        assert_eq!(r.targets[0].hp_temp, Some(4));
    }

    #[test]
    fn a_token_without_hit_points_gets_one_warning_and_no_change() {
        let src = caster(
            "ilvaren",
            vec![ability(
                "scorching-ray",
                "Scorching Ray",
                Some(targeting(TargetShape::Creatures, 120.0, 3)),
                vec![
                    typed_attack(5, "1d6", DamageType::Fire),
                    typed_attack(5, "1d6", DamageType::Fire),
                ],
            )],
        );
        let creature = monster("m", 10, &[DamageType::Fire], &[]);
        let mut bare = token_at("t", 1.0);
        bare.hp_current = None;
        bare.hp_max = None;
        bare.hp_temp = None;
        let t = [(bare.clone(), creature.clone()), (bare, creature)];
        let r = run(&src, "scorching-ray", &t, &[15, 6, 15, 6]).unwrap();
        assert_eq!(codes(&r), ["no_hit_points"]);
        for o in &r.resolution.outcomes {
            assert_eq!((o.damage[0].rolled, o.damage[0].applied), (6, 3));
            assert!(!o.dropped_to_zero);
        }
        assert!(r.targets.is_empty());
    }
}
