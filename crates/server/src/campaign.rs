//! The pure core: campaign state and the one place it changes.
//!
//! No sockets, no clock, no global state. Time, randomness and id minting are
//! passed to [`Campaign::apply`]. A rejected intent leaves state, `seq` and
//! the log untouched: every handler validates first and mutates last.

use std::collections::HashMap;

use tt_contracts::v1::{
    effect_step, event, intent, AssetRef, ChatPosted, Combatant, CreateTokenIntent, Creature,
    CreatureSize, CreatureUpserted, DiceRolled, Encounter, EncounterEnded, EncounterUpdated,
    EndEncounterIntent, EndTurnIntent, Event, EventUndone, HitOutcome, Intent, Member, Modifier,
    MoveTokenIntent, RemoveTokenIntent, ResolutionState, ResolutionUpdated, Role, RollDiceIntent,
    RollMode, RollPurpose, RollResult, RollVisibility, Scene, SceneUpserted, SetConditionsIntent,
    SetControllersIntent, SetHitPointsIntent, SetInitiativeIntent, SetLevelBackgroundIntent,
    Snapshot, StartEncounterIntent, Token, TokenMoved, TokenRemoved, TokenUpserted, UndoIntent,
    UseAbilityIntent, Vec2, Warning,
};
use tt_rules::ability::{resolve_ability, ResolveError};
use tt_rules::conditions::expire_timers;
use tt_rules::damage::health_state;
use tt_rules::dice::{self, Expr};

use crate::ids::IdSource;
use crate::rng::RollSource;

/// Longest chat text, in characters.
pub const MAX_CHAT_CHARS: usize = 2000;
/// Longest dice label, in characters.
pub const MAX_LABEL_CHARS: usize = 120;
/// Most waypoints in one move.
pub const MAX_PATH_POINTS: usize = 256;
/// Largest absolute coordinate accepted, in scene units. Keeps distance
/// arithmetic far from overflow.
pub const MAX_COORDINATE: f32 = 1_000_000.0;
/// Most tokens in a campaign.
pub const MAX_TOKENS: usize = 500;
/// Longest token name, in characters.
pub const MAX_TOKEN_NAME_CHARS: usize = 120;
/// Most conditions on one token.
pub const MAX_CONDITIONS: usize = 32;
/// Largest temporary hit point value accepted.
pub const MAX_TEMP_HP: i32 = 999;

/// Most creatures one campaign may hold.
pub const MAX_CREATURES: usize = 500;
/// Longest imported creature name, in characters.
pub const MAX_CREATURE_NAME_CHARS: usize = 120;
/// Most abilities on one imported creature.
pub const MAX_ABILITIES: usize = 200;
/// Most resources on one imported creature.
pub const MAX_RESOURCES: usize = 50;

/// An intent refused outright. `code` is a stable snake_case string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rejection {
    pub code: &'static str,
    pub message: String,
}

impl Rejection {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    fn not_permitted() -> Self {
        Self::new("not_permitted", "you are not allowed to do that")
    }
}

/// What kind of intent produced a group; decides whether undo may take it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupKind {
    /// A move: undone with a `TokenMoved` back to the earlier place.
    Move,
    /// Ability use, set HP, set conditions, create or remove a token.
    Edit,
    /// Chat, dice, set controllers, set level background, encounters.
    NotUndoable,
    /// An undo group: never undone itself (no redo in M3).
    Undo,
}

/// What one applied intent changed, kept next to the log for undo (#101).
///
/// `first_seq..=last_seq` are the intent's events (consecutive, the same
/// `client_intent_id`). The id lists are found by comparing the stored
/// tokens and creatures before and after the handler ran, so they hold every
/// token or creature that was changed, created or removed, whatever the
/// intent: each id once, in the order of `Campaign::tokens` after the change
/// (removed ids last). Undo's dependency rule: a group cannot be undone while
/// a later, not undone group lists any of the same ids.
///
/// For an undoable group `before_tokens` and `before_creatures` hold the
/// whole stored value, as it was before the intent, of each listed id that
/// existed then (a removed token is kept whole). An id created by the group
/// has no entry. They stay empty for groups that cannot be undone.
#[derive(Debug, Clone, PartialEq)]
pub struct AppliedGroup {
    pub first_seq: u64,
    pub last_seq: u64,
    pub actor_user_id: String,
    pub client_intent_id: String,
    pub changed_token_ids: Vec<String>,
    pub changed_creature_ids: Vec<String>,
    pub kind: GroupKind,
    pub before_tokens: Vec<Token>,
    pub before_creatures: Vec<Creature>,
    /// True once an undo group has reversed this one.
    pub undone: bool,
}

/// All state of one campaign, held in memory.
#[derive(Debug, Clone, Default)]
pub struct Campaign {
    pub campaign_id: String,
    pub members: Vec<Member>,
    pub scenes: Vec<Scene>,
    pub tokens: Vec<Token>,
    pub creatures: Vec<Creature>,
    pub encounters: Vec<Encounter>,
    pub active_scene_id: String,
    /// Uploads this server issued (ADR 0014), by `asset_id`. Not part of the
    /// snapshot; only these may be put on a level.
    pub assets: HashMap<String, AssetRef>,
    /// Sequence number of the last event; 0 before any.
    pub seq: u64,
    /// Append-only event log, unfiltered (server-side only).
    pub log: Vec<Event>,
    /// One entry per applied intent, in `seq` order; see [`AppliedGroup`].
    pub groups: Vec<AppliedGroup>,
    /// For each token removed by `RemoveTokenIntent`: whether it was hidden.
    /// The token is gone when `TokenRemoved` is filtered, so this keeps the
    /// answer (`view.rs`). Undo (#101) must delete the entry when it puts a
    /// token back.
    pub removed_token_hidden: HashMap<String, bool>,
}

/// What [`Campaign::import_creature`] did.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportOutcome {
    /// The appended event, to broadcast.
    pub event: Event,
    /// The sheet as stored, with its server id.
    pub creature: Creature,
    /// True if it replaced an earlier import.
    pub replaced: bool,
}

/// A handler's result: the scene the events belong to and the event bodies.
type Outcome = Result<(String, Vec<event::Kind>), Rejection>;

/// Like [`Outcome`], plus warnings to attach to the FIRST event (a warning
/// never stops the action).
type WarnedOutcome = Result<(String, Vec<event::Kind>, Vec<Warning>), Rejection>;

impl Campaign {
    /// Build a campaign from a full (unfiltered) snapshot.
    pub fn from_snapshot(snapshot: Snapshot) -> Self {
        let campaign_id = snapshot
            .scenes
            .iter()
            .map(|s| s.campaign_id.clone())
            .find(|id| !id.is_empty())
            .unwrap_or_default();
        let mut tokens = snapshot.tokens;
        // Never trust a stored health state: recompute it for every token
        // with hit points.
        for t in &mut tokens {
            if let Some(max) = t.hp_max {
                t.health_state = health_state(t.hp_current.unwrap_or(max), max) as i32;
            }
        }
        Self {
            campaign_id,
            members: snapshot.members,
            scenes: snapshot.scenes,
            tokens,
            creatures: snapshot.creatures,
            encounters: snapshot.encounters,
            active_scene_id: snapshot.active_scene_id,
            assets: HashMap::new(),
            seq: snapshot.seq,
            log: Vec::new(),
            groups: Vec::new(),
            removed_token_hidden: HashMap::new(),
        }
    }

    /// Record an upload so a `SetLevelBackground` intent may name it. An id
    /// already registered keeps its first record.
    pub fn register_asset(&mut self, asset: AssetRef) {
        self.assets.entry(asset.asset_id.clone()).or_insert(asset);
    }

    /// Add or replace a creature sheet that came from an import (ADR 0015),
    /// as the GM `actor_user_id`. Not an intent: the HTTP layer calls it
    /// after parsing the file. Order of refusals: `not_permitted`,
    /// `bad_creature` (see [`validate_import`]), `too_many_creatures`
    /// (500 sheets and this is not a re-import). The id is minted last.
    ///
    /// * The incoming `creature_id` is ignored: a re-import keeps the id of
    ///   the sheet it replaces, anything else gets a fresh one from `ids`.
    /// * A new sheet gets an empty `owner_user_id` (ownership is given later
    ///   through controllers); a re-import keeps the replaced sheet's owner,
    ///   so an import never undoes ownership the GM set.
    /// * A re-import matches on a non-empty `provenance.source` AND a
    ///   non-empty `provenance.external_id`. Tokens linked to the sheet keep
    ///   their own hit points.
    ///
    /// Appends one `CreatureUpserted` (empty `client_intent_id`, the active
    /// scene, `warnings` on it) to the log. The group recorded for it is
    /// `NotUndoable`: an import cannot be undone, but it lists the creature
    /// so an earlier undoable change to the same sheet can no longer be
    /// undone past it.
    pub fn import_creature(
        &mut self,
        actor_user_id: &str,
        mut creature: Creature,
        warnings: Vec<Warning>,
        now_ms: i64,
        ids: &mut dyn IdSource,
    ) -> Result<ImportOutcome, Rejection> {
        if self.role_of(actor_user_id) != Some(Role::Gm) {
            return Err(Rejection::not_permitted());
        }
        validate_import(&creature)?;
        let provenance = creature.provenance.clone().unwrap_or_default();
        let existing = if provenance.external_id.is_empty() {
            None
        } else {
            self.creatures.iter().position(|c| {
                c.provenance.as_ref().is_some_and(|p| {
                    p.source == provenance.source && p.external_id == provenance.external_id
                })
            })
        };
        if existing.is_none() && self.creatures.len() >= MAX_CREATURES {
            return Err(Rejection::new(
                "too_many_creatures",
                format!("the campaign already has {MAX_CREATURES} creatures"),
            ));
        }

        // Validated: mint the id and mutate.
        creature.owner_user_id = existing
            .map(|i| self.creatures[i].owner_user_id.clone())
            .unwrap_or_default();
        let replaced = existing.is_some();
        creature.creature_id = match existing {
            Some(i) => self.creatures[i].creature_id.clone(),
            None => ids.next_id(now_ms),
        };
        match existing {
            Some(i) => self.creatures[i] = creature.clone(),
            None => self.creatures.push(creature.clone()),
        }
        self.seq += 1;
        let event = Event {
            seq: self.seq,
            at_ms: now_ms,
            scene_id: self.active_scene_id.clone(),
            actor_user_id: actor_user_id.to_string(),
            client_intent_id: String::new(),
            warnings,
            kind: Some(event::Kind::CreatureUpserted(CreatureUpserted {
                creature: Some(creature.clone()),
            })),
        };
        self.log.push(event.clone());
        self.groups.push(AppliedGroup {
            first_seq: self.seq,
            last_seq: self.seq,
            actor_user_id: actor_user_id.to_string(),
            client_intent_id: String::new(),
            changed_token_ids: Vec::new(),
            changed_creature_ids: vec![creature.creature_id.clone()],
            kind: GroupKind::NotUndoable,
            before_tokens: Vec::new(),
            before_creatures: Vec::new(),
            undone: false,
        });
        Ok(ImportOutcome {
            event,
            creature,
            replaced,
        })
    }

    /// The role of a member; `None` for a user not in this campaign or with
    /// an unrecognised role.
    pub fn role_of(&self, user_id: &str) -> Option<Role> {
        let member = self.members.iter().find(|m| m.user_id == user_id)?;
        match Role::try_from(member.role) {
            Ok(r @ (Role::Gm | Role::Player | Role::Spectator)) => Some(r),
            _ => None,
        }
    }

    /// Validate and apply one intent. On success returns the new events (also
    /// appended to the log) with strictly increasing `seq`.
    pub fn apply(
        &mut self,
        actor_user_id: &str,
        intent: Intent,
        now_ms: i64,
        rng: &mut dyn RollSource,
        ids: &mut dyn IdSource,
    ) -> Result<Vec<Event>, Rejection> {
        // Spectators, non-members and unknown roles may do nothing.
        let role = match self.role_of(actor_user_id) {
            Some(r @ (Role::Gm | Role::Player)) => r,
            _ => return Err(Rejection::not_permitted()),
        };
        let is_gm = role == Role::Gm;
        let client_intent_id = intent.client_intent_id;
        let Some(kind) = intent.kind else {
            return Err(Rejection::new("malformed_intent", "the intent has no kind"));
        };
        let tokens_before = self.tokens.clone();
        let creatures_before = self.creatures.clone();
        let group_kind = match &kind {
            intent::Kind::MoveToken(_) => GroupKind::Move,
            intent::Kind::UseAbility(_)
            | intent::Kind::SetHitPoints(_)
            | intent::Kind::SetConditions(_)
            | intent::Kind::CreateToken(_)
            | intent::Kind::RemoveToken(_) => GroupKind::Edit,
            intent::Kind::Undo(_) => GroupKind::Undo,
            _ => GroupKind::NotUndoable,
        };
        let (scene_id, kinds, warnings) = match kind {
            intent::Kind::MoveToken(m) => plain(self.move_token(actor_user_id, is_gm, m))?,
            intent::Kind::Chat(c) => plain(self.chat(actor_user_id, &c.text))?,
            intent::Kind::RollDice(r) => {
                plain(self.roll_dice(actor_user_id, is_gm, r, now_ms, rng, ids))?
            }
            intent::Kind::SetControllers(s) => plain(self.set_controllers(is_gm, s))?,
            intent::Kind::SetLevelBackground(b) => plain(self.set_level_background(is_gm, b))?,
            intent::Kind::UseAbility(u) => {
                self.use_ability(actor_user_id, is_gm, u, now_ms, rng, ids)?
            }
            intent::Kind::SetHitPoints(h) => plain(self.set_hit_points(actor_user_id, is_gm, h))?,
            intent::Kind::SetConditions(c) => self.set_conditions(is_gm, c)?,
            intent::Kind::CreateToken(t) => plain(self.create_token(is_gm, t, now_ms, ids))?,
            intent::Kind::RemoveToken(r) => plain(self.remove_token(is_gm, r))?,
            intent::Kind::Undo(u) => plain(self.undo(is_gm, u))?,
            intent::Kind::StartEncounter(s) => {
                plain(self.start_encounter(actor_user_id, is_gm, s, now_ms, rng, ids))?
            }
            intent::Kind::EndTurn(e) => {
                plain(self.end_turn(actor_user_id, is_gm, e, now_ms, rng, ids))?
            }
            intent::Kind::SetInitiative(s) => plain(self.set_initiative(is_gm, s))?,
            intent::Kind::EndEncounter(e) => plain(self.end_encounter(is_gm, e))?,
        };
        let mut events = Vec::with_capacity(kinds.len());
        let mut warnings = Some(warnings);
        let first_seq = self.seq + 1;
        for kind in kinds {
            self.seq += 1;
            let event = Event {
                seq: self.seq,
                at_ms: now_ms,
                scene_id: scene_id.clone(),
                actor_user_id: actor_user_id.to_string(),
                client_intent_id: client_intent_id.clone(),
                warnings: warnings.take().unwrap_or_default(),
                kind: Some(kind),
            };
            self.log.push(event.clone());
            events.push(event);
        }
        let changed_token_ids = changed_ids(&tokens_before, &self.tokens, |t| &t.token_id);
        let changed_creature_ids =
            changed_ids(&creatures_before, &self.creatures, |c| &c.creature_id);
        let undoable = matches!(group_kind, GroupKind::Move | GroupKind::Edit);
        let (before_tokens, before_creatures) = if undoable {
            (
                tokens_before
                    .iter()
                    .filter(|t| changed_token_ids.contains(&t.token_id))
                    .cloned()
                    .collect(),
                creatures_before
                    .iter()
                    .filter(|c| changed_creature_ids.contains(&c.creature_id))
                    .cloned()
                    .collect(),
            )
        } else {
            (Vec::new(), Vec::new())
        };
        self.groups.push(AppliedGroup {
            first_seq,
            last_seq: self.seq,
            actor_user_id: actor_user_id.to_string(),
            client_intent_id,
            changed_token_ids,
            changed_creature_ids,
            kind: group_kind,
            before_tokens,
            before_creatures,
            undone: false,
        });
        Ok(events)
    }

    /// `UseAbilityIntent`: resolve an attack ability with the rules engine.
    /// Order of refusals: permission (a player naming a missing, hidden or
    /// uncontrolled source gets `not_permitted`, so no GM creature's abilities
    /// leak), `unknown_token` (GM), `no_creature`, `unknown_ability`,
    /// `unknown_target`, then the engine's `no_targets` / `not_implemented`,
    /// then `rng_unavailable`. A dice error in the ability's own data is
    /// reported as `not_implemented`: the client's request was fine.
    fn use_ability(
        &mut self,
        actor: &str,
        is_gm: bool,
        u: UseAbilityIntent,
        now_ms: i64,
        rng: &mut dyn RollSource,
        ids: &mut dyn IdSource,
    ) -> WarnedOutcome {
        let source_token = match self.tokens.iter().find(|t| t.token_id == u.source_token_id) {
            Some(t) if is_gm || (!t.hidden && t.controller_user_ids.iter().any(|c| c == actor)) => {
                t.clone()
            }
            Some(_) => return Err(Rejection::not_permitted()),
            None if is_gm => {
                return Err(Rejection::new(
                    "unknown_token",
                    format!("no token {:?}", u.source_token_id),
                ))
            }
            None => return Err(Rejection::not_permitted()),
        };
        let creature_idx = self.creatures.iter().position(|c| {
            !source_token.creature_id.is_empty() && c.creature_id == source_token.creature_id
        });
        let Some(creature_idx) = creature_idx else {
            return Err(Rejection::new(
                "no_creature",
                "the source token has no creature",
            ));
        };
        let source = self.creatures[creature_idx].clone();
        if !source
            .abilities
            .iter()
            .any(|a| a.ability_id == u.ability_id)
        {
            return Err(Rejection::new(
                "unknown_ability",
                format!("the creature has no ability {:?}", u.ability_id),
            ));
        }
        // Targets, in the order listed. A hidden token is unknown to a player.
        let mut targets: Vec<(Token, Creature)> = Vec::with_capacity(u.target_token_ids.len());
        for id in &u.target_token_ids {
            let pair = self
                .tokens
                .iter()
                .find(|t| &t.token_id == id && (is_gm || !t.hidden))
                .filter(|t| !t.creature_id.is_empty())
                .and_then(|t| {
                    let c = self
                        .creatures
                        .iter()
                        .find(|c| c.creature_id == t.creature_id)?;
                    Some((t.clone(), c.clone()))
                });
            let Some(pair) = pair else {
                return Err(Rejection::new(
                    "unknown_target",
                    format!("token {id:?} does not exist or has no creature"),
                ));
            };
            targets.push(pair);
        }
        let grid = self
            .scenes
            .iter()
            .find(|s| s.scene_id == source_token.scene_id)
            .and_then(|s| s.grid)
            .unwrap_or_default();
        // The running encounter on the source token's scene, if any.
        let encounter = self
            .encounters
            .iter()
            .find(|e| e.scene_id == source_token.scene_id);
        let turn = encounter.map(|e| (e.encounter_id.as_str(), e.round));
        let mode = RollMode::try_from(u.mode).unwrap_or(RollMode::Unspecified);
        let resolved = resolve_ability(
            &source_token,
            &source,
            &u.ability_id,
            &targets,
            &grid,
            mode,
            turn,
            rng,
        )
        .map_err(|e| match e {
            ResolveError::UnknownAbility => Rejection::new("unknown_ability", e.to_string()),
            ResolveError::NoTargets => Rejection::new("no_targets", e.to_string()),
            ResolveError::NotImplemented | ResolveError::Dice(_) => {
                Rejection::new("not_implemented", e.to_string())
            }
        })?;
        // Checked after rolling and before any state change: a failed
        // entropy read discards the whole use.
        if rng.take_failure() {
            return Err(Rejection::new(
                "rng_unavailable",
                "the server could not get secure randomness; try again",
            ));
        }

        // Validated: fill ids and attribution, then mutate.
        let mut resolution = resolved.resolution;
        resolution.resolution_id = ids.next_id(now_ms);
        resolution.state = ResolutionState::Done as i32;
        // The engine copies one damage roll of a save or flat step into every
        // outcome of that step; every copy gets the same id. Attack steps roll
        // per target, so equal-looking rolls there stay separate rolls.
        let mut shared: HashMap<(u32, usize), String> = HashMap::new();
        for outcome in &mut resolution.outcomes {
            let step = outcome.step_index;
            let step_shares = matches!(
                HitOutcome::try_from(outcome.outcome),
                Ok(HitOutcome::SaveFailed | HitOutcome::SaveSucceeded | HitOutcome::Automatic)
            );
            if let Some(roll) = outcome.roll.as_mut() {
                roll.roll_id = ids.next_id(now_ms);
            }
            for (n, roll) in outcome.damage_rolls.iter_mut().enumerate() {
                roll.roll_id = if step_shares {
                    shared
                        .entry((step, n))
                        .or_insert_with(|| ids.next_id(now_ms))
                        .clone()
                } else {
                    ids.next_id(now_ms)
                };
            }
            for roll in outcome
                .roll
                .iter_mut()
                .chain(outcome.damage_rolls.iter_mut())
            {
                roll.roller_user_id = actor.to_string();
                roll.visibility = RollVisibility::Public as i32;
            }
        }
        let warnings = resolution.warnings.clone();
        let mut kinds = vec![event::Kind::ResolutionUpdated(ResolutionUpdated {
            resolution: Some(resolution),
        })];
        for updated in resolved.targets {
            if let Some(stored) = self
                .tokens
                .iter_mut()
                .find(|t| t.token_id == updated.token_id)
            {
                if *stored != updated {
                    *stored = updated.clone();
                    kinds.push(event::Kind::TokenUpserted(TokenUpserted {
                        token: Some(updated),
                    }));
                }
            }
        }
        if resolved.source != source {
            self.creatures[creature_idx] = resolved.source.clone();
            kinds.push(event::Kind::CreatureUpserted(CreatureUpserted {
                creature: Some(resolved.source),
            }));
        }
        Ok((source_token.scene_id, kinds, warnings))
    }

    /// `SetHitPointsIntent`. Order of refusals: permission (a player naming a
    /// missing, hidden or uncontrolled token gets `not_permitted`),
    /// `unknown_token` (GM), `no_hit_points`, `bad_hit_points`. Sets the
    /// token only; `Creature.hp_*` is the stat block and never changes.
    fn set_hit_points(&mut self, actor: &str, is_gm: bool, h: SetHitPointsIntent) -> Outcome {
        let idx = match self.tokens.iter().position(|t| t.token_id == h.token_id) {
            Some(i) => {
                let t = &self.tokens[i];
                if !is_gm && (t.hidden || !t.controller_user_ids.iter().any(|u| u == actor)) {
                    return Err(Rejection::not_permitted());
                }
                i
            }
            None if is_gm => {
                return Err(Rejection::new(
                    "unknown_token",
                    format!("no token {:?}", h.token_id),
                ))
            }
            None => return Err(Rejection::not_permitted()),
        };
        let Some(max) = self.tokens[idx].hp_max.filter(|m| *m > 0) else {
            return Err(Rejection::new(
                "no_hit_points",
                "the token has no hit points to set",
            ));
        };
        if h.hp_current < 0 || h.hp_current > max {
            return Err(Rejection::new(
                "bad_hit_points",
                format!("hp_current must be from 0 to {max}"),
            ));
        }
        if h.hp_temp < 0 || h.hp_temp > MAX_TEMP_HP {
            return Err(Rejection::new(
                "bad_hit_points",
                format!("hp_temp must be from 0 to {MAX_TEMP_HP}"),
            ));
        }

        // Validated: mutate.
        let token = &mut self.tokens[idx];
        token.hp_current = Some(h.hp_current);
        token.hp_temp = Some(h.hp_temp);
        token.health_state = health_state(h.hp_current, max) as i32;
        let token = token.clone();
        Ok((
            token.scene_id.clone(),
            vec![event::Kind::TokenUpserted(TokenUpserted {
                token: Some(token),
            })],
        ))
    }

    /// `SetConditionsIntent` (GM only). Order of refusals: `not_permitted`,
    /// `unknown_token`, `bad_condition_id`, `conflicting_conditions`,
    /// `too_many_conditions`. An added id the creature is immune to is added
    /// anyway, with a `condition_immune` warning.
    fn set_conditions(&mut self, is_gm: bool, c: SetConditionsIntent) -> WarnedOutcome {
        if !is_gm {
            return Err(Rejection::not_permitted());
        }
        let Some(idx) = self.tokens.iter().position(|t| t.token_id == c.token_id) else {
            return Err(Rejection::new(
                "unknown_token",
                format!("no token {:?}", c.token_id),
            ));
        };
        if let Some(bad) = c
            .add
            .iter()
            .chain(&c.remove)
            .find(|id| !is_condition_id(id))
        {
            return Err(Rejection::new(
                "bad_condition_id",
                format!("{bad:?} is not a condition id ([a-z0-9_], 1 to 40 characters)"),
            ));
        }
        if let Some(both) = c.add.iter().find(|id| c.remove.contains(id)) {
            return Err(Rejection::new(
                "conflicting_conditions",
                format!("{both:?} is in both add and remove"),
            ));
        }
        let token = &self.tokens[idx];
        let mut conditions: Vec<String> = token
            .conditions
            .iter()
            .filter(|id| !c.remove.contains(id))
            .cloned()
            .collect();
        let mut added: Vec<&String> = Vec::new();
        for id in &c.add {
            if !added.contains(&id) {
                added.push(id);
            }
            if !conditions.contains(id) {
                conditions.push(id.clone());
            }
        }
        if conditions.len() > MAX_CONDITIONS {
            return Err(Rejection::new(
                "too_many_conditions",
                format!("a token may have at most {MAX_CONDITIONS} conditions"),
            ));
        }
        let immunities: &[String] = self
            .creatures
            .iter()
            .find(|cr| !token.creature_id.is_empty() && cr.creature_id == token.creature_id)
            .map_or(&[], |cr| &cr.condition_immunities);
        let warnings: Vec<Warning> = added
            .iter()
            .filter(|id| immunities.contains(**id))
            .map(|id| Warning {
                code: "condition_immune".into(),
                message: format!("the creature is immune to {id}; added anyway"),
            })
            .collect();

        // Validated: mutate.
        let token = &mut self.tokens[idx];
        token.conditions = conditions;
        let kept = token.conditions.clone();
        token
            .condition_timers
            .retain(|t| kept.contains(&t.effect_id));
        let token = token.clone();
        Ok((
            token.scene_id.clone(),
            vec![event::Kind::TokenUpserted(TokenUpserted {
                token: Some(token),
            })],
            warnings,
        ))
    }

    /// `CreateTokenIntent` (GM only). Order of refusals: `not_permitted`,
    /// `unknown_scene`, `unknown_level`, `unknown_creature`, `bad_coordinate`
    /// (also an unset position), `name_too_long`, `unknown_user`,
    /// `invalid_controller`, `too_many_tokens`. The id is minted last.
    fn create_token(
        &mut self,
        is_gm: bool,
        t: CreateTokenIntent,
        now_ms: i64,
        ids: &mut dyn IdSource,
    ) -> Outcome {
        if !is_gm {
            return Err(Rejection::not_permitted());
        }
        let Some(scene) = self.scenes.iter().find(|s| s.scene_id == t.scene_id) else {
            return Err(Rejection::new(
                "unknown_scene",
                format!("no scene {:?}", t.scene_id),
            ));
        };
        let level_id = if t.level_id.is_empty() {
            scene
                .levels
                .first()
                .map(|l| l.level_id.clone())
                .unwrap_or_default()
        } else if scene.levels.iter().any(|l| l.level_id == t.level_id) {
            t.level_id.clone()
        } else {
            return Err(Rejection::new(
                "unknown_level",
                format!("no level {:?} in this scene", t.level_id),
            ));
        };
        let creature = if t.creature_id.is_empty() {
            None
        } else {
            match self
                .creatures
                .iter()
                .find(|c| c.creature_id == t.creature_id)
            {
                Some(c) => Some(c.clone()),
                None => {
                    return Err(Rejection::new(
                        "unknown_creature",
                        format!("no creature {:?}", t.creature_id),
                    ))
                }
            }
        };
        let in_range = |v: f32| v.is_finite() && v.abs() <= MAX_COORDINATE;
        let Some(position) = t.position.filter(|p| in_range(p.x) && in_range(p.y)) else {
            return Err(Rejection::new(
                "bad_coordinate",
                format!("a position within {MAX_COORDINATE} units is required"),
            ));
        };
        if t.name.chars().count() > MAX_TOKEN_NAME_CHARS {
            return Err(Rejection::new(
                "name_too_long",
                format!("name is longer than {MAX_TOKEN_NAME_CHARS} characters"),
            ));
        }
        let mut controllers: Vec<String> = Vec::new();
        for user_id in t.controller_user_ids {
            match self.role_of(&user_id) {
                Some(Role::Gm | Role::Player) => {}
                Some(_) => {
                    return Err(Rejection::new(
                        "invalid_controller",
                        format!("{user_id:?} cannot control tokens"),
                    ))
                }
                None => {
                    return Err(Rejection::new(
                        "unknown_user",
                        format!("{user_id:?} is not a member of this campaign"),
                    ))
                }
            }
            if !controllers.contains(&user_id) {
                controllers.push(user_id);
            }
        }
        if self.tokens.len() >= MAX_TOKENS {
            return Err(Rejection::new(
                "too_many_tokens",
                format!("the campaign already has {MAX_TOKENS} tokens"),
            ));
        }

        // Validated: mint the id and mutate.
        let name = if !t.name.is_empty() {
            t.name
        } else {
            creature
                .as_ref()
                .map_or_else(|| "Token".to_string(), |c| c.name.clone())
        };
        let size = match CreatureSize::try_from(t.size) {
            Ok(s) if s != CreatureSize::Unspecified => s,
            _ => creature
                .as_ref()
                .and_then(|c| CreatureSize::try_from(c.size).ok())
                .filter(|s| *s != CreatureSize::Unspecified)
                .unwrap_or(CreatureSize::Medium),
        };
        let mut token = Token {
            token_id: ids.next_id(now_ms),
            scene_id: t.scene_id.clone(),
            level_id,
            creature_id: t.creature_id,
            name,
            position: Some(position),
            size: size as i32,
            hidden: t.hidden,
            controller_user_ids: controllers.clone(),
            ..Default::default()
        };
        if let Some(c) = creature.as_ref().filter(|c| c.hp_max > 0) {
            token.hp_current = Some(c.hp_current);
            token.hp_max = Some(c.hp_max);
            token.hp_temp = Some(c.hp_temp);
            token.health_state = health_state(c.hp_current, c.hp_max) as i32;
        }
        self.tokens.push(token.clone());
        let mut kinds = vec![event::Kind::TokenUpserted(TokenUpserted {
            token: Some(token),
        })];
        // As set_controllers: the new controllers need the sheet; the filter
        // decides who gets it.
        if let (Some(creature), false) = (creature, controllers.is_empty()) {
            kinds.push(event::Kind::CreatureUpserted(CreatureUpserted {
                creature: Some(creature),
            }));
        }
        Ok((t.scene_id, kinds))
    }

    /// `RemoveTokenIntent` (GM only): `not_permitted`, `unknown_token`,
    /// `in_encounter`. The creature stays. Whether the token was hidden is
    /// kept in `removed_token_hidden` so the event can be filtered.
    fn remove_token(&mut self, is_gm: bool, r: RemoveTokenIntent) -> Outcome {
        if !is_gm {
            return Err(Rejection::not_permitted());
        }
        let Some(idx) = self.tokens.iter().position(|t| t.token_id == r.token_id) else {
            return Err(Rejection::new(
                "unknown_token",
                format!("no token {:?}", r.token_id),
            ));
        };
        if self
            .encounters
            .iter()
            .any(|e| e.combatants.iter().any(|c| c.token_id == r.token_id))
        {
            return Err(Rejection::new(
                "in_encounter",
                "the token is in a running encounter; end the encounter first",
            ));
        }

        // Validated: mutate.
        let token = self.tokens.remove(idx);
        self.removed_token_hidden
            .insert(token.token_id.clone(), token.hidden);
        Ok((
            token.scene_id,
            vec![event::Kind::TokenRemoved(TokenRemoved {
                token_id: token.token_id,
            })],
        ))
    }

    fn move_token(&mut self, actor: &str, is_gm: bool, m: MoveTokenIntent) -> Outcome {
        let idx = self.tokens.iter().position(|t| t.token_id == m.token_id);
        let idx = match idx {
            Some(i) => {
                let t = &self.tokens[i];
                // Players never move hidden tokens, so a hidden token behaves
                // exactly like a missing one to them.
                if !is_gm && (t.hidden || !t.controller_user_ids.iter().any(|u| u == actor)) {
                    return Err(Rejection::not_permitted());
                }
                i
            }
            None if is_gm => {
                return Err(Rejection::new(
                    "unknown_token",
                    format!("no token {:?}", m.token_id),
                ))
            }
            None => return Err(Rejection::not_permitted()),
        };
        if m.path.is_empty() {
            return Err(Rejection::new("empty_path", "path has no waypoints"));
        }
        if m.path.len() > MAX_PATH_POINTS {
            return Err(Rejection::new(
                "path_too_long",
                format!("path has more than {MAX_PATH_POINTS} waypoints"),
            ));
        }
        let in_range = |v: f32| v.is_finite() && v.abs() <= MAX_COORDINATE;
        if !m.path.iter().all(|p| in_range(p.x) && in_range(p.y)) {
            return Err(Rejection::new(
                "bad_coordinate",
                format!("coordinates must be finite and within {MAX_COORDINATE} units"),
            ));
        }
        let token = &self.tokens[idx];
        let scene = self.scenes.iter().find(|s| s.scene_id == token.scene_id);
        if !m.level_id.is_empty() {
            if let Some(scene) = scene {
                if !scene.levels.iter().any(|l| l.level_id == m.level_id) {
                    return Err(Rejection::new(
                        "unknown_level",
                        format!("no level {:?} in this scene", m.level_id),
                    ));
                }
            }
        }
        let (cell_size, feet_per_cell) = scene
            .and_then(|s| s.grid.as_ref())
            .map(|g| (g.cell_size, g.feet_per_cell))
            .unwrap_or((0.0, 0.0));
        let destination = m.path[m.path.len() - 1];
        let start = token.position.unwrap_or(m.path[0]);
        let distance_feet = path_distance_feet(&start, &m.path, cell_size, feet_per_cell);
        let scene_id = token.scene_id.clone();

        // Validated: mutate.
        let token = &mut self.tokens[idx];
        token.position = Some(destination);
        if !m.level_id.is_empty() {
            token.level_id = m.level_id.clone();
        }
        Ok((
            scene_id,
            vec![event::Kind::TokenMoved(TokenMoved {
                token_id: m.token_id,
                path: m.path,
                level_id: m.level_id,
                distance_feet,
            })],
        ))
    }

    /// `UndoIntent` (GM only): reverse one earlier group with compensating
    /// events. Order of refusals: `not_permitted`, `unknown_event`,
    /// `not_first_event`, `not_undoable`, `already_undone`, `undo_blocked`.
    ///
    /// A later group blocks the undo when it has not been undone and lists
    /// one of the same tokens or creatures. Undo groups never block: each one
    /// restored exactly the state of the group it reversed, so with that
    /// group marked undone the pair changes nothing for earlier groups.
    fn undo(&mut self, is_gm: bool, u: UndoIntent) -> Outcome {
        if !is_gm {
            return Err(Rejection::not_permitted());
        }
        let seq = u.event_seq;
        let Some(gi) = self
            .groups
            .iter()
            .position(|g| g.first_seq <= seq && seq <= g.last_seq)
        else {
            return Err(Rejection::new(
                "unknown_event",
                format!("no event with seq {seq}"),
            ));
        };
        let g = &self.groups[gi];
        if g.first_seq != seq {
            return Err(Rejection::new(
                "not_first_event",
                format!(
                    "event {seq} is part of the group that starts at seq {}; name that one",
                    g.first_seq
                ),
            ));
        }
        if !matches!(g.kind, GroupKind::Move | GroupKind::Edit) {
            return Err(Rejection::new(
                "not_undoable",
                "only an ability use, set hit points, set conditions, move, token create or token remove can be undone",
            ));
        }
        if g.undone {
            return Err(Rejection::new(
                "already_undone",
                format!("the group at seq {seq} was already undone"),
            ));
        }
        let shares = |l: &AppliedGroup| {
            l.changed_token_ids
                .iter()
                .any(|id| g.changed_token_ids.contains(id))
                || l.changed_creature_ids
                    .iter()
                    .any(|id| g.changed_creature_ids.contains(id))
        };
        if let Some(later) = self.groups[gi + 1..]
            .iter()
            .find(|l| !l.undone && l.kind != GroupKind::Undo && shares(l))
        {
            return Err(Rejection::new(
                "undo_blocked",
                format!(
                    "the group at seq {} changed the same tokens or creatures; undo it first",
                    later.first_seq
                ),
            ));
        }

        // Validated: mutate.
        let g = self.groups[gi].clone();
        let scene_id = self
            .log
            .iter()
            .find(|e| e.seq == g.first_seq)
            .map(|e| e.scene_id.clone())
            .unwrap_or_default();
        // Sheets first, so a restored token's sheet event carries the state
        // the sheet has after the undo.
        for before in &g.before_creatures {
            match self
                .creatures
                .iter_mut()
                .find(|c| c.creature_id == before.creature_id)
            {
                Some(stored) => *stored = before.clone(),
                None => self.creatures.push(before.clone()),
            }
        }
        let mut kinds: Vec<event::Kind> = Vec::new();
        for id in &g.changed_token_ids {
            let current = self.tokens.iter().position(|t| &t.token_id == id);
            let before = g.before_tokens.iter().find(|t| &t.token_id == id);
            match (current, before) {
                // Created by the group: take it away again.
                (Some(idx), None) => {
                    let token = self.tokens.remove(idx);
                    self.removed_token_hidden
                        .insert(token.token_id.clone(), token.hidden);
                    kinds.push(event::Kind::TokenRemoved(TokenRemoved {
                        token_id: token.token_id,
                    }));
                }
                // Removed by the group: put it back whole.
                (None, Some(before)) => {
                    self.tokens.push(before.clone());
                    self.removed_token_hidden.remove(id);
                    kinds.push(event::Kind::TokenUpserted(TokenUpserted {
                        token: Some(before.clone()),
                    }));
                    let sheet = self.creatures.iter().find(|c| {
                        !before.creature_id.is_empty() && c.creature_id == before.creature_id
                    });
                    if let Some(sheet) = sheet {
                        // A sheet this undo restores is sent below anyway.
                        if !before.controller_user_ids.is_empty()
                            && !g.changed_creature_ids.contains(&sheet.creature_id)
                        {
                            kinds.push(event::Kind::CreatureUpserted(CreatureUpserted {
                                creature: Some(sheet.clone()),
                            }));
                        }
                    }
                }
                (Some(idx), Some(before)) => {
                    let moved_from = self.tokens[idx].position;
                    let compensation = match (g.kind, before.position) {
                        (GroupKind::Move, Some(to)) => {
                            let (cell, feet) = self
                                .scenes
                                .iter()
                                .find(|s| s.scene_id == before.scene_id)
                                .and_then(|s| s.grid.as_ref())
                                .map_or((0.0, 0.0), |gr| (gr.cell_size, gr.feet_per_cell));
                            let distance_feet = moved_from
                                .map_or(0.0, |f| path_distance_feet(&f, &[to], cell, feet));
                            event::Kind::TokenMoved(TokenMoved {
                                token_id: id.clone(),
                                path: vec![to],
                                level_id: before.level_id.clone(),
                                distance_feet,
                            })
                        }
                        _ => event::Kind::TokenUpserted(TokenUpserted {
                            token: Some(before.clone()),
                        }),
                    };
                    self.tokens[idx] = before.clone();
                    kinds.push(compensation);
                }
                (None, None) => {}
            }
        }
        for before in &g.before_creatures {
            kinds.push(event::Kind::CreatureUpserted(CreatureUpserted {
                creature: Some(before.clone()),
            }));
        }
        for undone_seq in g.first_seq..=g.last_seq {
            kinds.push(event::Kind::EventUndone(EventUndone { undone_seq }));
        }
        self.groups[gi].undone = true;
        Ok((scene_id, kinds))
    }

    /// `StartEncounterIntent` (GM only). Order of refusals: `not_permitted`,
    /// `unknown_scene`, `no_combatants`, `unknown_token` (missing, or on
    /// another scene), `encounter_running`, `rng_unavailable`. Duplicate ids
    /// are listed once. One d20 plus `initiative_bonus` per token (no creature:
    /// bonus 0), then `EncounterUpdated`: round 1, turn 0, sorted by
    /// initiative (higher first), then `initiative_bonus` (higher first), then
    /// list order. Ids are minted only after the entropy check. No turn-start
    /// effects run for the first combatant.
    fn start_encounter(
        &mut self,
        actor: &str,
        is_gm: bool,
        s: StartEncounterIntent,
        now_ms: i64,
        rng: &mut dyn RollSource,
        ids: &mut dyn IdSource,
    ) -> Outcome {
        if !is_gm {
            return Err(Rejection::not_permitted());
        }
        if !self.scenes.iter().any(|sc| sc.scene_id == s.scene_id) {
            return Err(Rejection::new(
                "unknown_scene",
                format!("no scene {:?}", s.scene_id),
            ));
        }
        let mut listed: Vec<&String> = Vec::new();
        for id in &s.token_ids {
            if !listed.contains(&id) {
                listed.push(id);
            }
        }
        if listed.is_empty() {
            return Err(Rejection::new(
                "no_combatants",
                "an encounter needs at least one token",
            ));
        }
        let mut tokens: Vec<&Token> = Vec::with_capacity(listed.len());
        for id in &listed {
            match self
                .tokens
                .iter()
                .find(|t| &&t.token_id == id && t.scene_id == s.scene_id)
            {
                Some(t) => tokens.push(t),
                None => {
                    return Err(Rejection::new(
                        "unknown_token",
                        format!("token {id:?} is not on scene {:?}", s.scene_id),
                    ))
                }
            }
        }
        if self.encounters.iter().any(|e| e.scene_id == s.scene_id) {
            return Err(Rejection::new(
                "encounter_running",
                "the scene already has a running encounter",
            ));
        }

        // Roll everything, then check entropy before any state change.
        let d20 = Expr::parse("1d20").expect("1d20 parses");
        let mut pending: Vec<(String, i32, RollResult)> = Vec::with_capacity(tokens.len());
        for t in &tokens {
            let creature = self
                .creatures
                .iter()
                .find(|c| !t.creature_id.is_empty() && c.creature_id == t.creature_id);
            let bonus = creature.map_or(0, |c| c.initiative_bonus);
            let rolled = dice::roll(&d20, RollMode::Normal, rng);
            let visibility = if t.hidden {
                RollVisibility::GmOnly
            } else {
                RollVisibility::Public
            };
            let roll = RollResult {
                roll_id: String::new(),
                notation: if bonus == 0 {
                    "1d20".to_string()
                } else {
                    format!("1d20{bonus:+}")
                },
                purpose: RollPurpose::Initiative as i32,
                mode: RollMode::Normal as i32,
                dice: rolled.dice,
                modifiers: if bonus == 0 {
                    Vec::new()
                } else {
                    vec![Modifier {
                        source: "initiative".into(),
                        value: bonus,
                    }]
                },
                total: rolled.total.saturating_add(bonus),
                critical: rolled.critical,
                fumble: rolled.fumble,
                visibility: visibility as i32,
                roller_user_id: actor.to_string(),
                roller_creature_id: creature.map_or_else(String::new, |c| c.creature_id.clone()),
                label: format!("{} initiative", t.name),
            };
            pending.push((t.token_id.clone(), bonus, roll));
        }
        if rng.take_failure() {
            return Err(Rejection::new(
                "rng_unavailable",
                "the server could not get secure randomness; try again",
            ));
        }

        // Validated: mint ids and mutate.
        let mut kinds = Vec::with_capacity(pending.len() + 1);
        let mut entries: Vec<(i32, i32, usize, String)> = Vec::with_capacity(pending.len());
        for (i, (token_id, bonus, mut roll)) in pending.into_iter().enumerate() {
            roll.roll_id = ids.next_id(now_ms);
            entries.push((roll.total, bonus, i, token_id));
            kinds.push(event::Kind::DiceRolled(DiceRolled { roll: Some(roll) }));
        }
        entries.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)).then(a.2.cmp(&b.2)));
        let encounter = Encounter {
            encounter_id: ids.next_id(now_ms),
            scene_id: s.scene_id.clone(),
            round: 1,
            turn_index: 0,
            combatants: entries
                .into_iter()
                .enumerate()
                .map(|(order, (initiative, _, _, token_id))| Combatant {
                    token_id,
                    initiative,
                    order: order as u32,
                    ..Default::default()
                })
                .collect(),
        };
        self.encounters.push(encounter.clone());
        kinds.push(event::Kind::EncounterUpdated(EncounterUpdated {
            encounter: Some(encounter),
        }));
        Ok((s.scene_id, kinds))
    }

    /// `EndTurnIntent` (GM, or a player controlling the current combatant's
    /// visible token). Order of refusals: `unknown_encounter` (GM) or
    /// `not_permitted` (a player naming a missing or invisible encounter, or
    /// not controlling the current token), then `rng_unavailable`. Events:
    /// `EncounterUpdated`, a `TokenUpserted` per token whose timers ended,
    /// then the recharge `DiceRolled`s and one `CreatureUpserted` of the
    /// creature whose turn starts.
    fn end_turn(
        &mut self,
        actor: &str,
        is_gm: bool,
        e: EndTurnIntent,
        now_ms: i64,
        rng: &mut dyn RollSource,
        ids: &mut dyn IdSource,
    ) -> Outcome {
        let viewer = crate::view::Viewer {
            user_id: actor.to_string(),
            role: if is_gm { Role::Gm } else { Role::Player },
        };
        let found = self
            .encounters
            .iter()
            .position(|x| x.encounter_id == e.encounter_id)
            .filter(|i| is_gm || self.encounter_visible(&self.encounters[*i], &viewer));
        let Some(ei) = found else {
            return Err(if is_gm {
                Rejection::new(
                    "unknown_encounter",
                    format!("no encounter {:?}", e.encounter_id),
                )
            } else {
                Rejection::not_permitted()
            });
        };
        let enc = self.encounters[ei].clone();
        let n = enc.combatants.len();
        let cur = enc.turn_index as usize;
        let current = enc.combatants.get(cur);
        if !is_gm {
            let allowed = current.is_some_and(|c| {
                self.tokens.iter().any(|t| {
                    t.token_id == c.token_id
                        && !t.hidden
                        && t.controller_user_ids.iter().any(|u| u == actor)
                })
            });
            if !allowed {
                return Err(Rejection::not_permitted());
            }
        }
        let Some(current) = current else {
            return Err(Rejection::new(
                "unknown_encounter",
                "the encounter has no current combatant",
            ));
        };
        let (next, round) = if cur + 1 >= n {
            (0, enc.round.saturating_add(1))
        } else {
            (cur + 1, enc.round)
        };
        let mut updated = enc.clone();
        updated.turn_index = next as u32;
        updated.round = round;
        let starter = updated.combatants[next].token_id.clone();
        let expired = expire_timers(
            &self.tokens,
            &updated,
            Some((current.token_id.as_str(), enc.round)),
            (starter.as_str(), round),
        );

        // Recharge: a d6 for each spent recharge ability of the creature
        // whose turn starts.
        let start_token = self.tokens.iter().find(|t| t.token_id == starter);
        let sheet_idx = start_token.and_then(|t| {
            self.creatures
                .iter()
                .position(|c| !t.creature_id.is_empty() && c.creature_id == t.creature_id)
        });
        let hidden = start_token.is_some_and(|t| t.hidden);
        let mut rolls: Vec<RollResult> = Vec::new();
        let mut recharged = sheet_idx.map(|i| self.creatures[i].clone());
        let mut changed = false;
        if let Some(sheet) = recharged.as_mut() {
            let d6 = Expr::parse("1d6").expect("1d6 parses");
            let visibility = if hidden || sheet.owner_user_id.is_empty() {
                RollVisibility::GmOnly
            } else {
                RollVisibility::Public
            };
            let creature_id = sheet.creature_id.clone();
            for ability in &mut sheet.abilities {
                let Some(min_roll) = ability
                    .recharge
                    .as_ref()
                    .filter(|_| ability.recharge_spent)
                    .map(|r| r.min_roll)
                else {
                    continue;
                };
                let rolled = dice::roll(&d6, RollMode::Normal, rng);
                if rolled.total >= i32::try_from(min_roll).unwrap_or(i32::MAX) {
                    ability.recharge_spent = false;
                    changed = true;
                }
                let name = if ability.name.is_empty() {
                    &ability.ability_id
                } else {
                    &ability.name
                };
                rolls.push(RollResult {
                    roll_id: String::new(),
                    notation: d6.canonical(),
                    purpose: RollPurpose::Freeform as i32,
                    mode: RollMode::Normal as i32,
                    dice: rolled.dice,
                    modifiers: Vec::new(),
                    total: rolled.total,
                    critical: false,
                    fumble: false,
                    visibility: visibility as i32,
                    roller_user_id: actor.to_string(),
                    roller_creature_id: creature_id.clone(),
                    label: format!("{name} recharge"),
                });
            }
        }
        if rng.take_failure() {
            return Err(Rejection::new(
                "rng_unavailable",
                "the server could not get secure randomness; try again",
            ));
        }

        // Validated: mutate.
        self.encounters[ei] = updated.clone();
        let mut kinds = vec![event::Kind::EncounterUpdated(EncounterUpdated {
            encounter: Some(updated),
        })];
        for token in expired {
            if let Some(stored) = self
                .tokens
                .iter_mut()
                .find(|t| t.token_id == token.token_id)
            {
                *stored = token.clone();
            }
            kinds.push(event::Kind::TokenUpserted(TokenUpserted {
                token: Some(token),
            }));
        }
        for mut roll in rolls {
            roll.roll_id = ids.next_id(now_ms);
            kinds.push(event::Kind::DiceRolled(DiceRolled { roll: Some(roll) }));
        }
        if let (true, Some(idx), Some(sheet)) = (changed, sheet_idx, recharged) {
            self.creatures[idx] = sheet.clone();
            kinds.push(event::Kind::CreatureUpserted(CreatureUpserted {
                creature: Some(sheet),
            }));
        }
        Ok((enc.scene_id, kinds))
    }

    /// `SetInitiativeIntent` (GM only): `not_permitted`, `unknown_encounter`,
    /// `unknown_combatant`. Sets `initiative` and `order`, re-sorts (higher
    /// initiative first, then lower `order`; equal keys keep their places)
    /// and keeps the turn with the combatant whose turn it was.
    fn set_initiative(&mut self, is_gm: bool, s: SetInitiativeIntent) -> Outcome {
        if !is_gm {
            return Err(Rejection::not_permitted());
        }
        let Some(ei) = self
            .encounters
            .iter()
            .position(|x| x.encounter_id == s.encounter_id)
        else {
            return Err(Rejection::new(
                "unknown_encounter",
                format!("no encounter {:?}", s.encounter_id),
            ));
        };
        let Some(ci) = self.encounters[ei]
            .combatants
            .iter()
            .position(|c| c.token_id == s.token_id)
        else {
            return Err(Rejection::new(
                "unknown_combatant",
                format!("token {:?} is not in the encounter", s.token_id),
            ));
        };

        // Validated: mutate.
        let enc = &mut self.encounters[ei];
        enc.combatants[ci].initiative = s.initiative;
        enc.combatants[ci].order = s.order;
        let mut indexed: Vec<(usize, Combatant)> = std::mem::take(&mut enc.combatants)
            .into_iter()
            .enumerate()
            .collect();
        indexed.sort_by(|a, b| {
            b.1.initiative
                .cmp(&a.1.initiative)
                .then(a.1.order.cmp(&b.1.order))
        });
        let turn = enc.turn_index as usize;
        if let Some(pos) = indexed.iter().position(|(orig, _)| *orig == turn) {
            enc.turn_index = pos as u32;
        }
        enc.combatants = indexed.into_iter().map(|(_, c)| c).collect();
        let enc = enc.clone();
        Ok((
            enc.scene_id.clone(),
            vec![event::Kind::EncounterUpdated(EncounterUpdated {
                encounter: Some(enc),
            })],
        ))
    }

    /// `EndEncounterIntent` (GM only): `not_permitted`, `unknown_encounter`.
    /// Removes the encounter and its condition timers (the conditions stay).
    fn end_encounter(&mut self, is_gm: bool, e: EndEncounterIntent) -> Outcome {
        if !is_gm {
            return Err(Rejection::not_permitted());
        }
        let Some(ei) = self
            .encounters
            .iter()
            .position(|x| x.encounter_id == e.encounter_id)
        else {
            return Err(Rejection::new(
                "unknown_encounter",
                format!("no encounter {:?}", e.encounter_id),
            ));
        };

        // Validated: mutate.
        let enc = self.encounters.remove(ei);
        let mut kinds = vec![event::Kind::EncounterEnded(EncounterEnded {
            encounter_id: enc.encounter_id.clone(),
        })];
        for token in &mut self.tokens {
            if token
                .condition_timers
                .iter()
                .any(|t| t.encounter_id == enc.encounter_id)
            {
                token
                    .condition_timers
                    .retain(|t| t.encounter_id != enc.encounter_id);
                kinds.push(event::Kind::TokenUpserted(TokenUpserted {
                    token: Some(token.clone()),
                }));
            }
        }
        Ok((enc.scene_id, kinds))
    }

    fn chat(&mut self, actor: &str, text: &str) -> Outcome {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return Err(Rejection::new("empty_chat", "chat text is empty"));
        }
        if text.chars().count() > MAX_CHAT_CHARS {
            return Err(Rejection::new(
                "chat_too_long",
                format!("chat text is longer than {MAX_CHAT_CHARS} characters"),
            ));
        }
        Ok((
            self.active_scene_id.clone(),
            vec![event::Kind::ChatPosted(ChatPosted {
                user_id: actor.to_string(),
                text: trimmed.to_string(),
            })],
        ))
    }

    fn roll_dice(
        &mut self,
        actor: &str,
        is_gm: bool,
        r: RollDiceIntent,
        now_ms: i64,
        rng: &mut dyn RollSource,
        ids: &mut dyn IdSource,
    ) -> Outcome {
        if r.label.chars().count() > MAX_LABEL_CHARS {
            return Err(Rejection::new(
                "label_too_long",
                format!("label is longer than {MAX_LABEL_CHARS} characters"),
            ));
        }
        let expr = Expr::parse(&r.notation).map_err(|e| Rejection::new(e.code(), e.to_string()))?;
        let mode = RollMode::try_from(r.mode).unwrap_or(RollMode::Unspecified);
        let rolled = dice::roll(&expr, mode, rng);
        if rng.take_failure() {
            return Err(Rejection::new(
                "rng_unavailable",
                "the server could not get secure randomness; try again",
            ));
        }
        // ADR 0006: only a GM can hide a roll; everyone else is public.
        let visibility = if is_gm && r.visibility == RollVisibility::GmOnly as i32 {
            RollVisibility::GmOnly
        } else {
            RollVisibility::Public
        };
        let roll = RollResult {
            roll_id: ids.next_id(now_ms),
            notation: expr.canonical(),
            purpose: RollPurpose::Freeform as i32,
            mode: rolled.mode as i32,
            dice: rolled.dice,
            modifiers: dice::to_modifiers(&expr),
            total: rolled.total,
            critical: rolled.critical,
            fumble: rolled.fumble,
            visibility: visibility as i32,
            roller_user_id: actor.to_string(),
            roller_creature_id: String::new(),
            label: r.label,
        };
        Ok((
            self.active_scene_id.clone(),
            vec![event::Kind::DiceRolled(DiceRolled { roll: Some(roll) })],
        ))
    }

    fn set_level_background(&mut self, is_gm: bool, b: SetLevelBackgroundIntent) -> Outcome {
        if !is_gm {
            return Err(Rejection::not_permitted());
        }
        let Some(scene_idx) = self.scenes.iter().position(|s| s.scene_id == b.scene_id) else {
            return Err(Rejection::new(
                "unknown_scene",
                format!("no scene {:?}", b.scene_id),
            ));
        };
        let Some(level_idx) = self.scenes[scene_idx]
            .levels
            .iter()
            .position(|l| l.level_id == b.level_id)
        else {
            return Err(Rejection::new(
                "unknown_level",
                format!("no level {:?} in this scene", b.level_id),
            ));
        };
        let background = if b.asset_id.is_empty() {
            None
        } else {
            match self.assets.get(&b.asset_id) {
                Some(asset) => Some(asset.clone()),
                None => {
                    return Err(Rejection::new(
                        "unknown_asset",
                        format!("no uploaded asset {:?}", b.asset_id),
                    ))
                }
            }
        };

        // Validated: mutate.
        self.scenes[scene_idx].levels[level_idx].background = background;
        let scene = self.scenes[scene_idx].clone();
        Ok((
            scene.scene_id.clone(),
            vec![event::Kind::SceneUpserted(SceneUpserted {
                scene: Some(scene),
            })],
        ))
    }

    fn set_controllers(&mut self, is_gm: bool, s: SetControllersIntent) -> Outcome {
        if !is_gm {
            return Err(Rejection::not_permitted());
        }
        let Some(idx) = self.tokens.iter().position(|t| t.token_id == s.token_id) else {
            return Err(Rejection::new(
                "unknown_token",
                format!("no token {:?}", s.token_id),
            ));
        };
        let mut controllers: Vec<String> = Vec::new();
        for user_id in s.controller_user_ids {
            match self.role_of(&user_id) {
                Some(Role::Gm | Role::Player) => {}
                Some(_) => {
                    return Err(Rejection::new(
                        "invalid_controller",
                        format!("{user_id:?} cannot control tokens"),
                    ))
                }
                None => {
                    return Err(Rejection::new(
                        "unknown_user",
                        format!("{user_id:?} is not a member of this campaign"),
                    ))
                }
            }
            if !controllers.contains(&user_id) {
                controllers.push(user_id);
            }
        }
        self.tokens[idx].controller_user_ids = controllers;
        let token = self.tokens[idx].clone();
        let scene_id = token.scene_id.clone();
        // A newly added controller must also receive the sheet of the
        // creature the token represents; the filter decides who gets it.
        let creature = self
            .creatures
            .iter()
            .find(|c| !token.creature_id.is_empty() && c.creature_id == token.creature_id)
            .cloned();
        let mut kinds = vec![event::Kind::TokenUpserted(TokenUpserted {
            token: Some(token),
        })];
        if let Some(creature) = creature {
            kinds.push(event::Kind::CreatureUpserted(CreatureUpserted {
                creature: Some(creature),
            }));
        }
        Ok((scene_id, kinds))
    }
}

/// A condition id: `[a-z0-9_]{1,40}`.
/// Check an imported creature before anything is stored: `bad_creature` when
/// the name is empty or over [`MAX_CREATURE_NAME_CHARS`], there are more than
/// [`MAX_ABILITIES`] abilities or [`MAX_RESOURCES`] resources, a damage or
/// healing dice string does not parse, `hp_max` is negative, `hp_current` is
/// outside `0..=hp_max`, or `provenance.source` is empty.
pub fn validate_import(c: &Creature) -> Result<(), Rejection> {
    let bad = |msg: String| Err(Rejection::new("bad_creature", msg));
    if c.name.trim().is_empty() {
        return bad("the creature needs a name".into());
    }
    if c.name.chars().count() > MAX_CREATURE_NAME_CHARS {
        return bad(format!(
            "the name is longer than {MAX_CREATURE_NAME_CHARS} characters"
        ));
    }
    if c.abilities.len() > MAX_ABILITIES {
        return bad(format!("more than {MAX_ABILITIES} abilities"));
    }
    if c.resources.len() > MAX_RESOURCES {
        return bad(format!("more than {MAX_RESOURCES} resources"));
    }
    if c.hp_max < 0 || c.hp_current < 0 || c.hp_current > c.hp_max {
        return bad("hp_max must be 0 or more and hp_current within 0..=hp_max".into());
    }
    if c.provenance.as_ref().is_none_or(|p| p.source.is_empty()) {
        return bad("provenance.source is required".into());
    }
    for ability in &c.abilities {
        for step in ability.steps.iter().filter_map(|s| s.step.as_ref()) {
            let (damage, healing): (&[_], &str) = match step {
                effect_step::Step::Attack(a) => (&a.damage, ""),
                effect_step::Step::Save(s) => (&s.damage, ""),
                effect_step::Step::Flat(f) => (&f.damage, &f.healing_dice),
                _ => continue,
            };
            let dice = damage
                .iter()
                .map(|p: &tt_contracts::v1::DamagePart| p.dice.as_str())
                .chain((!healing.is_empty()).then_some(healing));
            for text in dice {
                if let Err(e) = Expr::parse(text) {
                    return bad(format!(
                        "ability {:?} has dice {:?} that do not parse: {e:?}",
                        ability.ability_id,
                        text.chars().take(40).collect::<String>()
                    ));
                }
            }
        }
    }
    Ok(())
}

fn is_condition_id(id: &str) -> bool {
    (1..=40).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// Ids (by `key`) of items in `after` that are new or differ from `before`,
/// then ids that were in `before` and are gone.
fn changed_ids<T: PartialEq>(
    before: &[T],
    after: &[T],
    key: impl Fn(&T) -> &String,
) -> Vec<String> {
    let old: HashMap<&String, &T> = before.iter().map(|x| (key(x), x)).collect();
    let mut out: Vec<String> = after
        .iter()
        .filter(|x| old.get(key(x)).is_none_or(|o| *o != *x))
        .map(|x| key(x).clone())
        .collect();
    let kept: std::collections::HashSet<&String> = after.iter().map(&key).collect();
    out.extend(
        before
            .iter()
            .map(&key)
            .filter(|id| !kept.contains(*id))
            .cloned(),
    );
    out
}

/// Adapt a handler with no warnings to the warned shape.
fn plain(outcome: Outcome) -> WarnedOutcome {
    outcome.map(|(scene_id, kinds)| (scene_id, kinds, Vec::new()))
}

/// Sum of straight-line segment lengths from `start` through `path`, in feet.
/// Zero when the grid is unknown or has no cell size.
pub fn path_distance_feet(start: &Vec2, path: &[Vec2], cell_size: f32, feet_per_cell: f32) -> f32 {
    if cell_size <= 0.0 || !cell_size.is_finite() {
        return 0.0;
    }
    let mut prev = (f64::from(start.x), f64::from(start.y));
    let mut units = 0.0_f64;
    for p in path {
        let next = (f64::from(p.x), f64::from(p.y));
        units += ((next.0 - prev.0).powi(2) + (next.1 - prev.1).powi(2)).sqrt();
        prev = next;
    }
    let feet = (units / f64::from(cell_size) * f64::from(feet_per_cell)) as f32;
    if feet.is_finite() {
        feet
    } else {
        0.0
    }
}
