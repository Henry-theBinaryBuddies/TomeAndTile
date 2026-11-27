//! Per-viewer filtering. Pure. EVERY message that leaves the server goes
//! through [`Campaign::snapshot_for`] or [`Campaign::event_for`].
//!
//! Where the contract is silent the more restrictive choice is made; the list
//! is in the crate README.

use tt_contracts::v1::{
    event, Creature, Encounter, Event, Role, RollResult, RollVisibility, Snapshot, Token, Warning,
};

use crate::campaign::Campaign;

/// Who is looking: a campaign member and their role.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Viewer {
    pub user_id: String,
    pub role: Role,
}

impl Viewer {
    fn is_gm(&self) -> bool {
        self.role == Role::Gm
    }

    /// Players (not spectators) listed on a token control it.
    fn controls(&self, token: &Token) -> bool {
        match self.role {
            Role::Gm => true,
            Role::Player => token.controller_user_ids.contains(&self.user_id),
            _ => false,
        }
    }
}

impl Campaign {
    /// A copy of `token` with the fields `viewer` may not see left unset.
    /// There is no reveal mechanism in the contract yet, so hp is for the GM
    /// and controllers only. Condition timers keep `ends_round` and
    /// `at_end_of_turn`; for a non-GM viewer `source_token_id` is blanked
    /// when that token is hidden or gone, and `encounter_id` when the
    /// encounter is not visible to them or gone (#133), so neither can
    /// reveal a hidden combatant or where it sits in the turn order.
    fn redact_token(&self, token: &Token, viewer: &Viewer) -> Token {
        let mut t = token.clone();
        if !viewer.controls(token) {
            t.hp_current = None;
            t.hp_max = None;
            t.hp_temp = None;
        }
        if !viewer.is_gm() {
            for timer in &mut t.condition_timers {
                if !self
                    .token_by_id(&timer.source_token_id)
                    .is_some_and(|s| !s.hidden)
                {
                    timer.source_token_id.clear();
                }
                if !self
                    .encounters
                    .iter()
                    .find(|e| e.encounter_id == timer.encounter_id)
                    .is_some_and(|e| self.encounter_visible(e, viewer))
                {
                    timer.encounter_id.clear();
                }
            }
        }
        t
    }

    /// May this viewer see the sheet of the creature behind `token_id`? A
    /// token that is unknown, or has no sheet, is not.
    fn token_sheet_visible(&self, token_id: &str, viewer: &Viewer) -> bool {
        self.token_by_id(token_id)
            .and_then(|t| {
                self.creatures
                    .iter()
                    .find(|c| c.creature_id == t.creature_id)
            })
            .is_some_and(|c| self.creature_visible(c, viewer))
    }

    /// Clear what a roll made by a creature whose sheet `viewer` may not see
    /// would show of that sheet: dice, modifiers and notation (the total plus
    /// the die would give the modifier away). Everything else stays.
    fn redact_roll(&self, roll: &mut RollResult, viewer: &Viewer) {
        if roll.roller_creature_id.is_empty() {
            return;
        }
        let visible = self
            .creatures
            .iter()
            .find(|c| c.creature_id == roll.roller_creature_id)
            .is_some_and(|c| self.creature_visible(c, viewer));
        if !visible {
            roll.dice.clear();
            roll.modifiers.clear();
            roll.notation.clear();
        }
    }

    /// The result-card rules of #133 for a non-GM viewer, applied to the
    /// resolution inside `out` and to `out.warnings`. See the crate README.
    fn redact_resolution_event(&self, out: &mut Event, viewer: &Viewer) {
        let Some(event::Kind::ResolutionUpdated(ru)) = out.kind.as_mut() else {
            return;
        };
        let Some(res) = ru.resolution.as_mut() else {
            return;
        };
        let source_visible = self.token_sheet_visible(&res.source_token_id, viewer);
        for outcome in &mut res.outcomes {
            for roll in outcome
                .roll
                .iter_mut()
                .chain(outcome.damage_rolls.iter_mut())
            {
                self.redact_roll(roll, viewer);
            }
            // `applied` against `rolled` shows resistances and immunities;
            // a token with no sheet has none to show.
            let has_sheet = self
                .token_by_id(&outcome.token_id)
                .is_some_and(|t| !t.creature_id.is_empty());
            if has_sheet && !self.token_sheet_visible(&outcome.token_id, viewer) {
                outcome.damage.clear();
            }
        }
        // Warnings about the source's sheet need the source's sheet;
        // `condition_immune` names a target token and needs that target's.
        let outcomes = res.outcomes.clone();
        let keep = |w: &Warning| match w.code.as_str() {
            "not_enough_resource" | "too_many_targets" | "out_of_range" | "ability_recharging" => {
                source_visible
            }
            "condition_immune" => outcomes
                .iter()
                .find(|o| w.message.starts_with(&format!("token {} ", o.token_id)))
                .is_some_and(|o| self.token_sheet_visible(&o.token_id, viewer)),
            _ => true,
        };
        res.warnings.retain(keep);
        out.warnings.retain(keep);
    }

    fn token_by_id(&self, token_id: &str) -> Option<&Token> {
        self.tokens.iter().find(|t| t.token_id == token_id)
    }

    /// May this viewer receive the full sheet of `creature`? The GM; the
    /// owner; and controllers of a visible token linked to it. Spectators
    /// never.
    fn creature_visible(&self, creature: &Creature, viewer: &Viewer) -> bool {
        match viewer.role {
            Role::Gm => true,
            Role::Player => {
                (!creature.owner_user_id.is_empty() && creature.owner_user_id == viewer.user_id)
                    || self.tokens.iter().any(|t| {
                        !t.hidden
                            && t.creature_id == creature.creature_id
                            && !t.creature_id.is_empty()
                            && viewer.controls(t)
                    })
            }
            _ => false,
        }
    }

    /// Non-GM viewers see an encounter only if none of its combatants is a
    /// hidden (or unknown) token, so the turn order cannot reveal one.
    pub(crate) fn encounter_visible(&self, encounter: &Encounter, viewer: &Viewer) -> bool {
        viewer.is_gm() || self.encounter_visible_to_players(encounter)
    }

    /// The non-GM answer of [`Self::encounter_visible`]; it does not depend
    /// on which player.
    pub(crate) fn encounter_visible_to_players(&self, encounter: &Encounter) -> bool {
        encounter
            .combatants
            .iter()
            .all(|c| self.token_by_id(&c.token_id).is_some_and(|t| !t.hidden))
    }

    /// Everything `viewer` may see, as of the current `seq`.
    pub fn snapshot_for(&self, viewer: &Viewer) -> Snapshot {
        Snapshot {
            seq: self.seq,
            user_id: viewer.user_id.clone(),
            role: viewer.role as i32,
            members: self.members.clone(),
            scenes: self.scenes.clone(),
            tokens: self
                .tokens
                .iter()
                .filter(|t| viewer.is_gm() || !t.hidden)
                .map(|t| self.redact_token(t, viewer))
                .collect(),
            creatures: self
                .creatures
                .iter()
                .filter(|c| self.creature_visible(c, viewer))
                .cloned()
                .collect(),
            encounters: self
                .encounters
                .iter()
                .filter(|e| self.encounter_visible(e, viewer))
                .cloned()
                .collect(),
            active_scene_id: self.active_scene_id.clone(),
        }
    }

    /// May `viewer` receive `original`, an event that has been undone? As
    /// `event_for` with the current state, except a `TokenRemoved` whose token
    /// undo has put back: that is judged by the restored token, because the
    /// hidden flag kept for removed tokens is deleted on restore.
    fn undone_original_visible(&self, original: &Event, viewer: &Viewer) -> bool {
        if let Some(event::Kind::TokenRemoved(r)) = original.kind.as_ref() {
            if let Some(t) = self.token_by_id(&r.token_id) {
                return viewer.is_gm() || !t.hidden;
            }
        }
        self.event_for(original, viewer).is_some()
    }

    /// The version of `event` this viewer may receive, or `None`.
    pub fn event_for(&self, event: &Event, viewer: &Viewer) -> Option<Event> {
        let gm = viewer.is_gm();
        let allowed = match event.kind.as_ref()? {
            event::Kind::TokenUpserted(u) => {
                let token = u.token.as_ref()?;
                if token.hidden && !gm {
                    return None;
                }
                let mut out = event.clone();
                out.kind = Some(event::Kind::TokenUpserted(
                    tt_contracts::v1::TokenUpserted {
                        token: Some(self.redact_token(token, viewer)),
                    },
                ));
                // Warnings on a token event (`condition_immune`) describe the
                // creature's sheet: only for viewers who may see it.
                if !gm && !self.token_sheet_visible(&token.token_id, viewer) {
                    out.warnings.clear();
                }
                return Some(out);
            }
            // The token is gone, so the campaign kept whether it was hidden:
            // everyone for a visible token, the GM otherwise (also for an id
            // that was never removed through an intent).
            event::Kind::TokenRemoved(r) => {
                gm || self.removed_token_hidden.get(&r.token_id) == Some(&false)
            }
            // Unknown or hidden tokens: GM only.
            event::Kind::TokenMoved(m) => {
                gm || self.token_by_id(&m.token_id).is_some_and(|t| !t.hidden)
            }
            event::Kind::ChatPosted(_) => true,
            // Only an explicit PUBLIC roll is shared.
            event::Kind::DiceRolled(d) => {
                if gm {
                    true
                } else {
                    let r = d.roll.as_ref()?;
                    if r.visibility != RollVisibility::Public as i32 {
                        return None;
                    }
                    // A public roll of a creature the viewer cannot see (a
                    // monster's initiative) loses its dice and modifiers.
                    let mut out = event.clone();
                    if let Some(event::Kind::DiceRolled(dr)) = out.kind.as_mut() {
                        if let Some(roll) = dr.roll.as_mut() {
                            self.redact_roll(roll, viewer);
                        }
                    }
                    return Some(out);
                }
            }
            event::Kind::EncounterUpdated(u) => {
                gm || u
                    .encounter
                    .as_ref()
                    .is_some_and(|e| self.encounter_visible(e, viewer))
            }
            // Like `EncounterUpdated`: the campaign kept, when the encounter
            // ended, whether players could see it. Unknown ids: GM only.
            event::Kind::EncounterEnded(e) => {
                gm || self.removed_encounter_visible.get(&e.encounter_id) == Some(&true)
            }
            // Player rolls are public (ADR 0006): everyone sees a resolution
            // unless its source or any target token is hidden (or unknown).
            event::Kind::ResolutionUpdated(u) => {
                if gm {
                    true
                } else {
                    let r = u.resolution.as_ref()?;
                    let visible = |id: &str| self.token_by_id(id).is_some_and(|t| !t.hidden);
                    if !(visible(&r.source_token_id)
                        && r.outcomes.iter().all(|o| visible(&o.token_id)))
                    {
                        return None;
                    }
                    let mut out = event.clone();
                    self.redact_resolution_event(&mut out, viewer);
                    return Some(out);
                }
            }
            // Exactly the viewers who may receive the undone original,
            // filtered with the current state. An original not in the log
            // is GM only.
            event::Kind::EventUndone(u) => {
                gm || self
                    .log
                    .binary_search_by_key(&u.undone_seq, |e| e.seq)
                    .ok()
                    .is_some_and(|i| self.undone_original_visible(&self.log[i], viewer))
            }
            event::Kind::CreatureUpserted(c) => c
                .creature
                .as_ref()
                .is_some_and(|c| self.creature_visible(c, viewer)),
            event::Kind::SceneUpserted(_) => true,
        };
        allowed.then(|| event.clone())
    }
}
