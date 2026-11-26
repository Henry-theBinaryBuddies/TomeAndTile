//! The pure scene model: the client's view of the world, built from decoded
//! `Snapshot`s and `Event`s, plus camera, hit-testing, drag logic and the
//! per-frame draw list. No browser and no GPU here, so all of it is tested
//! natively. Nothing in this file may panic on any input.
//!
//! Units (see README): positions, grid sizes and map sizes are scene units
//! (pixels of the source map image). `Token.position` is the token's CENTRE.
//! A token is a disc of diameter `cells * Grid.cell_size`, with `cells` from
//! its `CreatureSize`. Pointer input is in CSS pixels relative to the canvas.

use std::collections::{HashMap, HashSet};

use prost::Message as _;
use tt_contracts::v1::{
    client_message, event, intent, server_message, ClientMessage, CreatureSize, Event, GridKind,
    HealthState, Intent, Level, MoveTokenIntent, Role, Scene, ServerMessage, Snapshot, Token, Vec2,
};

use crate::camera::{clamp_coord, Camera, Pt};
use crate::draw::{color_for_id, DrawItem, DrawList};

/// Cell size used when a scene's grid size is missing or nonsense.
pub const DEFAULT_CELL: f64 = 70.0;
/// A move preview that the server has not answered within this long is dropped.
pub const PENDING_TIMEOUT_MS: f64 = 5000.0;
/// Pointer travel (CSS px) before a press on a token becomes a drag.
pub const DRAG_THRESHOLD_PX: f64 = 3.0;
/// Pointer travel (CSS px) a press on a token may wander while picking and still
/// count as a click. Wider than `DRAG_THRESHOLD_PX` because a finger tap moves
/// more than a mouse click; the map does not move until this is exceeded.
pub const PICK_SLOP_PX: f64 = 8.0;
/// Below this cell size on screen the grid is not drawn at all.
pub const MIN_GRID_CELL_PX: f64 = 6.0;
/// At or above this cell size on screen the grid is fully opaque.
pub const FULL_GRID_CELL_PX: f64 = 14.0;
/// Hard cap on grid lines per frame; over it the grid is skipped.
pub const MAX_GRID_LINES: usize = 1500;
/// Tokens are always at least this big (screen px radius) to hit-test.
pub const MIN_HIT_RADIUS_PX: f64 = 8.0;
/// Ring colour for a token the host has marked as picked: cyan, so it cannot be
/// mistaken for the yellow selection ring.
const HIGHLIGHT_COLOR: [f32; 4] = [0.3, 0.85, 1.0, 1.0];
/// The thin outline every token has, and the health rings that replace it.
/// Drawn from `Token.health_state` only, never from HP numbers, so every
/// viewer sees the same ring (ADR 0006).
const OUTLINE_COLOR: [f32; 4] = [0.0, 0.0, 0.0, 0.6];
const BLOODIED_COLOR: [f32; 4] = [1.0, 0.55, 0.1, 1.0];
const NEAR_DEATH_COLOR: [f32; 4] = [0.9, 0.12, 0.12, 1.0];
const DOWN_COLOR: [f32; 4] = [0.55, 0.55, 0.58, 1.0];
/// A downed token's disc is drawn at this fraction of its opacity.
const DOWN_FADE: f32 = 0.45;
/// The condition marker: a small disc at the token's upper right edge.
const CONDITION_MARKER_COLOR: [f32; 4] = [0.78, 0.42, 1.0, 1.0];
/// Screen radius of the condition marker, fixed so it stays visible on a
/// phone at any zoom.
pub const CONDITION_MARKER_PX: f32 = 6.0;
const BACKGROUND: [f32; 4] = [0.09, 0.09, 0.11, 1.0];
const PLACEHOLDER: [f32; 4] = [0.15, 0.16, 0.19, 1.0];
const PLACEHOLDER_FAILED: [f32; 4] = [0.26, 0.12, 0.12, 1.0];
const MAX_URL_LEN: usize = 2048;
/// Longest label text asked for (characters). Longer names are cut here, before
/// the host shortens them further to fit its maximum width.
const MAX_LABEL_CHARS: usize = 120;
/// Gap (screen px) between a token's edge and the top of its name label.
pub const LABEL_GAP_PX: f32 = 3.0;
/// A label is hidden when its token's on-screen diameter is below this (px).
pub const LABEL_MIN_TOKEN_PX: f64 = 20.0;
/// A label may be this many times its token's on-screen diameter wide, within
/// `LABEL_MIN_WIDTH_PX..=LABEL_MAX_WIDTH_PX`, rounded down to `LABEL_WIDTH_STEP_PX`.
const LABEL_WIDTH_FACTOR: f64 = 1.25;
pub const LABEL_MIN_WIDTH_PX: u32 = 40;
pub const LABEL_MAX_WIDTH_PX: u32 = 140;
pub const LABEL_WIDTH_STEP_PX: u32 = 20;
/// Prefix of a label's image key. A key is never a fetchable URL: the first
/// colon comes before any slash, so `loadable_url` refuses it.
const LABEL_KEY_PREFIX: &str = "label:";

/// Load state of an image the model wants drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ImageState {
    Pending,
    Ready { width: u32, height: u32 },
    Failed,
}

/// A name label the host should render to a bitmap: `key` is the image key to
/// upload it under, `text` the name, `dpr` the device pixel ratio it is for and
/// `max_width` the widest it may be, in CSS px (the text is ellipsized to fit).
#[derive(Clone, Debug, PartialEq)]
pub struct LabelRequest {
    pub key: String,
    pub text: String,
    pub dpr: f64,
    pub max_width: u32,
}

/// The server refused an intent.
#[derive(Clone, Debug, PartialEq)]
pub struct Rejection {
    pub client_intent_id: String,
    pub code: String,
    pub message: String,
}

/// What `apply_server_message` did, for the host to react to.
#[derive(Clone, Debug, PartialEq)]
pub enum Applied {
    Snapshot,
    Event,
    Rejected(Rejection),
    /// A message kind this renderer does not use (e.g. a media token).
    Ignored,
}

/// The grid as the model uses it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GridInfo {
    pub cell: f64,
    pub offset: Pt,
    /// True for a square grid (the only kind drawn and snapped to).
    pub square: bool,
}

#[derive(Clone, Debug)]
struct Pending {
    client_intent_id: String,
    token_id: String,
    dest: Pt,
    sent_ms: f64,
}

#[derive(Clone, Debug)]
enum Interaction {
    Idle,
    Pan {
        last: Pt,
    },
    Drag {
        token_id: String,
        /// Token centre minus pointer position, in world units.
        grab_offset: Pt,
        start_screen: Pt,
        preview: Pt,
        moved: bool,
    },
    /// Button 0 went down on a token while picking. A release here (with the
    /// pointer still under `PICK_SLOP_PX` from the press) picks the token; travel
    /// past the slop turns the press into a plain `Pan`. The camera stays put
    /// until then.
    PickPress {
        token_id: String,
        start_screen: Pt,
    },
}

/// The client's whole view of one campaign.
pub struct Model {
    user_id: String,
    role: i32,
    scenes: Vec<Scene>,
    active_scene_id: String,
    tokens: Vec<Token>,
    last_seq: u64,
    camera: Camera,
    needs_fit: bool,
    images: HashMap<String, ImageState>,
    /// Name-label bitmaps by key (see `label_key`); kept apart from `images` so
    /// labels can never be mistaken for fetches.
    labels: HashMap<String, ImageState>,
    /// Creature id -> name, from snapshots and `CreatureUpserted`.
    creature_names: HashMap<String, String>,
    /// Device pixel ratio, for label bitmap sizes. Always finite and positive.
    dpr: f64,
    selected: Option<String>,
    interaction: Interaction,
    pending: Vec<Pending>,
    nonce: u64,
    next_intent: u64,
    last_rejection: Option<Rejection>,
    /// Host-owned: clicks on tokens are picks, not selects or drags.
    picking: bool,
    /// Host-owned: tokens marked as picked (ring drawn). Never holds "".
    highlights: HashSet<String>,
    /// Picks made by the user, waiting for the host to take them.
    picks: Vec<String>,
}

impl Model {
    /// `nonce` makes client intent ids unique across page loads; the host
    /// passes something random.
    pub fn new(nonce: u64) -> Self {
        Model {
            user_id: String::new(),
            role: Role::Unspecified as i32,
            scenes: Vec::new(),
            active_scene_id: String::new(),
            tokens: Vec::new(),
            last_seq: 0,
            camera: Camera::default(),
            needs_fit: false,
            images: HashMap::new(),
            labels: HashMap::new(),
            creature_names: HashMap::new(),
            dpr: 1.0,
            selected: None,
            interaction: Interaction::Idle,
            pending: Vec::new(),
            nonce,
            next_intent: 0,
            last_rejection: None,
            picking: false,
            highlights: HashSet::new(),
            picks: Vec::new(),
        }
    }

    // ------------------------------------------------------------ state in

    /// Decode a whole `ServerMessage` and apply it. Bad bytes are an error and
    /// leave the model untouched.
    pub fn apply_server_message(&mut self, bytes: &[u8]) -> Result<Applied, prost::DecodeError> {
        let msg = ServerMessage::decode(bytes)?;
        Ok(match msg.kind {
            Some(server_message::Kind::Snapshot(s)) => {
                self.apply_snapshot(s);
                Applied::Snapshot
            }
            Some(server_message::Kind::Event(e)) => {
                self.apply_event(e);
                Applied::Event
            }
            Some(server_message::Kind::IntentRejected(r)) => {
                let rej = Rejection {
                    client_intent_id: r.client_intent_id,
                    code: r.code,
                    message: r.message,
                };
                // Only a refusal of one of this renderer's own moves is reported. A
                // refusal of a chat or dice intent belongs to the chat panel; the host
                // would otherwise show it on the map as well.
                if self.apply_rejection(&rej) {
                    Applied::Rejected(rej)
                } else {
                    Applied::Ignored
                }
            }
            _ => Applied::Ignored,
        })
    }

    /// Replace everything with the viewer's state as of the snapshot.
    pub fn apply_snapshot(&mut self, s: Snapshot) {
        let scene_changed = s.active_scene_id != self.active_scene_id || self.scenes.is_empty();
        self.user_id = s.user_id;
        self.role = s.role;
        self.scenes = s
            .scenes
            .into_iter()
            .filter(|sc| !sc.scene_id.is_empty())
            .collect();
        self.active_scene_id = s.active_scene_id;
        self.tokens = s.tokens.into_iter().filter_map(sanitize_token).collect();
        self.creature_names = s
            .creatures
            .into_iter()
            .filter(|c| !c.creature_id.is_empty())
            .map(|c| (c.creature_id, c.name))
            .collect();
        self.last_seq = s.seq;
        self.selected = None;
        self.interaction = Interaction::Idle;
        self.pending.clear();
        // `picking` and `highlights` belong to the host and survive; picks made
        // before the snapshot refer to a world that has been replaced.
        self.picks.clear();
        if scene_changed {
            self.needs_fit = true;
            // The viewport usually exists already (the host sizes the canvas before the
            // first Snapshot arrives), so frame the scene now, not at the next resize.
            self.apply_fit_if_needed();
        }
    }

    /// Apply one event. Events that do not affect what is drawn, events for
    /// unknown tokens and events with bad numbers are ignored.
    ///
    /// Events are applied in arrival order whatever their `seq`: the server
    /// sends a snapshot and then each event once, in order (ADR 0004), and a
    /// reconnect starts from a fresh snapshot. `seq` is only recorded.
    pub fn apply_event(&mut self, e: Event) {
        self.last_seq = self.last_seq.max(e.seq);
        match e.kind {
            Some(event::Kind::TokenUpserted(u)) => {
                if let Some(t) = u.token.and_then(sanitize_token) {
                    self.pending.retain(|p| p.token_id != t.token_id);
                    match self.tokens.iter_mut().find(|x| x.token_id == t.token_id) {
                        Some(slot) => *slot = t,
                        None => self.tokens.push(t),
                    }
                }
            }
            Some(event::Kind::TokenRemoved(r)) => {
                self.tokens.retain(|t| t.token_id != r.token_id);
                self.pending.retain(|p| p.token_id != r.token_id);
                if self.selected.as_deref() == Some(r.token_id.as_str()) {
                    self.selected = None;
                }
                if matches!(&self.interaction, Interaction::Drag { token_id, .. } | Interaction::PickPress { token_id, .. } if *token_id == r.token_id)
                {
                    self.interaction = Interaction::Idle;
                }
            }
            Some(event::Kind::TokenMoved(m)) => {
                let Some(dest) = m.path.last().and_then(vec2_to_pt) else {
                    return;
                };
                if let Some(t) = self.tokens.iter_mut().find(|t| t.token_id == m.token_id) {
                    t.position = Some(pt_to_vec2(dest));
                    if !m.level_id.is_empty() {
                        t.level_id = m.level_id;
                    }
                    // The server has answered for this token.
                    self.pending.retain(|p| p.token_id != t.token_id);
                }
            }
            Some(event::Kind::CreatureUpserted(u)) => {
                if let Some(c) = u.creature.filter(|c| !c.creature_id.is_empty()) {
                    self.creature_names.insert(c.creature_id, c.name);
                }
            }
            Some(event::Kind::SceneUpserted(u)) => {
                if let Some(sc) = u.scene.filter(|s| !s.scene_id.is_empty()) {
                    match self.scenes.iter_mut().find(|x| x.scene_id == sc.scene_id) {
                        Some(slot) => *slot = sc,
                        None => self.scenes.push(sc),
                    }
                }
            }
            _ => {}
        }
    }

    /// An intent was refused: drop its preview and remember why. Returns true if the
    /// refused intent was one of this renderer's own moves (it had a preview).
    pub fn apply_rejection(&mut self, r: &Rejection) -> bool {
        let before = self.pending.len();
        self.pending
            .retain(|p| p.client_intent_id != r.client_intent_id);
        self.last_rejection = Some(r.clone());
        self.pending.len() != before
    }

    /// Drop previews the server never answered. Call once per frame.
    pub fn expire_pending(&mut self, now_ms: f64) {
        self.pending
            .retain(|p| now_ms - p.sent_ms <= PENDING_TIMEOUT_MS);
    }

    // ------------------------------------------------------------- queries

    pub fn user_id(&self) -> &str {
        &self.user_id
    }

    pub fn last_seq(&self) -> u64 {
        self.last_seq
    }

    pub fn last_rejection(&self) -> Option<&Rejection> {
        self.last_rejection.as_ref()
    }

    pub fn camera(&self) -> &Camera {
        &self.camera
    }

    pub fn selected(&self) -> Option<&str> {
        self.selected.as_deref()
    }

    pub fn token(&self, id: &str) -> Option<&Token> {
        self.tokens.iter().find(|t| t.token_id == id)
    }

    pub fn token_count(&self) -> usize {
        self.tokens.len()
    }

    pub fn has_pending(&self, token_id: &str) -> bool {
        self.pending.iter().any(|p| p.token_id == token_id)
    }

    pub fn is_dragging(&self) -> bool {
        matches!(self.interaction, Interaction::Drag { moved: true, .. })
    }

    /// Is picking mode on?
    pub fn picking(&self) -> bool {
        self.picking
    }

    /// Has the host marked this token as picked?
    pub fn is_highlighted(&self, id: &str) -> bool {
        self.highlights.contains(id)
    }

    /// The scene being drawn: the snapshot's active scene, else the first.
    pub fn active_scene(&self) -> Option<&Scene> {
        self.scenes
            .iter()
            .find(|s| s.scene_id == self.active_scene_id)
            .or_else(|| self.scenes.first())
    }

    /// The one level drawn: the scene's first level.
    pub fn current_level(&self) -> Option<&Level> {
        self.active_scene().and_then(|s| s.levels.first())
    }

    pub fn grid(&self) -> GridInfo {
        let Some(g) = self.active_scene().and_then(|s| s.grid.as_ref()) else {
            return GridInfo {
                cell: DEFAULT_CELL,
                offset: Pt::default(),
                square: false,
            };
        };
        let cell = f64::from(g.cell_size);
        let cell = if cell.is_finite() && (1.0..=1.0e6).contains(&cell) {
            cell
        } else {
            DEFAULT_CELL
        };
        let offset = g.offset.as_ref().and_then(vec2_to_pt).unwrap_or_default();
        GridInfo {
            cell,
            offset,
            square: GridKind::try_from(g.kind) == Ok(GridKind::Square),
        }
    }

    /// Is this token on the active scene and level?
    fn on_view(&self, t: &Token) -> bool {
        let Some(scene) = self.active_scene() else {
            return false;
        };
        if !t.scene_id.is_empty() && t.scene_id != scene.scene_id {
            return false;
        }
        match scene.levels.first() {
            Some(l) => t.level_id.is_empty() || t.level_id == l.level_id,
            None => true,
        }
    }

    /// Token diameter in world units.
    pub fn token_diameter(&self, t: &Token) -> f64 {
        size_cells(t.size) * self.grid().cell
    }

    /// May this viewer move this token? The GM may move any; players only
    /// tokens that list them as a controller. Spectators never.
    pub fn can_move(&self, t: &Token) -> bool {
        if self.role == Role::Gm as i32 {
            return true;
        }
        self.role == Role::Player as i32
            && !self.user_id.is_empty()
            && t.controller_user_ids.contains(&self.user_id)
    }

    /// Where a token is drawn: the drag preview, the pending destination, or
    /// its real position.
    fn display_pos(&self, t: &Token) -> Pt {
        if let Interaction::Drag {
            token_id,
            preview,
            moved: true,
            ..
        } = &self.interaction
        {
            if *token_id == t.token_id {
                return *preview;
            }
        }
        if let Some(p) = self.pending.iter().find(|p| p.token_id == t.token_id) {
            return p.dest;
        }
        t.position.as_ref().and_then(vec2_to_pt).unwrap_or_default()
    }

    /// The topmost token whose disc contains `world`, if any. Tokens drawn
    /// later are on top, so the search runs from the end. Tiny discs get a
    /// minimum on-screen radius so they can still be grabbed.
    pub fn hit_test(&self, world: Pt) -> Option<&str> {
        if !world.is_finite() {
            return None;
        }
        let min_r = MIN_HIT_RADIUS_PX / self.camera.zoom;
        self.tokens
            .iter()
            .rev()
            .filter(|t| self.on_view(t))
            .find(|t| {
                let c = self.display_pos(t);
                let r = (self.token_diameter(t) * 0.5).max(min_r);
                let (dx, dy) = (world.x - c.x, world.y - c.y);
                dx * dx + dy * dy <= r * r
            })
            .map(|t| t.token_id.as_str())
    }

    /// Snap a world point to the grid for a token of `cells` cells across:
    /// odd sizes centre in a cell, even sizes (2, 4) sit on grid corners.
    /// With no square grid the point is returned unchanged.
    pub fn snap(&self, p: Pt, cells: f64) -> Pt {
        let g = self.grid();
        if !g.square || !p.is_finite() {
            return p;
        }
        let even = cells >= 2.0 && (cells.round() as i64) % 2 == 0;
        let axis = |v: f64, o: f64| {
            let u = (v - o) / g.cell;
            let s = if even { u.round() } else { u.floor() + 0.5 };
            clamp_coord(o + s * g.cell)
        };
        Pt::new(axis(p.x, g.offset.x), axis(p.y, g.offset.y))
    }

    // -------------------------------------------------------------- camera

    /// Set the device pixel ratio name labels are rendered for. Anything not
    /// finite and positive is ignored; the value is kept within 0.25..8.
    pub fn set_device_pixel_ratio(&mut self, dpr: f64) {
        if dpr.is_finite() && dpr > 0.0 {
            self.dpr = dpr.clamp(0.25, 8.0);
        }
    }

    pub fn resize(&mut self, width: f64, height: f64) {
        self.camera.set_viewport(width, height);
        self.apply_fit_if_needed();
    }

    /// Frame the map (or, with no map, the tokens) once the viewport and
    /// scene both exist. User pan and zoom cancel a pending fit.
    fn apply_fit_if_needed(&mut self) {
        if !self.needs_fit
            || self.camera.width <= 0.0
            || self.camera.height <= 0.0
            || self.active_scene().is_none()
            || self.waiting_for_map_size()
        {
            return;
        }
        let (min, max) = self.content_bounds();
        self.camera.fit(min, max, 24.0);
        self.needs_fit = false;
    }

    /// The map's size is not in the scene and its image has not loaded (or failed)
    /// yet, so framing now would use the tokens instead of the map.
    fn waiting_for_map_size(&self) -> bool {
        let Some(bg) = self.current_level().and_then(|l| l.background.as_ref()) else {
            return false;
        };
        if (bg.width > 0 && bg.height > 0) || !loadable_url(&bg.url) {
            return false;
        }
        !matches!(
            self.images.get(&bg.url),
            Some(ImageState::Ready { .. } | ImageState::Failed)
        )
    }

    fn map_size(&self) -> Option<(f64, f64)> {
        let bg = self.current_level()?.background.as_ref()?;
        if bg.width > 0 && bg.height > 0 {
            return Some((f64::from(bg.width), f64::from(bg.height)));
        }
        match self.images.get(&bg.url) {
            Some(ImageState::Ready { width, height }) => {
                Some((f64::from(*width), f64::from(*height)))
            }
            _ => None,
        }
    }

    fn content_bounds(&self) -> (Pt, Pt) {
        if let Some((w, h)) = self.map_size() {
            return (Pt::new(0.0, 0.0), Pt::new(w, h));
        }
        let cell = self.grid().cell;
        let mut min = Pt::new(0.0, 0.0);
        let mut max = Pt::new(cell * 12.0, cell * 8.0);
        for t in self.tokens.iter().filter(|t| self.on_view(t)) {
            let p = self.display_pos(t);
            min.x = min.x.min(p.x - 2.0 * cell);
            min.y = min.y.min(p.y - 2.0 * cell);
            max.x = max.x.max(p.x + 2.0 * cell);
            max.y = max.y.max(p.y + 2.0 * cell);
        }
        (min, max)
    }

    /// Zoom by a wheel movement about a screen point. `delta` is in pixels,
    /// positive = away from the user = zoom out.
    pub fn wheel(&mut self, x: f64, y: f64, delta: f64) {
        if !(x.is_finite() && y.is_finite() && delta.is_finite()) {
            return;
        }
        let factor = (-delta * 0.0015).exp().clamp(0.5, 2.0);
        self.camera.zoom_about(Pt::new(x, y), factor);
        self.needs_fit = false;
    }

    // -------------------------------------------------------------- picking

    /// Turn picking mode on or off. Turning it on abandons a drag in progress
    /// (no intent is sent); turning it off turns a press on a token into a plain
    /// pan, so the release picks nothing.
    pub fn set_picking(&mut self, enabled: bool) {
        self.picking = enabled;
        match &self.interaction {
            Interaction::Drag { .. } if enabled => self.interaction = Interaction::Idle,
            Interaction::PickPress { start_screen, .. } if !enabled => {
                self.interaction = Interaction::Pan {
                    last: *start_screen,
                };
            }
            _ => {}
        }
    }

    /// Replace the whole set of highlighted tokens. Empty ids are dropped; ids
    /// that are not on the map are kept but draw nothing.
    pub fn set_highlights(&mut self, ids: impl IntoIterator<Item = String>) {
        self.highlights = ids.into_iter().filter(|id| !id.is_empty()).collect();
    }

    /// The tokens the user has picked since the last call, oldest first.
    pub fn take_picks(&mut self) -> Vec<String> {
        std::mem::take(&mut self.picks)
    }

    // ---------------------------------------------------------------- input

    /// Pointer pressed at screen `(x, y)`. Button 0 on a token selects it and,
    /// if the viewer may move it, starts a drag. Anything else pans. While
    /// picking, button 0 on a token starts a pick press instead (no select, no
    /// drag) and the selection is left alone.
    pub fn pointer_down(&mut self, x: f64, y: f64, button: i16) {
        let s = Pt::new(x, y);
        if !s.is_finite() {
            return;
        }
        let world = self.camera.screen_to_world(s);
        let hit = if button == 0 {
            self.hit_test(world).map(str::to_owned)
        } else {
            None
        };
        if self.picking {
            self.interaction = match hit {
                Some(token_id) => Interaction::PickPress {
                    token_id,
                    start_screen: s,
                },
                None => Interaction::Pan { last: s },
            };
            return;
        }
        self.selected = hit.clone();
        self.interaction = Interaction::Pan { last: s };
        let Some(id) = hit else { return };
        let Some(t) = self.token(&id) else { return };
        if self.can_move(t) && !self.has_pending(&id) {
            let c = self.display_pos(t);
            self.interaction = Interaction::Drag {
                token_id: id,
                grab_offset: Pt::new(c.x - world.x, c.y - world.y),
                start_screen: s,
                preview: c,
                moved: false,
            };
        }
        // A token the viewer cannot move is selected, and the press pans.
    }

    pub fn pointer_move(&mut self, x: f64, y: f64) {
        let s = Pt::new(x, y);
        if !s.is_finite() {
            return;
        }
        let world = self.camera.screen_to_world(s);
        match &mut self.interaction {
            Interaction::Idle => {}
            Interaction::Pan { last } => {
                let (dx, dy) = (s.x - last.x, s.y - last.y);
                *last = s;
                self.camera.pan_by_screen(dx, dy);
                self.needs_fit = false;
            }
            Interaction::PickPress { start_screen, .. } => {
                let (tx, ty) = (s.x - start_screen.x, s.y - start_screen.y);
                if tx * tx + ty * ty >= PICK_SLOP_PX * PICK_SLOP_PX {
                    // Too far to be a click: the user is panning, not picking.
                    // Catch the map up with the whole travel so far.
                    self.camera.pan_by_screen(tx, ty);
                    self.needs_fit = false;
                    self.interaction = Interaction::Pan { last: s };
                }
            }
            Interaction::Drag {
                grab_offset,
                start_screen,
                preview,
                moved,
                ..
            } => {
                if !*moved {
                    let (dx, dy) = (s.x - start_screen.x, s.y - start_screen.y);
                    if dx * dx + dy * dy < DRAG_THRESHOLD_PX * DRAG_THRESHOLD_PX {
                        return;
                    }
                    *moved = true;
                }
                *preview = Pt::new(
                    clamp_coord(world.x + grab_offset.x),
                    clamp_coord(world.y + grab_offset.y),
                );
            }
        }
    }

    /// Pointer released. If a drag ended on a different spot, returns the
    /// encoded `ClientMessage` carrying a `MoveTokenIntent` for the host to
    /// send. The token itself does not move until the server's event arrives
    /// (ADR 0004); until then (or a rejection, or the timeout) it is drawn at
    /// the snapped destination. A release that ends a pick press queues the
    /// token for `take_picks` and returns `None`.
    pub fn pointer_up(&mut self, x: f64, y: f64, now_ms: f64) -> Option<Vec<u8>> {
        self.pointer_move(x, y);
        let interaction = std::mem::replace(&mut self.interaction, Interaction::Idle);
        if let Interaction::PickPress { token_id, .. } = &interaction {
            // Still received and still on the drawn scene and level.
            if self.token(token_id).is_some_and(|t| self.on_view(t)) {
                self.picks.push(token_id.clone());
            }
            return None;
        }
        let Interaction::Drag {
            token_id,
            preview,
            moved: true,
            ..
        } = interaction
        else {
            return None;
        };
        let t = self.token(&token_id)?;
        if !self.can_move(t) {
            return None;
        }
        let dest = self.snap(preview, size_cells(t.size));
        let here = t.position.as_ref().and_then(vec2_to_pt).unwrap_or_default();
        if (dest.x - here.x).abs() < 1e-3 && (dest.y - here.y).abs() < 1e-3 {
            return None;
        }
        let level_id = if t.level_id.is_empty() {
            self.current_level()
                .map(|l| l.level_id.clone())
                .unwrap_or_default()
        } else {
            t.level_id.clone()
        };
        let client_intent_id = format!("ci-{:x}-{}", self.nonce, self.next_intent);
        self.next_intent += 1;
        self.pending.retain(|p| p.token_id != token_id);
        self.pending.push(Pending {
            client_intent_id: client_intent_id.clone(),
            token_id: token_id.clone(),
            dest,
            sent_ms: now_ms,
        });
        Some(encode_move(client_intent_id, token_id, level_id, dest))
    }

    /// The pointer was cancelled (focus lost, capture lost): abandon any drag.
    pub fn pointer_cancel(&mut self) {
        self.interaction = Interaction::Idle;
    }

    // --------------------------------------------------------------- images

    /// URLs the model wants loaded and has not asked for before. Marks them
    /// pending, so each URL is returned once.
    pub fn take_image_requests(&mut self) -> Vec<String> {
        let mut want: Vec<String> = Vec::new();
        if let Some(bg) = self.current_level().and_then(|l| l.background.as_ref()) {
            want.push(bg.url.clone());
        }
        for t in self.tokens.iter().filter(|t| self.on_view(t)) {
            if let Some(img) = &t.image {
                want.push(img.url.clone());
            }
        }
        let mut out = Vec::new();
        for url in want {
            if loadable_url(&url) && !self.images.contains_key(&url) {
                self.images.insert(url.clone(), ImageState::Pending);
                out.push(url);
            }
        }
        out
    }

    pub fn mark_image_loaded(&mut self, url: &str, width: u32, height: u32) {
        self.images
            .insert(url.to_owned(), ImageState::Ready { width, height });
        self.apply_fit_if_needed();
    }

    pub fn mark_image_failed(&mut self, url: &str) {
        self.images.insert(url.to_owned(), ImageState::Failed);
        // A map whose size was being waited for is framed by its tokens instead.
        self.apply_fit_if_needed();
    }

    pub fn image_state(&self, url: &str) -> Option<ImageState> {
        self.images.get(url).copied()
    }

    fn ready_image(&self, url: &str) -> Option<String> {
        matches!(self.images.get(url), Some(ImageState::Ready { .. })).then(|| url.to_owned())
    }

    // --------------------------------------------------------------- labels

    /// The name to show under a token: its creature's name when the viewer has
    /// received that creature, else the token's own name, else none. Control
    /// characters and surrounding space are dropped; long names are cut.
    pub fn label_text(&self, t: &Token) -> Option<String> {
        let from_creature = self
            .creature_names
            .get(&t.creature_id)
            .and_then(|n| clean_label(n));
        from_creature.or_else(|| clean_label(&t.name))
    }

    fn label_key(&self, text: &str, width: u32) -> String {
        format!("{LABEL_KEY_PREFIX}{:.2}:{width}:{text}", self.dpr)
    }

    /// The label width step (CSS px) for a token drawn `diameter_px` across, or
    /// `None` when the token is too small on screen to label.
    pub fn label_width_for(diameter_px: f64) -> Option<u32> {
        if !diameter_px.is_finite() || diameter_px < LABEL_MIN_TOKEN_PX {
            return None;
        }
        let w = (diameter_px * LABEL_WIDTH_FACTOR)
            .clamp(f64::from(LABEL_MIN_WIDTH_PX), f64::from(LABEL_MAX_WIDTH_PX))
            as u32;
        Some(w / LABEL_WIDTH_STEP_PX * LABEL_WIDTH_STEP_PX)
    }

    /// Label text and width step for a token at the current zoom.
    fn label_spec(&self, t: &Token) -> Option<(String, u32)> {
        let width = Self::label_width_for(self.token_diameter(t) * self.camera.zoom)?;
        Some((self.label_text(t)?, width))
    }

    /// Labels the model wants rendered and has not asked for before (one per
    /// distinct text, device pixel ratio and width step, for tokens on the drawn
    /// scene and level that are big enough on screen). Marks them pending, so
    /// each is returned once.
    pub fn take_label_requests(&mut self) -> Vec<LabelRequest> {
        let mut out = Vec::new();
        let specs: Vec<(String, u32)> = self
            .tokens
            .iter()
            .filter(|t| self.on_view(t))
            .filter_map(|t| self.label_spec(t))
            .collect();
        for (text, max_width) in specs {
            let key = self.label_key(&text, max_width);
            if !self.labels.contains_key(&key) {
                self.labels.insert(key.clone(), ImageState::Pending);
                out.push(LabelRequest {
                    key,
                    text,
                    dpr: self.dpr,
                    max_width,
                });
            }
        }
        out
    }

    pub fn mark_label_loaded(&mut self, key: &str, width: u32, height: u32) {
        self.labels
            .insert(key.to_owned(), ImageState::Ready { width, height });
    }

    pub fn mark_label_failed(&mut self, key: &str) {
        self.labels.insert(key.to_owned(), ImageState::Failed);
    }

    pub fn label_state(&self, key: &str) -> Option<ImageState> {
        self.labels.get(key).copied()
    }

    /// The label of `t`, centred under its disc, at a fixed screen size
    /// (bitmap pixels over the device pixel ratio). Nothing until it is ready.
    fn push_label(&self, items: &mut Vec<DrawItem>, t: &Token) {
        let Some((text, width_step)) = self.label_spec(t) else {
            return;
        };
        let key = self.label_key(&text, width_step);
        let Some(ImageState::Ready { width, height }) = self.labels.get(&key).copied() else {
            return;
        };
        if width == 0 || height == 0 {
            return;
        }
        let cam = &self.camera;
        let s = cam.world_to_screen(self.display_pos(t));
        let r = (self.token_diameter(t) * 0.5 * cam.zoom).max(3.0) as f32;
        let (w, h) = (
            width as f32 / self.dpr as f32,
            height as f32 / self.dpr as f32,
        );
        let x0 = s.x as f32 - w * 0.5;
        let y0 = s.y as f32 + r + LABEL_GAP_PX;
        items.push(DrawItem::Image {
            url: key,
            x0,
            y0,
            x1: x0 + w,
            y1: y0 + h,
        });
    }

    // ------------------------------------------------------------ draw list

    /// What to draw this frame. Pure: depends only on the model.
    pub fn draw_list(&self) -> DrawList {
        let mut items = Vec::new();
        if self.active_scene().is_none() {
            return DrawList {
                background: BACKGROUND,
                items,
            };
        }
        let cam = &self.camera;
        let map = self.map_size();

        // Map image, or a placeholder of the same size until it is ready.
        if let (Some((w, h)), Some(bg)) = (
            map,
            self.current_level().and_then(|l| l.background.as_ref()),
        ) {
            let a = cam.world_to_screen(Pt::new(0.0, 0.0));
            let b = cam.world_to_screen(Pt::new(w, h));
            let (x0, y0, x1, y1) = (a.x as f32, a.y as f32, b.x as f32, b.y as f32);
            match self.images.get(&bg.url) {
                Some(ImageState::Ready { .. }) => items.push(DrawItem::Image {
                    url: bg.url.clone(),
                    x0,
                    y0,
                    x1,
                    y1,
                }),
                state => items.push(DrawItem::Rect {
                    x0,
                    y0,
                    x1,
                    y1,
                    color: if state == Some(&ImageState::Failed) {
                        PLACEHOLDER_FAILED
                    } else {
                        PLACEHOLDER
                    },
                }),
            }
        }

        self.push_grid(&mut items, map);

        // Tokens in order, the dragged one last so it is on top.
        let dragged = match &self.interaction {
            Interaction::Drag {
                token_id,
                moved: true,
                ..
            } => Some(token_id.as_str()),
            _ => None,
        };
        let visible: Vec<&Token> = self.tokens.iter().filter(|t| self.on_view(t)).collect();
        for t in visible
            .iter()
            .filter(|t| Some(t.token_id.as_str()) != dragged)
        {
            self.push_token(&mut items, t);
        }
        if let Some(t) = visible
            .iter()
            .find(|t| Some(t.token_id.as_str()) == dragged)
        {
            // Ring where it would land if released now.
            if let Interaction::Drag { preview, .. } = &self.interaction {
                let dest = self.snap(*preview, size_cells(t.size));
                let s = cam.world_to_screen(dest);
                items.push(DrawItem::Ring {
                    cx: s.x as f32,
                    cy: s.y as f32,
                    r: (self.token_diameter(t) * 0.5 * cam.zoom).max(3.0) as f32,
                    thickness: 2.0,
                    color: [1.0, 1.0, 1.0, 0.7],
                });
            }
            self.push_token(&mut items, t);
        }
        // Name labels last, so they stay readable over every token and ring.
        // Same order as the tokens: the dragged one, and so its label, last.
        for t in visible
            .iter()
            .filter(|t| Some(t.token_id.as_str()) != dragged)
        {
            self.push_label(&mut items, t);
        }
        if let Some(t) = visible
            .iter()
            .find(|t| Some(t.token_id.as_str()) == dragged)
        {
            self.push_label(&mut items, t);
        }
        DrawList {
            background: BACKGROUND,
            items,
        }
    }

    fn push_grid(&self, items: &mut Vec<DrawItem>, map: Option<(f64, f64)>) {
        let g = self.grid();
        let cam = &self.camera;
        let cell_px = g.cell * cam.zoom;
        if !g.square || cell_px < MIN_GRID_CELL_PX || cam.width <= 0.0 || cam.height <= 0.0 {
            return;
        }
        // Visible world area, clipped to the map when its size is known.
        let (mut min, mut max) = cam.visible_world();
        if let Some((w, h)) = map {
            min = Pt::new(min.x.max(0.0), min.y.max(0.0));
            max = Pt::new(max.x.min(w), max.y.min(h));
        }
        if min.x >= max.x || min.y >= max.y {
            return;
        }
        let k = |v: f64, o: f64| (v - o) / g.cell;
        let (kx0, kx1) = (k(min.x, g.offset.x).ceil(), k(max.x, g.offset.x).floor());
        let (ky0, ky1) = (k(min.y, g.offset.y).ceil(), k(max.y, g.offset.y).floor());
        let count = (kx1 - kx0 + 1.0).max(0.0) + (ky1 - ky0 + 1.0).max(0.0);
        if count.is_nan() || count > MAX_GRID_LINES as f64 {
            return;
        }
        let fade = ((cell_px - MIN_GRID_CELL_PX) / (FULL_GRID_CELL_PX - MIN_GRID_CELL_PX))
            .clamp(0.0, 1.0) as f32;
        let color = [0.55, 0.55, 0.6, 0.5 * fade];
        let a = cam.world_to_screen(min);
        let b = cam.world_to_screen(max);
        let mut kx = kx0;
        while kx <= kx1 {
            let sx = cam
                .world_to_screen(Pt::new(g.offset.x + kx * g.cell, 0.0))
                .x;
            items.push(DrawItem::Rect {
                x0: (sx - 0.5) as f32,
                y0: a.y as f32,
                x1: (sx + 0.5) as f32,
                y1: b.y as f32,
                color,
            });
            kx += 1.0;
        }
        let mut ky = ky0;
        while ky <= ky1 {
            let sy = cam
                .world_to_screen(Pt::new(0.0, g.offset.y + ky * g.cell))
                .y;
            items.push(DrawItem::Rect {
                x0: a.x as f32,
                y0: (sy - 0.5) as f32,
                x1: b.x as f32,
                y1: (sy + 0.5) as f32,
                color,
            });
            ky += 1.0;
        }
    }

    fn push_token(&self, items: &mut Vec<DrawItem>, t: &Token) {
        let cam = &self.camera;
        let s = cam.world_to_screen(self.display_pos(t));
        let r = (self.token_diameter(t) * 0.5 * cam.zoom).max(3.0) as f32;
        let (cx, cy) = (s.x as f32, s.y as f32);
        let mut color = color_for_id(&t.token_id);
        if t.hidden {
            color[3] = 0.5;
        }
        let health = HealthState::try_from(t.health_state).unwrap_or(HealthState::Unspecified);
        if health == HealthState::Down {
            color[3] *= DOWN_FADE;
        }
        let image = t.image.as_ref().and_then(|i| self.ready_image(&i.url));
        items.push(DrawItem::Disc {
            cx,
            cy,
            r,
            color: if image.is_some() {
                [1.0, 1.0, 1.0, color[3]]
            } else {
                color
            },
            image,
        });
        // The health ring takes the outline's place at the token's edge.
        let (thickness, ring_color) = match health {
            HealthState::Bloodied => (3.0, BLOODIED_COLOR),
            HealthState::NearDeath => (3.0, NEAR_DEATH_COLOR),
            HealthState::Down => (3.0, DOWN_COLOR),
            HealthState::Healthy | HealthState::Unspecified => (2.0, OUTLINE_COLOR),
        };
        items.push(DrawItem::Ring {
            cx,
            cy,
            r,
            thickness,
            color: ring_color,
        });
        let dragging = matches!(&self.interaction, Interaction::Drag { token_id, moved: true, .. } if *token_id == t.token_id);
        if dragging || self.selected.as_deref() == Some(t.token_id.as_str()) {
            items.push(DrawItem::Ring {
                cx,
                cy,
                r: r + 4.0,
                thickness: 3.0,
                color: [1.0, 0.85, 0.2, 1.0],
            });
        }
        if self.highlights.contains(&t.token_id) {
            items.push(DrawItem::Ring {
                cx,
                cy,
                r: r + 8.0,
                thickness: 3.0,
                color: HIGHLIGHT_COLOR,
            });
        }
        if !t.conditions.is_empty() {
            // Upper right, on the edge (45 degrees from the centre).
            let d = r * std::f32::consts::FRAC_1_SQRT_2;
            let (mx, my) = (cx + d, cy - d);
            items.push(DrawItem::Disc {
                cx: mx,
                cy: my,
                r: CONDITION_MARKER_PX,
                color: CONDITION_MARKER_COLOR,
                image: None,
            });
            items.push(DrawItem::Ring {
                cx: mx,
                cy: my,
                r: CONDITION_MARKER_PX,
                thickness: 1.5,
                color: OUTLINE_COLOR,
            });
        }
    }
}

// ------------------------------------------------------------------ helpers

/// Token width in grid cells for a creature size. `Unspecified` and unknown
/// values count as one cell; Tiny is half a cell.
pub fn size_cells(size: i32) -> f64 {
    match CreatureSize::try_from(size) {
        Ok(CreatureSize::Tiny) => 0.5,
        Ok(CreatureSize::Large) => 2.0,
        Ok(CreatureSize::Huge) => 3.0,
        Ok(CreatureSize::Gargantuan) => 4.0,
        _ => 1.0,
    }
}

/// Only http(s) and relative URLs of sane length are fetched.
pub fn loadable_url(url: &str) -> bool {
    if url.is_empty() || url.len() > MAX_URL_LEN || url.chars().any(char::is_control) {
        return false;
    }
    let lower = url.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return true;
    }
    // Anything else with a scheme (javascript:, data:, file:) is refused.
    match url.find([':', '/', '?', '#']) {
        Some(i) => url.as_bytes()[i] != b':',
        None => true,
    }
}

/// Label text: control characters removed, trimmed, at most `MAX_LABEL_CHARS`
/// characters; `None` if nothing is left.
fn clean_label(name: &str) -> Option<String> {
    let cleaned: String = name.chars().filter(|c| !c.is_control()).collect();
    let cleaned: String = cleaned.trim().chars().take(MAX_LABEL_CHARS).collect();
    (!cleaned.is_empty()).then_some(cleaned)
}

fn vec2_to_pt(v: &Vec2) -> Option<Pt> {
    let p = Pt::new(f64::from(v.x), f64::from(v.y));
    p.is_finite()
        .then(|| Pt::new(clamp_coord(p.x), clamp_coord(p.y)))
}

fn pt_to_vec2(p: Pt) -> Vec2 {
    Vec2 {
        x: p.x as f32,
        y: p.y as f32,
    }
}

/// Drop tokens with no id or a non-finite position; clamp the rest.
fn sanitize_token(mut t: Token) -> Option<Token> {
    if t.token_id.is_empty() {
        return None;
    }
    let pos = match t.position.take() {
        Some(v) => vec2_to_pt(&v)?,
        None => Pt::default(),
    };
    t.position = Some(pt_to_vec2(pos));
    Some(t)
}

fn encode_move(client_intent_id: String, token_id: String, level_id: String, dest: Pt) -> Vec<u8> {
    ClientMessage {
        kind: Some(client_message::Kind::Intent(Intent {
            client_intent_id,
            kind: Some(intent::Kind::MoveToken(MoveTokenIntent {
                token_id,
                path: vec![pt_to_vec2(dest)],
                level_id,
            })),
        })),
    }
    .encode_to_vec()
}

#[cfg(test)]
mod tests;
