//! `UseAbilityIntent`: attack, save, flat and apply-effect steps, applied to
//! the target tokens in a simplified way. See the README for what is skipped.

use std::collections::HashMap;

use tt_contracts::v1::{
    effect_step, event, Ability, AbilityScore, AbilityScores, Creature, DamageApplied, DamagePart,
    HitOutcome, Modifier, Resolution, ResolutionState, ResolutionUpdated, RollMode, RollPurpose,
    RollResult, TargetOutcome, TargetShape, Token, TokenUpserted, UseAbilityIntent, Warning,
};

use crate::dice::{self, Notation};

/// Most target ids in one `UseAbilityIntent` (the real server's `MAX_TARGETS`).
const MAX_TARGETS: usize = 64;
use crate::tokens::{apply_damage, apply_healing, has_hp, refresh_health};
use crate::{warning, Before, Game, Group, Outcome, Rejection, UndoRecord};

/// One damage part: its type and its dice.
type Parts = Vec<(i32, Notation)>;

/// One step with its dice parsed ahead of any die thrown.
enum Step {
    Attack {
        index: u32,
        attack: Notation,
        /// Each part as written, and with its dice doubled for a critical hit.
        damage: Vec<(i32, Notation, Notation)>,
    },
    Save {
        index: u32,
        ability: AbilityScore,
        dc: i32,
        damage: Parts,
        half_on_success: bool,
    },
    Flat {
        index: u32,
        damage: Parts,
        healing: Option<Notation>,
    },
    Apply {
        index: u32,
        effect_id: String,
        only_on_failure: bool,
    },
}

/// A resolved target: what the steps need to know about its creature.
struct Target {
    token_id: String,
    creature_id: String,
    armor_class: i32,
    scores: AbilityScores,
    proficiency_bonus: i32,
    save_proficiencies: Vec<i32>,
}

impl Target {
    fn new(token_id: &str, c: &Creature) -> Self {
        Self {
            token_id: token_id.to_string(),
            creature_id: c.creature_id.clone(),
            armor_class: c.armor_class,
            scores: c.scores.unwrap_or_default(),
            proficiency_bonus: c.proficiency_bonus,
            save_proficiencies: c.save_proficiencies.clone(),
        }
    }

    /// The ability modifier and the proficiency bonus (0 if not proficient)
    /// that go on this target's save.
    fn save_bonus(&self, ability: AbilityScore) -> (i32, i32) {
        let s = &self.scores;
        let score = match ability {
            AbilityScore::Strength => s.strength,
            AbilityScore::Dexterity => s.dexterity,
            AbilityScore::Constitution => s.constitution,
            AbilityScore::Intelligence => s.intelligence,
            AbilityScore::Wisdom => s.wisdom,
            AbilityScore::Charisma => s.charisma,
            AbilityScore::Unspecified => return (0, 0),
        };
        let proficient = self.save_proficiencies.contains(&(ability as i32));
        (
            (score - 10).div_euclid(2),
            if proficient {
                self.proficiency_bonus
            } else {
                0
            },
        )
    }
}

fn unparsed() -> Rejection {
    (
        "not_supported_by_mock",
        "the mock parses only NdM, NdM+K and NdM-K dice".to_string(),
    )
}

fn parse_parts(parts: &[DamagePart]) -> Result<Parts, Rejection> {
    parts
        .iter()
        .map(|p| {
            Ok((
                p.r#type,
                dice::parse_notation(&p.dice).ok_or_else(unparsed)?,
            ))
        })
        .collect()
}

/// Parse every step. A plugin step, or dice outside what the mock parses, is
/// `not_supported_by_mock`.
fn prepare(ability: &Ability) -> Result<Vec<Step>, Rejection> {
    let mut out = Vec::with_capacity(ability.steps.len());
    for (i, step) in ability.steps.iter().enumerate() {
        let index = u32::try_from(i).unwrap_or(u32::MAX);
        out.push(match &step.step {
            Some(effect_step::Step::Attack(a)) => {
                let mut damage = Vec::with_capacity(a.damage.len());
                for part in &a.damage {
                    let normal = dice::parse_notation(&part.dice).ok_or_else(unparsed)?;
                    damage.push((
                        part.r#type,
                        normal,
                        normal.doubled_dice().ok_or_else(unparsed)?,
                    ));
                }
                Step::Attack {
                    index,
                    attack: Notation::attack(a.to_hit_bonus),
                    damage,
                }
            }
            Some(effect_step::Step::Save(s)) => Step::Save {
                index,
                ability: AbilityScore::try_from(s.ability).unwrap_or(AbilityScore::Unspecified),
                dc: s.dc,
                damage: parse_parts(&s.damage)?,
                half_on_success: s.half_on_success,
            },
            Some(effect_step::Step::Flat(f)) => Step::Flat {
                index,
                damage: parse_parts(&f.damage)?,
                healing: if f.healing_dice.trim().is_empty() {
                    None
                } else {
                    Some(dice::parse_notation(&f.healing_dice).ok_or_else(unparsed)?)
                },
            },
            Some(effect_step::Step::ApplyEffect(a)) => Step::Apply {
                index,
                effect_id: a.effect_id.clone(),
                only_on_failure: a.only_on_failure,
            },
            Some(effect_step::Step::Plugin(_)) | None => {
                return Err((
                    "not_supported_by_mock",
                    "the mock does not run plugin steps".to_string(),
                ))
            }
        });
    }
    Ok(out)
}

/// A target token as it was and as the resolution is changing it. Nothing is
/// written to the game until every roll has succeeded.
struct Work {
    /// Position in `Game::tokens`.
    index: usize,
    before: Token,
    token: Token,
}

/// The state of one resolution while it is being rolled.
struct Run {
    /// Resolution number; roll ids are `roll-<n>-<k>`.
    n: u64,
    roll_k: u32,
    user_id: String,
    source_creature_id: String,
    ability_name: String,
    mode: RollMode,
    outcomes: Vec<TargetOutcome>,
    /// Latest outcome row per token, for apply-effect steps to attach to.
    last_row: HashMap<String, usize>,
    /// Distinct target tokens in first-listed order.
    work: Vec<Work>,
    /// Tokens already warned about for `no_hit_points`.
    no_hp_warned: Vec<String>,
    warnings: Vec<Warning>,
}

impl Run {
    fn work_mut(&mut self, token_id: &str) -> &mut Work {
        let i = self
            .work
            .iter()
            .position(|w| w.token.token_id == token_id)
            .expect("every target has a working copy");
        &mut self.work[i]
    }

    fn push_row(&mut self, row: TargetOutcome) {
        self.last_row
            .insert(row.token_id.clone(), self.outcomes.len());
        self.outcomes.push(row);
    }

    /// Take damage off the target, or warn once that it has no hit points.
    /// Returns true if it dropped to 0.
    fn damage(&mut self, token_id: &str, amount: i32) -> bool {
        if has_hp(&self.work_mut(token_id).token) {
            return apply_damage(&mut self.work_mut(token_id).token, amount);
        }
        self.warn_no_hp(token_id);
        false
    }

    /// Heal the target; the hit points restored, or the rolled amount shown
    /// (not applied) with a warning when it has no hit points.
    fn heal(&mut self, token_id: &str, amount: i32) -> i32 {
        if has_hp(&self.work_mut(token_id).token) {
            return apply_healing(&mut self.work_mut(token_id).token, amount);
        }
        self.warn_no_hp(token_id);
        amount.max(0)
    }

    fn warn_no_hp(&mut self, token_id: &str) {
        if !self.no_hp_warned.iter().any(|t| t == token_id) {
            self.no_hp_warned.push(token_id.to_string());
            self.warnings.push(warning(
                "no_hit_points",
                format!("token {token_id:?} has no hit points; the result is shown, not applied"),
            ));
        }
    }
}

fn damage_applied(rolls: &[(i32, RollResult)], halve: bool) -> Vec<DamageApplied> {
    rolls
        .iter()
        .map(|(ty, roll)| {
            let rolled = roll.total.max(0);
            DamageApplied {
                r#type: *ty,
                rolled,
                applied: if halve { rolled / 2 } else { rolled },
            }
        })
        .collect()
}

impl Game {
    /// One roll inside resolution `run.n`, id `roll-<n>-<k>` (the freeform ids
    /// are `roll-<seq>`, so the two never collide).
    fn roll(
        &mut self,
        run: &mut Run,
        notation: &Notation,
        mode: RollMode,
        purpose: RollPurpose,
        creature_id: &str,
        label: String,
    ) -> Result<RollResult, Rejection> {
        run.roll_k += 1;
        let mut roll = dice::roll(
            self.rng.as_mut(),
            notation,
            mode,
            format!("roll-{}-{}", run.n, run.roll_k),
            &run.user_id,
            label,
        )
        .ok_or((
            "not_supported_by_mock",
            "a roll total was out of range".to_string(),
        ))?;
        roll.purpose = purpose as i32;
        roll.roller_creature_id = creature_id.to_string();
        Ok(roll)
    }

    /// Roll each damage part once.
    fn roll_parts(
        &mut self,
        run: &mut Run,
        parts: &Parts,
        creature_id: &str,
    ) -> Result<Vec<(i32, RollResult)>, Rejection> {
        let mut rolls = Vec::with_capacity(parts.len());
        for (ty, notation) in parts {
            let label = format!("{} damage", run.ability_name);
            let roll = self.roll(
                run,
                notation,
                RollMode::Normal,
                RollPurpose::Damage,
                creature_id,
                label,
            )?;
            rolls.push((*ty, roll));
        }
        Ok(rolls)
    }

    fn run_attack(
        &mut self,
        run: &mut Run,
        step: (u32, &Notation, &[(i32, Notation, Notation)]),
        t: &Target,
    ) -> Result<(), Rejection> {
        let (index, attack, damage) = step;
        let source = run.source_creature_id.clone();
        let label = format!("{} attack", run.ability_name);
        let mode = run.mode;
        let roll = self.roll(run, attack, mode, RollPurpose::Attack, &source, label)?;
        let outcome = if roll.critical {
            HitOutcome::CriticalHit
        } else if roll.fumble || roll.total < t.armor_class {
            HitOutcome::Miss
        } else {
            HitOutcome::Hit
        };
        let mut rolls = Vec::new();
        if outcome != HitOutcome::Miss {
            for (ty, normal, doubled) in damage {
                let notation = if outcome == HitOutcome::CriticalHit {
                    doubled
                } else {
                    normal
                };
                let label = format!("{} damage", run.ability_name);
                let r = self.roll(
                    run,
                    notation,
                    RollMode::Normal,
                    RollPurpose::Damage,
                    &source,
                    label,
                )?;
                rolls.push((*ty, r));
            }
        }
        let applied = damage_applied(&rolls, false);
        let dropped =
            !rolls.is_empty() && run.damage(&t.token_id, applied.iter().map(|d| d.applied).sum());
        run.push_row(TargetOutcome {
            token_id: t.token_id.clone(),
            outcome: outcome as i32,
            roll: Some(roll),
            damage_rolls: rolls.into_iter().map(|(_, r)| r).collect(),
            damage: applied,
            healing: 0,
            effects_applied: Vec::new(),
            dropped_to_zero: dropped,
            step_index: index,
        });
        Ok(())
    }

    /// Every target saves, then the damage is rolled once for all of them.
    fn run_save(
        &mut self,
        run: &mut Run,
        step: (u32, AbilityScore, i32, &Parts, bool),
        targets: &[Target],
    ) -> Result<(), Rejection> {
        let (index, ability, dc, damage, half) = step;
        let mut saves = Vec::with_capacity(targets.len());
        for t in targets {
            let (modifier, proficiency) = t.save_bonus(ability);
            let label = format!("{} save", run.ability_name);
            let mut roll = self.roll(
                run,
                &Notation::attack(modifier + proficiency),
                RollMode::Normal,
                RollPurpose::Save,
                &t.creature_id,
                label,
            )?;
            roll.modifiers = vec![Modifier {
                source: "ability".to_string(),
                value: modifier,
            }];
            if proficiency != 0 {
                roll.modifiers.push(Modifier {
                    source: "proficiency".to_string(),
                    value: proficiency,
                });
            }
            let failed = roll.total < dc;
            saves.push((t, roll, failed));
        }
        let any_damage = saves.iter().any(|(_, _, failed)| *failed || half);
        let source = run.source_creature_id.clone();
        let rolls = if any_damage {
            self.roll_parts(run, damage, &source)?
        } else {
            Vec::new()
        };
        for (t, roll, failed) in saves {
            let takes = !rolls.is_empty() && (failed || half);
            let applied = if takes {
                damage_applied(&rolls, !failed)
            } else {
                Vec::new()
            };
            let dropped = takes && run.damage(&t.token_id, applied.iter().map(|d| d.applied).sum());
            run.push_row(TargetOutcome {
                token_id: t.token_id.clone(),
                outcome: if failed {
                    HitOutcome::SaveFailed
                } else {
                    HitOutcome::SaveSucceeded
                } as i32,
                roll: Some(roll),
                damage_rolls: if takes {
                    rolls.iter().map(|(_, r)| r.clone()).collect()
                } else {
                    Vec::new()
                },
                damage: applied,
                healing: 0,
                effects_applied: Vec::new(),
                dropped_to_zero: dropped,
                step_index: index,
            });
        }
        Ok(())
    }

    /// Damage and healing are rolled once and given to every target.
    fn run_flat(
        &mut self,
        run: &mut Run,
        step: (u32, &Parts, &Option<Notation>),
        targets: &[Target],
    ) -> Result<(), Rejection> {
        let (index, damage, healing) = step;
        let source = run.source_creature_id.clone();
        let rolls = self.roll_parts(run, damage, &source)?;
        let heal_roll = match healing {
            Some(notation) => {
                let label = format!("{} healing", run.ability_name);
                Some(self.roll(
                    run,
                    notation,
                    RollMode::Normal,
                    RollPurpose::Healing,
                    &source,
                    label,
                )?)
            }
            None => None,
        };
        for t in targets {
            let applied = damage_applied(&rolls, false);
            let dropped = !rolls.is_empty()
                && run.damage(&t.token_id, applied.iter().map(|d| d.applied).sum());
            let healed = match &heal_roll {
                Some(r) => run.heal(&t.token_id, r.total),
                None => 0,
            };
            let mut damage_rolls: Vec<RollResult> = rolls.iter().map(|(_, r)| r.clone()).collect();
            damage_rolls.extend(heal_roll.clone());
            run.push_row(TargetOutcome {
                token_id: t.token_id.clone(),
                outcome: HitOutcome::Automatic as i32,
                roll: None,
                damage_rolls,
                damage: applied,
                healing: healed,
                effects_applied: Vec::new(),
                dropped_to_zero: dropped,
                step_index: index,
            });
        }
        Ok(())
    }

    /// Add the effect to each target's token. It rides on the target's latest
    /// outcome (the attack or save it follows) and applies only after a hit or
    /// a failed save when `only_on_failure`; with no earlier outcome, or an
    /// automatic one, there is nothing to fail and it applies.
    fn run_apply(run: &mut Run, step: (u32, &str, bool), targets: &[Target]) {
        let (index, effect_id, only_on_failure) = step;
        for t in targets {
            let row = run.last_row.get(&t.token_id).copied();
            let applies = !only_on_failure
                || row.is_none_or(|i| {
                    !matches!(
                        HitOutcome::try_from(run.outcomes[i].outcome),
                        Ok(HitOutcome::Miss | HitOutcome::SaveSucceeded)
                    )
                });
            if !applies {
                continue;
            }
            let conditions = &mut run.work_mut(&t.token_id).token.conditions;
            if !conditions.iter().any(|c| c == effect_id) {
                conditions.push(effect_id.to_string());
            }
            match row {
                Some(i) => run.outcomes[i].effects_applied.push(effect_id.to_string()),
                None => run.push_row(TargetOutcome {
                    token_id: t.token_id.clone(),
                    outcome: HitOutcome::Automatic as i32,
                    effects_applied: vec![effect_id.to_string()],
                    step_index: index,
                    ..Default::default()
                }),
            }
        }
    }

    /// Resolve an ability and apply it to the target tokens. See the README
    /// for what is skipped.
    pub(crate) fn use_ability(
        &mut self,
        user_id: &str,
        u: &UseAbilityIntent,
    ) -> Result<Group, Rejection> {
        let source_index = self.controlled_token(user_id, &u.source_token_id)?;
        let source_token = self.tokens[source_index].clone();
        let source = self
            .template
            .creatures
            .iter()
            .find(|c| {
                !source_token.creature_id.is_empty() && c.creature_id == source_token.creature_id
            })
            .cloned()
            .ok_or((
                "no_creature",
                "the source token has no creature".to_string(),
            ))?;
        let ability = source
            .abilities
            .iter()
            .find(|a| a.ability_id == u.ability_id)
            .cloned()
            .ok_or((
                "unknown_ability",
                format!("the creature has no ability {:?}", u.ability_id),
            ))?;
        // As in the real server: the count first, then repeats (allowed once
        // per attack step, so only a multiattack may repeat an id).
        if u.target_token_ids.len() > MAX_TARGETS {
            return Err((
                "too_many_targets",
                format!("an ability may list at most {MAX_TARGETS} targets"),
            ));
        }
        let attack_steps = ability
            .steps
            .iter()
            .filter(|s| matches!(s.step, Some(effect_step::Step::Attack(_))))
            .count()
            .max(1);
        let mut copies: HashMap<&str, usize> = HashMap::new();
        for id in &u.target_token_ids {
            let n = copies.entry(id.as_str()).or_insert(0);
            *n += 1;
            if *n > attack_steps {
                return Err((
                    "duplicate_target",
                    format!("token {id:?} is listed more than {attack_steps} time(s)"),
                ));
            }
        }
        let shape = ability
            .targeting
            .as_ref()
            .map(|t| TargetShape::try_from(t.shape).unwrap_or(TargetShape::Unspecified));
        if shape == Some(TargetShape::Creatures) && u.target_token_ids.is_empty() {
            return Err((
                "no_targets",
                "the ability needs at least one target".to_string(),
            ));
        }
        let is_gm = user_id == crate::GM_USER;
        let targets: Vec<Target> = if shape == Some(TargetShape::Self_) {
            vec![Target::new(&source_token.token_id, &source)]
        } else {
            let mut targets = Vec::with_capacity(u.target_token_ids.len());
            for id in &u.target_token_ids {
                let creature = self
                    .tokens
                    .iter()
                    .find(|t| &t.token_id == id && (is_gm || !t.hidden))
                    .filter(|t| !t.creature_id.is_empty())
                    .and_then(|t| {
                        self.template
                            .creatures
                            .iter()
                            .find(|c| c.creature_id == t.creature_id)
                    });
                let Some(creature) = creature else {
                    return Err((
                        "unknown_target",
                        format!("token {id:?} does not exist or has no creature"),
                    ));
                };
                targets.push(Target::new(id, creature));
            }
            targets
        };
        let steps = prepare(&ability)?;
        let attacks = steps
            .iter()
            .filter(|s| matches!(s, Step::Attack { .. }))
            .count();
        if attacks > 1 && attacks != steps.len() {
            return Err((
                "not_implemented",
                "an ability with several attack steps and a save or flat step is not implemented"
                    .to_string(),
            ));
        }
        let area = matches!(
            shape,
            Some(TargetShape::Sphere | TargetShape::Cone | TargetShape::Line | TargetShape::Cube)
        );

        let mut warnings = Vec::new();
        if attacks == 1 {
            let max = ability.targeting.as_ref().map_or(0, |t| t.max_targets);
            let limited = matches!(
                shape,
                Some(TargetShape::Creatures | TargetShape::Unspecified)
            );
            if limited && max > 0 && targets.len() > max as usize {
                warnings.push(warning(
                    "too_many_targets",
                    format!("{} targets listed, at most {max} allowed", targets.len()),
                ));
            }
        } else if attacks > 1 && targets.len() > attacks {
            warnings.push(warning(
                "extra_targets_ignored",
                format!(
                    "{} targets listed for {attacks} attack steps; the extra are ignored",
                    targets.len()
                ),
            ));
        }
        if area {
            warnings.push(warning(
                "area_not_checked",
                "no template was placed; the listed tokens are the targets".to_string(),
            ));
        }

        self.resolutions += 1;
        let mut work: Vec<Work> = Vec::new();
        for t in &targets {
            if work.iter().all(|w| w.token.token_id != t.token_id) {
                if let Some(index) = self.tokens.iter().position(|k| k.token_id == t.token_id) {
                    let token = self.tokens[index].clone();
                    work.push(Work {
                        index,
                        before: token.clone(),
                        token,
                    });
                }
            }
        }
        let mut run = Run {
            n: self.resolutions,
            roll_k: 0,
            user_id: user_id.to_string(),
            source_creature_id: source.creature_id.clone(),
            ability_name: ability.name.clone(),
            mode: RollMode::try_from(u.mode).unwrap_or(RollMode::Unspecified),
            outcomes: Vec::new(),
            last_row: HashMap::new(),
            work,
            no_hp_warned: Vec::new(),
            warnings: Vec::new(),
        };

        if attacks > 1 {
            // Several attack steps: step i hits target i; the rest are skipped.
            for (step, t) in steps.iter().zip(targets.iter()) {
                if let Step::Attack {
                    index,
                    attack,
                    damage,
                } = step
                {
                    self.run_attack(&mut run, (*index, attack, damage), t)?;
                }
            }
        } else if !targets.is_empty() {
            for step in &steps {
                match step {
                    Step::Attack {
                        index,
                        attack,
                        damage,
                    } => {
                        for t in &targets {
                            self.run_attack(&mut run, (*index, attack, damage), t)?;
                        }
                    }
                    Step::Save {
                        index,
                        ability,
                        dc,
                        damage,
                        half_on_success,
                    } => self.run_save(
                        &mut run,
                        (*index, *ability, *dc, damage, *half_on_success),
                        &targets,
                    )?,
                    Step::Flat {
                        index,
                        damage,
                        healing,
                    } => self.run_flat(&mut run, (*index, damage, healing), &targets)?,
                    Step::Apply {
                        index,
                        effect_id,
                        only_on_failure,
                    } => Self::run_apply(&mut run, (*index, effect_id, *only_on_failure), &targets),
                }
            }
        }

        warnings.append(&mut run.warnings);
        let resolution = Resolution {
            resolution_id: format!("res-{}", run.n),
            source_token_id: source_token.token_id,
            ability_id: ability.ability_id,
            ability_name: ability.name,
            state: ResolutionState::Done as i32,
            outcomes: run.outcomes,
            warnings: warnings.clone(),
        };
        let mut events: Vec<Outcome> = vec![(
            event::Kind::ResolutionUpdated(ResolutionUpdated {
                resolution: Some(resolution),
            }),
            warnings,
        )];
        // Commit, and send one upsert per changed target in target order.
        let mut before = Vec::new();
        for w in run.work {
            let mut token = w.token;
            refresh_health(&mut token);
            if token != w.before {
                events.push((
                    event::Kind::TokenUpserted(TokenUpserted {
                        token: Some(token.clone()),
                    }),
                    Vec::new(),
                ));
                before.push(Before {
                    index: w.index,
                    token_id: token.token_id.clone(),
                    token: Some(w.before),
                });
                self.tokens[w.index] = token;
            }
        }
        Ok(Group {
            events,
            undo: Some(UndoRecord {
                moved: false,
                before,
            }),
        })
    }
}
