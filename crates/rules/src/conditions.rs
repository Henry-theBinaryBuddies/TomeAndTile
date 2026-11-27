//! Conditions: applying them, immunity, round timers and the SRD conditions
//! that change rolls (ADR 0005; ids and timer rules from #95).
//!
//! Pure functions over [`Token`]s. [`resolve_ability`](crate::ability) uses
//! [`timer_for`] and [`apply_condition`] for apply-effect steps and the
//! roll helpers here for attacks and saves; the caller (the server, #100)
//! uses [`expire_timers`] at every turn change.
//!
//! # Which ids do what
//!
//! Rules effects are decided here and nowhere else. Condition ids are matched
//! exactly (lower case).
//!
//! | id | effect |
//! |----|--------|
//! | `blinded`, `frightened`, `poisoned`, `prone`, `restrained` | the holder has disadvantage on its attack rolls |
//! | `invisible` | the holder has advantage on its attack rolls; attacks against it have disadvantage |
//! | `blinded`, `paralyzed`, `petrified`, `restrained`, `stunned`, `unconscious` | attacks against the holder have advantage |
//! | `prone` | attacks against the holder have advantage from within 5 feet, disadvantage from farther |
//! | `paralyzed`, `unconscious` | a hit from within 5 feet is a critical hit (damage dice doubled) |
//! | `paralyzed`, `petrified`, `stunned`, `unconscious` | the holder automatically fails Strength and Dexterity saves: no d20 is rolled, `roll` is unset, the outcome is `SAVE_FAILED` |
//! | `restrained` | disadvantage on the holder's Dexterity saves |
//!
//! `charmed`, `deafened`, `grappled`, `incapacitated`, `exhaustion` and every
//! other id (`bless`, `hunters_mark`, `shield`, ...) are labels: stored,
//! timed and shown, and they never change a roll. Effects on ability checks,
//! speed or actions, and numeric bonuses (the +5 of Shield, the d4 of Bless)
//! are not modelled.
//!
//! # Advantage and disadvantage
//!
//! Sources of an attack roll's mode: the intent's `mode`, long-range
//! disadvantage, and the conditions above (the attacker's and the target's).
//! By the 5e rule any advantage plus any disadvantage gives a normal roll,
//! however many of each; otherwise any advantage gives advantage and any
//! disadvantage gives disadvantage. The roll's `mode` shows the result.
//! Conditions are read from the tokens as passed in; a condition applied by
//! an earlier step of the same ability does not affect that ability's later
//! rolls.
//!
//! # Within 5 feet
//!
//! "Within 5 feet" is [`distance_feet`] of 5 or less. Where the distance is
//! unknown (not a square grid, no position), the prone rule gives neither
//! advantage nor disadvantage and there is no automatic critical.
//! `frightened` applies without a line-of-sight check (there is none).
//!
//! # Applying an effect
//!
//! An apply-effect step with an earlier attack or save step applies to each
//! target of the nearest such step; `only_on_failure` limits it to a hit, a
//! critical hit or a failed save. With no earlier attack or save step it
//! applies to every target, each getting an `AUTOMATIC` outcome. An id in the
//! target creature's `condition_immunities` is not applied (warning
//! `condition_immune`, and the id stays out of `effects_applied`). An id
//! already on the token is not added twice, and a new timer replaces the old
//! one; a new effect with no timer leaves an existing timer alone. An
//! `INSTANT` or unspecified duration still applies the condition, with no
//! timer and no warning. An empty `effect_id` does nothing.
//!
//! # Timers
//!
//! See [`timer_for`]. Only round-based durations get a timer, and only while
//! an encounter is running (the turn context is `Some`); everything else
//! gives no timer and the warning `duration_not_tracked` (`INSTANT` and
//! unspecified give no warning). `Duration.concentration` is ignored.
//! Warning order within an ability: all `condition_immune` and
//! `duration_not_tracked` warnings in step and target order (the latter once
//! per step, when something was applied), after `out_of_range` and before
//! `no_hit_points`.

use tt_contracts::v1::{ConditionTimer, Duration, DurationUnit, Encounter, Grid, RollMode, Token};

use crate::distance::distance_feet;

/// What an effect's duration becomes; see [`timer_for`].
#[derive(Debug, Clone, PartialEq)]
pub enum Timing {
    /// A timer to store on the token.
    Timer(ConditionTimer),
    /// No timer, and the `duration_not_tracked` warning.
    Untracked,
    /// No timer and no warning (`INSTANT` or unspecified).
    Nothing,
}

/// The timer for `effect_id` with `duration`, applied by `source_token_id`.
/// `turn` is `Some((encounter_id, round))` while an encounter runs on the
/// scene. In round R:
///
/// - `ROUNDS` N: ends in round R + N at the start of the source's turn;
/// - `UNTIL_START_OF_SOURCE_NEXT_TURN`: round R + 1, at the start;
/// - `UNTIL_END_OF_SOURCE_NEXT_TURN`: round R + 1, at the end;
/// - those three outside an encounter, and `MINUTES`, `HOURS`,
///   `UNTIL_DISPELLED`: [`Timing::Untracked`];
/// - `INSTANT`, unspecified or no duration: [`Timing::Nothing`].
pub fn timer_for(
    effect_id: &str,
    duration: Option<&Duration>,
    source_token_id: &str,
    turn: Option<(&str, u32)>,
) -> Timing {
    let Some(d) = duration else {
        return Timing::Nothing;
    };
    let unit = DurationUnit::try_from(d.unit).unwrap_or(DurationUnit::Unspecified);
    let (rounds, at_end) = match unit {
        DurationUnit::Unspecified | DurationUnit::Instant => return Timing::Nothing,
        DurationUnit::Minutes | DurationUnit::Hours | DurationUnit::UntilDispelled => {
            return Timing::Untracked
        }
        DurationUnit::Rounds => (d.amount, false),
        DurationUnit::UntilStartOfSourceNextTurn => (1, false),
        DurationUnit::UntilEndOfSourceNextTurn => (1, true),
    };
    let Some((encounter_id, round)) = turn else {
        return Timing::Untracked;
    };
    Timing::Timer(ConditionTimer {
        effect_id: effect_id.to_string(),
        encounter_id: encounter_id.to_string(),
        source_token_id: source_token_id.to_string(),
        ends_round: round.saturating_add(rounds),
        at_end_of_turn: at_end,
    })
}

/// Most conditions one token may carry. The server reuses this value for its
/// own checks (`SetConditions`, import), so the two limits cannot drift.
pub const MAX_CONDITIONS: usize = 32;

/// Add condition `id` to `token` (once) and store `timer`, replacing any
/// timer for the same id. With no `timer` an existing timer is kept.
///
/// A token that already has [`MAX_CONDITIONS`] conditions gets no new one:
/// nothing changes (no timer either) and the result is `false`. Otherwise
/// the result is `true`, also when the token already had `id`.
pub fn apply_condition(token: &mut Token, id: &str, timer: Option<ConditionTimer>) -> bool {
    if !token.conditions.iter().any(|c| c == id) {
        if token.conditions.len() >= MAX_CONDITIONS {
            return false;
        }
        token.conditions.push(id.to_string());
    }
    if let Some(t) = timer {
        token.condition_timers.retain(|x| x.effect_id != id);
        token.condition_timers.push(t);
    }
    true
}

/// Remove the timers that end at this turn change, with their conditions,
/// and return the tokens that changed (others are left out), in input order.
///
/// `ended` is the token whose turn just ended and the round that turn was in
/// (`None` for the first turn of an encounter); `started` is the token whose
/// turn starts and its round. Only timers of `encounter` are looked at.
/// Rounds are given, never inferred, so a creature that appears twice in
/// the turn order is no problem.
///
/// - A start timer (`at_end_of_turn` false) ends when its source starts a
///   turn in round `ends_round` or later.
/// - An end timer ends when its source's turn ends in round `ends_round` or
///   later.
/// - A timer whose source is not a combatant ends at the start of the first
///   turn of `ends_round` or later: a started turn that is the first of its
///   round, meaning `ended` is `None` or ended in an earlier round.
///
/// A timer past its round (a missed change) ends at the next matching turn.
pub fn expire_timers(
    tokens: &[Token],
    encounter: &Encounter,
    ended: Option<(&str, u32)>,
    started: (&str, u32),
) -> Vec<Token> {
    let first_of_round = ended.is_none_or(|(_, r)| r < started.1);
    let is_combatant = |id: &str| encounter.combatants.iter().any(|c| c.token_id == id);
    let ends_now = |t: &ConditionTimer| {
        if t.encounter_id != encounter.encounter_id {
            false
        } else if !is_combatant(&t.source_token_id) {
            first_of_round && started.1 >= t.ends_round
        } else if t.at_end_of_turn {
            ended.is_some_and(|(id, r)| id == t.source_token_id && r >= t.ends_round)
        } else {
            started.0 == t.source_token_id && started.1 >= t.ends_round
        }
    };
    let mut out = Vec::new();
    for token in tokens {
        if !token.condition_timers.iter().any(ends_now) {
            continue;
        }
        let mut t = token.clone();
        let (gone, kept): (Vec<_>, Vec<_>) =
            t.condition_timers.drain(..).partition(|x| ends_now(x));
        t.condition_timers = kept;
        t.conditions
            .retain(|c| !gone.iter().any(|x| &x.effect_id == c));
        out.push(t);
    }
    out
}

fn has(token: &Token, ids: &[&str]) -> bool {
    token.conditions.iter().any(|c| ids.contains(&c.as_str()))
}

/// Whether the two tokens are within 5 feet; `None` when the distance is
/// unknown.
fn within_5_feet(grid: &Grid, a: &Token, b: &Token) -> Option<bool> {
    distance_feet(grid, a, b).map(|d| d <= 5.0)
}

/// The 5e rule: any advantage with any disadvantage is a normal roll.
fn combine(advantage: bool, disadvantage: bool) -> RollMode {
    match (advantage, disadvantage) {
        (true, false) => RollMode::Advantage,
        (false, true) => RollMode::Disadvantage,
        _ => RollMode::Normal,
    }
}

/// The mode of one attack roll from the intent's `mode`, long-range
/// disadvantage and both tokens' conditions.
pub(crate) fn attack_mode(
    intent: RollMode,
    long_range: bool,
    grid: &Grid,
    attacker: &Token,
    target: &Token,
) -> RollMode {
    let mut adv = intent == RollMode::Advantage
        || has(attacker, &["invisible"])
        || has(
            target,
            &[
                "blinded",
                "paralyzed",
                "petrified",
                "restrained",
                "stunned",
                "unconscious",
            ],
        );
    let mut dis = intent == RollMode::Disadvantage
        || long_range
        || has(
            attacker,
            &["blinded", "frightened", "poisoned", "prone", "restrained"],
        )
        || has(target, &["invisible"]);
    if has(target, &["prone"]) {
        match within_5_feet(grid, attacker, target) {
            Some(true) => adv = true,
            Some(false) => dis = true,
            None => {}
        }
    }
    combine(adv, dis)
}

/// A hit on this target from this attacker is a critical hit.
pub(crate) fn auto_crit(grid: &Grid, attacker: &Token, target: &Token) -> bool {
    has(target, &["paralyzed", "unconscious"])
        && within_5_feet(grid, attacker, target) == Some(true)
}

/// The target fails a Strength or Dexterity save without rolling.
pub(crate) fn auto_fails_save(target: &Token, strength_or_dexterity: bool) -> bool {
    strength_or_dexterity
        && has(
            target,
            &["paralyzed", "petrified", "stunned", "unconscious"],
        )
}

/// The mode of a save for this target (`dexterity`: it is a Dexterity save).
pub(crate) fn save_mode(target: &Token, dexterity: bool) -> RollMode {
    combine(false, dexterity && has(target, &["restrained"]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ability::{resolve_ability, AbilityResolution};
    use crate::dice::Rng;
    use tt_contracts::v1::{
        effect_step, Ability, AbilityScore, AbilityScores, ApplyEffectStep, AttackStep, Combatant,
        Creature, CreatureSize, DamagePart, DamageType, DiagonalRule, EffectStep, GridKind,
        HitOutcome, SaveStep, TargetOutcome, TargetShape, Targeting, Vec2,
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

    fn grid() -> Grid {
        Grid {
            kind: GridKind::Square as i32,
            cell_size: 70.0,
            offset: None,
            feet_per_cell: 5.0,
            diagonal_rule: DiagonalRule::Five as i32,
        }
    }

    fn gridless() -> Grid {
        Grid {
            kind: GridKind::None as i32,
            ..grid()
        }
    }

    /// A token in cell `cx` of row 0 (cell 1 is 5 feet from cell 0).
    fn tok(id: &str, cx: f32, conditions: &[&str]) -> Token {
        Token {
            token_id: id.to_string(),
            position: Some(Vec2 {
                x: (cx + 0.5) * 70.0,
                y: 35.0,
            }),
            size: CreatureSize::Medium as i32,
            hp_current: Some(50),
            hp_max: Some(50),
            hp_temp: Some(0),
            conditions: conditions.iter().map(|c| c.to_string()).collect(),
            ..Default::default()
        }
    }

    fn timer(effect: &str, source: &str, round: u32, end: bool) -> ConditionTimer {
        ConditionTimer {
            effect_id: effect.to_string(),
            encounter_id: "enc".to_string(),
            source_token_id: source.to_string(),
            ends_round: round,
            at_end_of_turn: end,
        }
    }

    fn dur(unit: DurationUnit, amount: u32) -> Option<Duration> {
        Some(Duration {
            unit: unit as i32,
            amount,
            concentration: false,
        })
    }

    fn effect(id: &str, duration: Option<Duration>, only_on_failure: bool) -> EffectStep {
        EffectStep {
            step: Some(effect_step::Step::ApplyEffect(ApplyEffectStep {
                effect_id: id.to_string(),
                duration,
                only_on_failure,
            })),
        }
    }

    fn save(ability: AbilityScore, dc: i32) -> EffectStep {
        EffectStep {
            step: Some(effect_step::Step::Save(SaveStep {
                ability: ability as i32,
                dc,
                damage: vec![DamagePart {
                    dice: "2d4".to_string(),
                    r#type: DamageType::Poison as i32,
                }],
                half_on_success: true,
            })),
        }
    }

    fn attack(dice: &str) -> EffectStep {
        EffectStep {
            step: Some(effect_step::Step::Attack(AttackStep {
                to_hit_bonus: 5,
                damage: vec![DamagePart {
                    dice: dice.to_string(),
                    r#type: DamageType::Slashing as i32,
                }],
            })),
        }
    }

    fn creature(id: &str, abilities: Vec<Ability>) -> Creature {
        Creature {
            creature_id: id.to_string(),
            armor_class: 10,
            scores: Some(AbilityScores {
                strength: 10,
                dexterity: 10,
                constitution: 10,
                intelligence: 10,
                wisdom: 10,
                charisma: 10,
            }),
            abilities,
            ..Default::default()
        }
    }

    fn ability(id: &str, shape: TargetShape, steps: Vec<EffectStep>) -> Ability {
        Ability {
            ability_id: id.to_string(),
            name: id.to_string(),
            targeting: Some(Targeting {
                shape: shape as i32,
                range_feet: 60.0,
                max_targets: 1,
                ..Default::default()
            }),
            steps,
            ..Default::default()
        }
    }

    /// The Ember Wisp's Choking Ash as in the seed: Con save DC 11, 2d4
    /// poison (half on success), then blinded for 1 round on a failure. The
    /// wisp is immune to poisoned.
    fn wisp() -> Creature {
        let mut c = creature(
            "creature-ember-wisp",
            vec![ability(
                "choking-ash",
                TargetShape::Creatures,
                vec![
                    save(AbilityScore::Constitution, 11),
                    effect("blinded", dur(DurationUnit::Rounds, 1), true),
                ],
            )],
        );
        c.condition_immunities = vec!["poisoned".to_string()];
        c
    }

    fn use_ability(
        source_token: &Token,
        source: &Creature,
        id: &str,
        target: (Token, Creature),
        mode: RollMode,
        turn: Option<(&str, u32)>,
        faces: &[u64],
    ) -> (AbilityResolution, usize) {
        let mut rng = Scripted::faces(faces);
        let r = resolve_ability(
            source_token,
            source,
            id,
            &[target],
            &grid(),
            mode,
            turn,
            &mut rng,
        )
        .unwrap();
        (r, rng.calls)
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

    fn roll_mode(o: &TargetOutcome) -> RollMode {
        RollMode::try_from(o.roll.as_ref().unwrap().mode).unwrap()
    }

    /// One attack of 1d8 (AC 10 target, +5 to hit) with the given conditions
    /// on the attacker and the target at cell `cx`, on a square grid.
    fn swing(
        attacker_conditions: &[&str],
        conditions: &[&str],
        cx: f32,
        mode: RollMode,
        faces: &[u64],
    ) -> AbilityResolution {
        let src = creature(
            "fighter",
            vec![ability(
                "swing",
                TargetShape::Creatures,
                vec![attack("1d8")],
            )],
        );
        use_ability(
            &tok("src", 0.0, attacker_conditions),
            &src,
            "swing",
            (tok("tgt", cx, conditions), creature("goblin", Vec::new())),
            mode,
            None,
            faces,
        )
        .0
    }

    #[test]
    fn choking_ash_blinds_on_a_failed_save_with_a_timer_to_the_next_round() {
        // Con 12: +1. d20 3 = 4 < 11. 2d4 = 2, 3.
        let mut target = creature("victim", Vec::new());
        target.scores.as_mut().unwrap().constitution = 12;
        let (r, _) = use_ability(
            &tok("wisp", 0.0, &[]),
            &wisp(),
            "choking-ash",
            (tok("victim", 1.0, &[]), target),
            RollMode::Normal,
            Some(("enc", 2)),
            &[3, 2, 3],
        );
        let o = &r.resolution.outcomes[0];
        assert_eq!(kind(o), HitOutcome::SaveFailed);
        assert_eq!(o.effects_applied, ["blinded"]);
        assert!(codes(&r).is_empty());
        assert_eq!(r.targets.len(), 1);
        assert_eq!(r.targets[0].conditions, ["blinded"]);
        assert_eq!(
            r.targets[0].condition_timers,
            [timer("blinded", "wisp", 3, false)]
        );
    }

    #[test]
    fn choking_ash_applies_nothing_on_a_successful_save() {
        let (r, _) = use_ability(
            &tok("wisp", 0.0, &[]),
            &wisp(),
            "choking-ash",
            (tok("victim", 1.0, &[]), creature("victim", Vec::new())),
            RollMode::Normal,
            Some(("enc", 2)),
            &[20, 2, 3],
        );
        let o = &r.resolution.outcomes[0];
        assert_eq!(kind(o), HitOutcome::SaveSucceeded);
        assert!(o.effects_applied.is_empty());
        assert!(r.targets[0].conditions.is_empty());
        assert!(r.targets[0].condition_timers.is_empty());
    }

    #[test]
    fn an_effect_without_only_on_failure_applies_after_a_success_too() {
        let mut src = wisp();
        src.abilities[0].steps[1] = effect("blinded", dur(DurationUnit::Rounds, 1), false);
        let (r, _) = use_ability(
            &tok("wisp", 0.0, &[]),
            &src,
            "choking-ash",
            (tok("victim", 1.0, &[]), creature("victim", Vec::new())),
            RollMode::Normal,
            Some(("enc", 2)),
            &[20, 2, 3],
        );
        assert_eq!(kind(&r.resolution.outcomes[0]), HitOutcome::SaveSucceeded);
        assert_eq!(r.targets[0].conditions, ["blinded"]);
    }

    #[test]
    fn an_effect_after_an_attack_applies_only_on_a_hit() {
        let src = creature(
            "spider",
            vec![ability(
                "bite",
                TargetShape::Creatures,
                vec![
                    attack("1d8"),
                    effect("poisoned", dur(DurationUnit::Rounds, 2), true),
                ],
            )],
        );
        let go = |faces: &[u64]| {
            use_ability(
                &tok("src", 0.0, &[]),
                &src,
                "bite",
                (tok("tgt", 1.0, &[]), creature("goblin", Vec::new())),
                RollMode::Normal,
                Some(("enc", 1)),
                faces,
            )
            .0
        };
        // 2 + 5 = 7 < AC 10: a miss, nothing applied.
        let miss = go(&[2]);
        assert_eq!(kind(&miss.resolution.outcomes[0]), HitOutcome::Miss);
        assert!(miss.resolution.outcomes[0].effects_applied.is_empty());
        assert!(miss.targets.is_empty());
        // 12 + 5 hits, damage 4.
        let hit = go(&[12, 4]);
        assert_eq!(hit.resolution.outcomes.len(), 1);
        assert_eq!(hit.resolution.outcomes[0].effects_applied, ["poisoned"]);
        assert_eq!(
            hit.targets[0].condition_timers,
            [timer("poisoned", "src", 3, false)]
        );
    }

    #[test]
    fn a_condition_immunity_blocks_the_effect_with_a_warning() {
        let src = creature(
            "spider",
            vec![ability(
                "venom",
                TargetShape::Creatures,
                vec![effect("poisoned", dur(DurationUnit::Rounds, 2), false)],
            )],
        );
        let (r, _) = use_ability(
            &tok("src", 0.0, &[]),
            &src,
            "venom",
            (tok("wisp", 1.0, &[]), wisp()),
            RollMode::Normal,
            Some(("enc", 1)),
            &[],
        );
        let o = &r.resolution.outcomes[0];
        assert_eq!(kind(o), HitOutcome::Automatic);
        assert!(o.effects_applied.is_empty());
        assert_eq!(codes(&r), ["condition_immune"]);
        assert!(r.targets.is_empty());
    }

    #[test]
    fn shield_ends_at_the_start_of_the_sources_next_turn() {
        let src = creature(
            "wizard",
            vec![ability(
                "shield",
                TargetShape::Self_,
                vec![effect(
                    "shield",
                    dur(DurationUnit::UntilStartOfSourceNextTurn, 0),
                    false,
                )],
            )],
        );
        let mut rng = Scripted::faces(&[]);
        let r = resolve_ability(
            &tok("wiz", 0.0, &[]),
            &src,
            "shield",
            &[],
            &grid(),
            RollMode::Normal,
            Some(("enc", 3)),
            &mut rng,
        )
        .unwrap();
        assert_eq!(r.targets.len(), 1);
        assert_eq!(r.targets[0].token_id, "wiz");
        assert_eq!(r.targets[0].conditions, ["shield"]);
        assert_eq!(
            r.targets[0].condition_timers,
            [timer("shield", "wiz", 4, false)]
        );
        assert!(r.resolution.warnings.is_empty());
    }

    #[test]
    fn until_end_of_next_turn_sets_an_end_timer() {
        let t = timer_for(
            "x",
            dur(DurationUnit::UntilEndOfSourceNextTurn, 0).as_ref(),
            "s",
            Some(("enc", 5)),
        );
        assert_eq!(t, Timing::Timer(timer("x", "s", 6, true)));
    }

    #[test]
    fn bless_and_hunters_mark_get_no_timer_and_a_warning() {
        for (id, unit) in [
            ("bless", DurationUnit::Minutes),
            ("hunters_mark", DurationUnit::Hours),
            ("mark", DurationUnit::UntilDispelled),
        ] {
            let src = creature(
                "caster",
                vec![ability(
                    "buff",
                    TargetShape::Creatures,
                    vec![effect(id, dur(unit, 1), false)],
                )],
            );
            let (r, _) = use_ability(
                &tok("src", 0.0, &[]),
                &src,
                "buff",
                (tok("tgt", 1.0, &[]), creature("ally", Vec::new())),
                RollMode::Normal,
                Some(("enc", 1)),
                &[],
            );
            assert_eq!(r.targets[0].conditions, [id]);
            assert!(r.targets[0].condition_timers.is_empty());
            assert_eq!(codes(&r), ["duration_not_tracked"]);
            assert_eq!(r.resolution.outcomes[0].effects_applied, [id]);
        }
    }

    #[test]
    fn rounds_outside_an_encounter_are_not_tracked_and_instant_gives_nothing() {
        let rounds = dur(DurationUnit::Rounds, 3);
        assert_eq!(
            timer_for("x", rounds.as_ref(), "s", None),
            Timing::Untracked
        );
        for d in [
            dur(DurationUnit::Instant, 0),
            dur(DurationUnit::Unspecified, 0),
            None,
        ] {
            assert_eq!(
                timer_for("x", d.as_ref(), "s", Some(("e", 1))),
                Timing::Nothing
            );
        }
        // Instant still applies the condition, with no warning.
        let src = creature(
            "c",
            vec![ability(
                "shove",
                TargetShape::Creatures,
                vec![effect("prone", dur(DurationUnit::Instant, 0), false)],
            )],
        );
        let (r, _) = use_ability(
            &tok("src", 0.0, &[]),
            &src,
            "shove",
            (tok("tgt", 1.0, &[]), creature("g", Vec::new())),
            RollMode::Normal,
            None,
            &[],
        );
        assert_eq!(r.targets[0].conditions, ["prone"]);
        assert!(r.resolution.warnings.is_empty());
    }

    #[test]
    fn a_condition_is_not_added_twice_and_a_new_timer_replaces_the_old() {
        let mut t = tok("tgt", 1.0, &["blinded"]);
        t.condition_timers = vec![timer("blinded", "old", 2, false)];
        let src = creature(
            "wisp",
            vec![ability(
                "ash",
                TargetShape::Creatures,
                vec![effect("blinded", dur(DurationUnit::Rounds, 1), false)],
            )],
        );
        let (r, _) = use_ability(
            &tok("wisp", 0.0, &[]),
            &src,
            "ash",
            (t, creature("g", Vec::new())),
            RollMode::Normal,
            Some(("enc", 4)),
            &[],
        );
        assert_eq!(r.targets[0].conditions, ["blinded"]);
        assert_eq!(
            r.targets[0].condition_timers,
            [timer("blinded", "wisp", 5, false)]
        );
        // With no new timer, the old one stays.
        let mut apply = tok("tgt", 1.0, &["bless"]);
        apply.condition_timers = vec![timer("bless", "old", 2, false)];
        assert!(apply_condition(&mut apply, "bless", None));
        assert_eq!(apply.condition_timers.len(), 1);
    }

    #[test]
    fn apply_condition_refuses_a_new_id_at_the_cap_but_keeps_known_ones() {
        let ids: Vec<String> = (0..MAX_CONDITIONS).map(|n| format!("c{n}")).collect();
        let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
        let mut full = tok("tgt", 1.0, &refs);
        let t = timer("extra", "src", 3, false);
        assert!(!apply_condition(&mut full, "extra", Some(t)));
        assert_eq!(full.conditions.len(), MAX_CONDITIONS);
        assert!(full.condition_timers.is_empty());
        // An id the token already has is still fine (its timer is replaced).
        let t = timer("c0", "src", 3, false);
        assert!(apply_condition(&mut full, "c0", Some(t)));
        assert_eq!(full.condition_timers.len(), 1);
    }

    #[test]
    fn a_poisoned_attacker_with_advantage_from_the_mode_rolls_normally() {
        let r = swing(&["poisoned"], &[], 1.0, RollMode::Advantage, &[12, 4]);
        let o = &r.resolution.outcomes[0];
        assert_eq!(roll_mode(o), RollMode::Normal);
        assert_eq!(o.roll.as_ref().unwrap().dice.len(), 1);
        // Poisoned alone is disadvantage; advantage from two sources and
        // disadvantage from two cancel.
        let r = swing(&["poisoned"], &[], 1.0, RollMode::Normal, &[12, 15, 4]);
        assert_eq!(roll_mode(&r.resolution.outcomes[0]), RollMode::Disadvantage);
        let r = swing(
            &["poisoned"],
            &["stunned", "blinded"],
            1.0,
            RollMode::Advantage,
            &[12, 4],
        );
        assert_eq!(roll_mode(&r.resolution.outcomes[0]), RollMode::Normal);
    }

    #[test]
    fn attacker_and_target_conditions_set_the_mode() {
        let mode = |att: &[&str], tgt: &[&str]| {
            let r = swing(att, tgt, 1.0, RollMode::Normal, &[12, 12, 4, 5]);
            roll_mode(&r.resolution.outcomes[0])
        };
        for c in ["blinded", "frightened", "poisoned", "prone", "restrained"] {
            assert_eq!(mode(&[c], &[]), RollMode::Disadvantage, "attacker {c}");
        }
        assert_eq!(mode(&["invisible"], &[]), RollMode::Advantage);
        for c in [
            "blinded",
            "paralyzed",
            "petrified",
            "restrained",
            "stunned",
            "unconscious",
        ] {
            assert_eq!(mode(&[], &[c]), RollMode::Advantage, "target {c}");
        }
        assert_eq!(mode(&[], &["invisible"]), RollMode::Disadvantage);
        for c in [
            "charmed",
            "deafened",
            "grappled",
            "incapacitated",
            "exhaustion",
            "bless",
        ] {
            assert_eq!(mode(&[c], &[c]), RollMode::Normal, "label {c}");
        }
    }

    #[test]
    fn a_prone_target_gives_advantage_at_5_feet_and_disadvantage_at_30() {
        let near = swing(&[], &["prone"], 1.0, RollMode::Normal, &[12, 12, 4]);
        assert_eq!(roll_mode(&near.resolution.outcomes[0]), RollMode::Advantage);
        let far = swing(&[], &["prone"], 6.0, RollMode::Normal, &[12, 12, 4]);
        assert_eq!(
            roll_mode(&far.resolution.outcomes[0]),
            RollMode::Disadvantage
        );
    }

    #[test]
    fn an_unknown_distance_gives_no_prone_effect_and_no_automatic_critical() {
        let src = creature(
            "fighter",
            vec![ability(
                "swing",
                TargetShape::Creatures,
                vec![attack("1d8")],
            )],
        );
        let go = |cond: &str, faces: &[u64]| {
            let mut rng = Scripted::faces(faces);
            resolve_ability(
                &tok("src", 0.0, &[]),
                &src,
                "swing",
                &[(tok("tgt", 1.0, &[cond]), creature("g", Vec::new()))],
                &gridless(),
                RollMode::Normal,
                None,
                &mut rng,
            )
            .unwrap()
        };
        let prone = go("prone", &[12, 4]);
        assert_eq!(roll_mode(&prone.resolution.outcomes[0]), RollMode::Normal);
        // Paralyzed gives advantage (two d20) and a hit, but not a critical.
        let para = go("paralyzed", &[12, 12, 4]);
        assert_eq!(kind(&para.resolution.outcomes[0]), HitOutcome::Hit);
    }

    #[test]
    fn a_hit_on_a_paralyzed_target_is_a_critical_from_5_feet_but_not_from_10() {
        // Advantage from paralyzed: two d20; 1d8 doubles to 2d8 on a critical.
        let near = swing(&[], &["paralyzed"], 1.0, RollMode::Normal, &[12, 12, 4, 5]);
        let o = &near.resolution.outcomes[0];
        assert_eq!(kind(o), HitOutcome::CriticalHit);
        assert_eq!(o.damage_rolls[0].dice.len(), 2);
        let far = swing(&[], &["paralyzed"], 2.0, RollMode::Normal, &[12, 12, 4]);
        let o = &far.resolution.outcomes[0];
        assert_eq!(kind(o), HitOutcome::Hit);
        assert_eq!(o.damage_rolls[0].dice.len(), 1);
        let down = swing(
            &[],
            &["unconscious"],
            1.0,
            RollMode::Normal,
            &[12, 12, 4, 5],
        );
        assert_eq!(kind(&down.resolution.outcomes[0]), HitOutcome::CriticalHit);
        // A miss stays a miss: 1 + 5 < AC 10.
        let miss = swing(&[], &["paralyzed"], 1.0, RollMode::Normal, &[1, 1]);
        assert_eq!(kind(&miss.resolution.outcomes[0]), HitOutcome::Miss);
    }

    fn save_against(
        conditions: &[&str],
        score: AbilityScore,
        faces: &[u64],
    ) -> (AbilityResolution, usize) {
        let src = creature(
            "caster",
            vec![ability(
                "burst",
                TargetShape::Creatures,
                vec![save(score, 10)],
            )],
        );
        use_ability(
            &tok("src", 0.0, &[]),
            &src,
            "burst",
            (tok("tgt", 1.0, conditions), creature("g", Vec::new())),
            RollMode::Normal,
            None,
            faces,
        )
    }

    #[test]
    fn a_stunned_target_automatically_fails_a_dexterity_save_without_a_roll() {
        // Only the 2d4 damage is drawn: no d20.
        let (r, draws) = save_against(&["stunned"], AbilityScore::Dexterity, &[2, 3]);
        let o = &r.resolution.outcomes[0];
        assert_eq!(kind(o), HitOutcome::SaveFailed);
        assert!(o.roll.is_none());
        assert_eq!(draws, 2);
        assert_eq!(o.damage[0].applied, 5);
        for c in ["paralyzed", "petrified", "unconscious"] {
            let (r, _) = save_against(&[c], AbilityScore::Strength, &[2, 3]);
            assert!(r.resolution.outcomes[0].roll.is_none(), "{c}");
        }
        // Constitution saves are still rolled.
        let (r, _) = save_against(&["stunned"], AbilityScore::Constitution, &[15, 2, 3]);
        assert!(r.resolution.outcomes[0].roll.is_some());
    }

    #[test]
    fn a_restrained_target_has_disadvantage_on_dexterity_saves_only() {
        let (r, _) = save_against(&["restrained"], AbilityScore::Dexterity, &[15, 3, 2, 3]);
        assert_eq!(roll_mode(&r.resolution.outcomes[0]), RollMode::Disadvantage);
        let (r, _) = save_against(&["restrained"], AbilityScore::Strength, &[15, 2, 3]);
        assert_eq!(roll_mode(&r.resolution.outcomes[0]), RollMode::Normal);
    }

    // ---- Expiry ----

    fn encounter(order: &[&str], round: u32) -> Encounter {
        Encounter {
            encounter_id: "enc".to_string(),
            round,
            combatants: order
                .iter()
                .map(|t| Combatant {
                    token_id: t.to_string(),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        }
    }

    fn timed(id: &str, effect: &str, t: ConditionTimer) -> Token {
        let mut token = tok(id, 0.0, &[effect, "label"]);
        token.condition_timers = vec![t];
        token
    }

    #[test]
    fn a_start_timer_ends_at_the_start_of_the_sources_turn_in_its_round() {
        let enc = encounter(&["wisp", "victim"], 3);
        let victim = timed("victim", "blinded", timer("blinded", "wisp", 3, false));
        let one = std::slice::from_ref(&victim);
        // The wisp starts in round 2: too early.
        assert!(expire_timers(one, &enc, Some(("victim", 1)), ("wisp", 2)).is_empty());
        // Round 3, another combatant starts: nothing.
        assert!(expire_timers(one, &enc, Some(("wisp", 3)), ("victim", 3)).is_empty());
        // The wrap into round 3, the wisp starts.
        let out = expire_timers(one, &enc, Some(("victim", 2)), ("wisp", 3));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].conditions, ["label"]);
        assert!(out[0].condition_timers.is_empty());
    }

    #[test]
    fn an_end_timer_ends_at_the_end_of_the_sources_turn() {
        let enc = encounter(&["wisp", "victim"], 3);
        let victim = timed("victim", "shield", timer("shield", "wisp", 3, true));
        let one = std::slice::from_ref(&victim);
        // The wisp's turn in round 2 ends: too early.
        assert!(expire_timers(one, &enc, Some(("wisp", 2)), ("victim", 2)).is_empty());
        // The wisp's turn in round 3 ends.
        let out = expire_timers(one, &enc, Some(("wisp", 3)), ("victim", 3));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].conditions, ["label"]);
        assert!(out[0].condition_timers.is_empty());
        // Starting the wisp's turn does not end an end timer.
        assert!(expire_timers(one, &enc, Some(("victim", 2)), ("wisp", 3)).is_empty());
    }

    #[test]
    fn an_end_timer_for_the_last_combatant_waits_out_the_wrap() {
        let enc = encounter(&["wisp", "victim"], 3);
        let victim = timed("victim", "x", timer("x", "victim", 2, true));
        // The victim's turn in round 2 ends and round 3 starts.
        let out = expire_timers(&[victim], &enc, Some(("victim", 2)), ("wisp", 3));
        assert_eq!(out.len(), 1);
        // A timer for round 3 must not end at that wrap.
        let later = timed("victim", "x", timer("x", "victim", 3, true));
        assert!(expire_timers(&[later], &enc, Some(("victim", 2)), ("wisp", 3)).is_empty());
    }

    #[test]
    fn a_token_that_acts_twice_in_a_round_is_handled_by_the_given_rounds() {
        // The wisp acts first and last; a start timer ends at its first turn
        // in the round, an end timer at the end of either turn.
        let enc = encounter(&["wisp", "victim", "wisp"], 3);
        let start = timed("victim", "a", timer("a", "wisp", 3, false));
        let end = timed("victim", "b", timer("b", "wisp", 3, true));
        // Round 2's last turn (the wisp's second) ends, round 3 starts with the wisp.
        assert!(expire_timers(
            std::slice::from_ref(&end),
            &enc,
            Some(("wisp", 2)),
            ("wisp", 3)
        )
        .is_empty());
        let out = expire_timers(
            std::slice::from_ref(&start),
            &enc,
            Some(("wisp", 2)),
            ("wisp", 3),
        );
        assert_eq!(out.len(), 1);
        // The wisp's first turn of round 3 ends: the end timer goes.
        let out = expire_timers(
            std::slice::from_ref(&end),
            &enc,
            Some(("wisp", 3)),
            ("victim", 3),
        );
        assert_eq!(out.len(), 1);
        // Its second turn of round 3 starting is not a wrap: a non-combatant
        // timer does not fire there.
        let ghost = timed("victim", "c", timer("c", "gone", 3, false));
        assert!(expire_timers(&[ghost], &enc, Some(("victim", 3)), ("wisp", 3)).is_empty());
    }

    #[test]
    fn a_source_that_is_not_a_combatant_ends_at_the_first_turn_of_its_round() {
        let enc = encounter(&["a", "b"], 4);
        let t = timed("a", "x", timer("x", "gone", 4, true));
        let t2 = timed("b", "y", timer("y", "gone", 4, false));
        let tokens = [t, t2];
        // Round 3 wrap: too early.
        assert!(expire_timers(&tokens, &enc, Some(("b", 2)), ("a", 3)).is_empty());
        // A mid-round turn of round 4: not yet.
        assert!(expire_timers(&tokens, &enc, Some(("a", 4)), ("b", 4)).is_empty());
        // The first turn of round 4: both end.
        let out = expire_timers(&tokens, &enc, Some(("b", 3)), ("a", 4));
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|t| t.condition_timers.is_empty()));
    }

    #[test]
    fn expiry_returns_only_changed_tokens_and_ignores_other_encounters() {
        let enc = encounter(&["wisp", "victim"], 3);
        let mut other = timer("blinded", "wisp", 3, false);
        other.encounter_id = "elsewhere".to_string();
        let tokens = [
            timed("a", "blinded", other),
            tok("b", 1.0, &["prone"]),
            timed("c", "blinded", timer("blinded", "wisp", 3, false)),
        ];
        let out = expire_timers(&tokens, &enc, Some(("victim", 2)), ("wisp", 3));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].token_id, "c");
    }

    #[test]
    fn the_first_turn_of_an_encounter_has_no_ended_turn() {
        let enc = encounter(&["wisp", "victim"], 1);
        let t = timed("victim", "x", timer("x", "wisp", 1, true));
        assert!(expire_timers(&[t], &enc, None, ("wisp", 1)).is_empty());
        // A non-combatant timer for round 1 ends at that first turn.
        let g = timed("victim", "y", timer("y", "gone", 1, false));
        assert_eq!(expire_timers(&[g], &enc, None, ("wisp", 1)).len(), 1);
    }
}
