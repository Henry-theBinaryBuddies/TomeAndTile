//! Per-viewer filtering. Pure. EVERY message that leaves the server goes
//! through [`Campaign::snapshot_for`] or [`Campaign::event_for`].
//!
//! Where the contract is silent the more restrictive choice is made; the list
//! is in the crate README.

use tt_contracts::v1::{event, Creature, Encounter, Event, Role, RollVisibility, Snapshot, Token};

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

/// A copy of `token` with the fields `viewer` may not see left unset.
/// There is no reveal mechanism in the contract yet, so hp is for the GM and
/// controllers only.
fn redact_token(token: &Token, viewer: &Viewer) -> Token {
    let mut t = token.clone();
    if !viewer.controls(token) {
        t.hp_current = None;
        t.hp_max = None;
        t.hp_temp = None;
    }
    t
}

impl Campaign {
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
    fn encounter_visible(&self, encounter: &Encounter, viewer: &Viewer) -> bool {
        viewer.is_gm()
            || encounter
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
                .map(|t| redact_token(t, viewer))
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
                        token: Some(redact_token(token, viewer)),
                    },
                ));
                return Some(out);
            }
            // The token may already be gone, so its hidden flag is unknown:
            // GM only until a removal intent carries that information.
            event::Kind::TokenRemoved(_) => gm,
            // Unknown or hidden tokens: GM only.
            event::Kind::TokenMoved(m) => {
                gm || self.token_by_id(&m.token_id).is_some_and(|t| !t.hidden)
            }
            event::Kind::ChatPosted(_) => true,
            // Only an explicit PUBLIC roll is shared.
            event::Kind::DiceRolled(d) => {
                gm || d
                    .roll
                    .as_ref()
                    .is_some_and(|r| r.visibility == RollVisibility::Public as i32)
            }
            event::Kind::EncounterUpdated(u) => {
                gm || u
                    .encounter
                    .as_ref()
                    .is_some_and(|e| self.encounter_visible(e, viewer))
            }
            event::Kind::EncounterEnded(_) => true,
            // Not emitted in v0 and their payloads are not yet filterable.
            event::Kind::ResolutionUpdated(_) | event::Kind::EventUndone(_) => gm,
            event::Kind::CreatureUpserted(c) => c
                .creature
                .as_ref()
                .is_some_and(|c| self.creature_visible(c, viewer)),
            event::Kind::SceneUpserted(_) => true,
        };
        allowed.then(|| event.clone())
    }
}
