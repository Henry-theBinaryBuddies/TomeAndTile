//! Attack resolution: an attack roll against armor class, and damage dice on
//! a hit (ADR 0005, ADR 0006).
//!
//! Pure: no I/O, no clock, no id minting. Randomness is an injected
//! [`Rng`]. Nothing here changes hit points: damage is rolled but not applied
//! (that is a later milestone), so `TargetOutcome.damage` stays empty,
//! `dropped_to_zero` stays false and no creature is modified. `roll_id` and
//! `resolution_id` are left empty for the server to fill.

use std::fmt;

use tt_contracts::v1::{
    effect_step, Ability, Creature, HitOutcome, Resolution, ResolutionState, RollMode, RollPurpose,
    RollResult, TargetOutcome,
};

use crate::dice::{self, Expr, ParseError, Rng};

/// Why an ability could not be resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttackError {
    /// The ability id is not on the source creature.
    UnknownAbility,
    /// The ability has a step this engine cannot resolve yet: a save, flat,
    /// apply-effect or plugin step (or an empty step).
    NotImplemented,
    /// A dice expression (or the attack's `1d20` expression) was rejected.
    Dice(ParseError),
}

impl AttackError {
    /// Stable snake_case code for clients and logs. The server maps
    /// `unknown_ability` and `not_implemented` to the rejections of the same
    /// name; dice errors carry the [`ParseError::code`].
    pub fn code(&self) -> &'static str {
        match self {
            AttackError::UnknownAbility => "unknown_ability",
            AttackError::NotImplemented => "not_implemented",
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
struct Prepared {
    step_index: u32,
    attack: Expr,
    /// Each damage part as written, and with its dice doubled for a critical.
    damage: Vec<(Expr, Expr)>,
}

fn prepare(ability: &Ability) -> Result<Vec<Prepared>, AttackError> {
    let mut out = Vec::new();
    for (i, step) in ability.steps.iter().enumerate() {
        let Some(effect_step::Step::Attack(a)) = &step.step else {
            return Err(AttackError::NotImplemented);
        };
        let attack = Expr::parse(&format!("1d20{:+}", a.to_hit_bonus))?;
        let mut damage = Vec::with_capacity(a.damage.len());
        for part in &a.damage {
            let normal = Expr::parse(&part.dice)?;
            // A critical that would exceed the dice limit is an error found
            // now, not only when a critical happens to be rolled.
            let doubled = normal.doubled_dice().ok_or(ParseError::TooManyDice)?;
            damage.push((normal, doubled));
        }
        out.push(Prepared {
            // A step list is far shorter than u32::MAX; saturate regardless.
            step_index: u32::try_from(i).unwrap_or(u32::MAX),
            attack,
            damage,
        });
    }
    Ok(out)
}

fn roll_result(
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

fn resolve_one(
    source: &Creature,
    ability: &Ability,
    step: &Prepared,
    target_token_id: &str,
    target: &Creature,
    mode: RollMode,
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

/// Resolve `ability_id` on `source`, an ability made only of attack steps.
///
/// `targets` are (token id, creature) pairs in the order listed. With one
/// attack step every entry is a target of that step. With several, entry `n`
/// is the target of the `n`th attack step: entries past the last step are
/// ignored and steps past the last entry are not rolled. Each outcome carries
/// its `step_index`. An ability with no steps resolves to a done `Resolution`
/// with no outcomes.
///
/// Errors: [`AttackError::UnknownAbility`], [`AttackError::NotImplemented`]
/// for any non-attack step, or [`AttackError::Dice`] for dice that do not
/// parse. All are detected before any die is rolled.
pub fn resolve_attacks(
    source_token_id: &str,
    source: &Creature,
    ability_id: &str,
    targets: &[(String, Creature)],
    mode: RollMode,
    rng: &mut dyn Rng,
) -> Result<Resolution, AttackError> {
    let ability = source
        .abilities
        .iter()
        .find(|a| a.ability_id == ability_id)
        .ok_or(AttackError::UnknownAbility)?;
    let steps = prepare(ability)?;

    let mut outcomes = Vec::new();
    if let [only] = steps.as_slice() {
        for (token_id, creature) in targets {
            outcomes.push(resolve_one(
                source, ability, only, token_id, creature, mode, rng,
            ));
        }
    } else {
        for (step, (token_id, creature)) in steps.iter().zip(targets) {
            outcomes.push(resolve_one(
                source, ability, step, token_id, creature, mode, rng,
            ));
        }
    }

    Ok(Resolution {
        resolution_id: String::new(),
        source_token_id: source_token_id.to_string(),
        ability_id: ability.ability_id.clone(),
        ability_name: ability.name.clone(),
        state: ResolutionState::Done as i32,
        outcomes,
        warnings: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tt_contracts::v1::{
        ApplyEffectStep, AttackStep, DamagePart, DamageType, EffectStep, FlatStep, PluginStep,
        SaveStep,
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

    fn tgt(token: &str, ac: i32) -> (String, Creature) {
        (
            token.to_string(),
            creature(&format!("c-{token}"), ac, Vec::new()),
        )
    }

    fn run(
        source: &Creature,
        id: &str,
        targets: &[(String, Creature)],
        mode: RollMode,
        faces: &[u64],
    ) -> Result<Resolution, AttackError> {
        resolve_attacks(
            "tok-src",
            source,
            id,
            targets,
            mode,
            &mut Scripted::faces(faces),
        )
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
        assert!(r.warnings.is_empty());
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
            let e = run(&src, "longsword", &[], RollMode::Normal, &[]).unwrap_err();
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
            let e = run(&src, "longsword", &[], RollMode::Normal, &[]).unwrap_err();
            assert_eq!(e, AttackError::NotImplemented);
        }
    }

    #[test]
    fn bad_dice_text_carries_the_dice_code_before_any_roll() {
        let mut src = brannoch();
        src.abilities[0].steps = vec![attack_step(5, &["1d7"], DamageType::Slashing)];
        let mut rng = Scripted::faces(&[]);
        let e = resolve_attacks(
            "s",
            &src,
            "longsword",
            &[tgt("t", 10)],
            RollMode::Normal,
            &mut rng,
        )
        .unwrap_err();
        assert_eq!(e, AttackError::Dice(ParseError::BadSides));
        assert_eq!(e.code(), "dice_bad_sides");
        assert_eq!(rng.calls, 0);

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
        assert_eq!(AttackError::Dice(ParseError::Empty).code(), "dice_empty");
        assert!(!AttackError::NotImplemented.to_string().is_empty());
        let _: &dyn std::error::Error = &AttackError::UnknownAbility;
    }
}
