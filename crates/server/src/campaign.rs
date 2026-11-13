//! The pure core: campaign state and the one place it changes.
//!
//! No sockets, no clock, no global state. Time, randomness and id minting are
//! passed to [`Campaign::apply`]. A rejected intent leaves state, `seq` and
//! the log untouched: every handler validates first and mutates last.

use tt_contracts::v1::{
    event, intent, ChatPosted, Creature, CreatureUpserted, DiceRolled, Encounter, Event, Intent,
    Member, MoveTokenIntent, Role, RollDiceIntent, RollMode, RollPurpose, RollResult,
    RollVisibility, Scene, SetControllersIntent, Snapshot, Token, TokenMoved, TokenUpserted, Vec2,
};
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
    /// Sequence number of the last event; 0 before any.
    pub seq: u64,
    /// Append-only event log, unfiltered (server-side only).
    pub log: Vec<Event>,
}

/// A handler's result: the scene the events belong to and the event bodies.
type Outcome = Result<(String, Vec<event::Kind>), Rejection>;

impl Campaign {
    /// Build a campaign from a full (unfiltered) snapshot.
    pub fn from_snapshot(snapshot: Snapshot) -> Self {
        let campaign_id = snapshot
            .scenes
            .iter()
            .map(|s| s.campaign_id.clone())
            .find(|id| !id.is_empty())
            .unwrap_or_default();
        Self {
            campaign_id,
            members: snapshot.members,
            scenes: snapshot.scenes,
            tokens: snapshot.tokens,
            creatures: snapshot.creatures,
            encounters: snapshot.encounters,
            active_scene_id: snapshot.active_scene_id,
            seq: snapshot.seq,
            log: Vec::new(),
        }
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
        let (scene_id, kinds) = match kind {
            intent::Kind::MoveToken(m) => self.move_token(actor_user_id, is_gm, m)?,
            intent::Kind::Chat(c) => self.chat(actor_user_id, &c.text)?,
            intent::Kind::RollDice(r) => {
                self.roll_dice(actor_user_id, is_gm, r, now_ms, rng, ids)?
            }
            intent::Kind::SetControllers(s) => self.set_controllers(is_gm, s)?,
            _ => {
                return Err(Rejection::new(
                    "not_implemented",
                    "this intent is not implemented yet",
                ))
            }
        };
        let mut events = Vec::with_capacity(kinds.len());
        for kind in kinds {
            self.seq += 1;
            let event = Event {
                seq: self.seq,
                at_ms: now_ms,
                scene_id: scene_id.clone(),
                actor_user_id: actor_user_id.to_string(),
                client_intent_id: client_intent_id.clone(),
                warnings: Vec::new(),
                kind: Some(kind),
            };
            self.log.push(event.clone());
            events.push(event);
        }
        Ok(events)
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
