//! The pure scene model: the client's view of the world, built from decoded
//! `Snapshot`s and `Event`s, plus camera, hit-testing, drag logic and the
//! per-frame draw list. No browser and no GPU here, so all of it is tested
//! natively. Nothing in this file may panic on any input.
//!
//! Units (see README): positions, grid sizes and map sizes are scene units
//! (pixels of the source map image). `Token.position` is the token's CENTRE.
//! A token is a disc of diameter `cells * Grid.cell_size`, with `cells` from
//! its `CreatureSize`. Pointer input is in CSS pixels relative to the canvas.

use std::collections::HashMap;

use prost::Message as _;
use tt_contracts::v1::{
    client_message, event, intent, server_message, ClientMessage, CreatureSize, Event, GridKind,
    Intent, Level, MoveTokenIntent, Role, Scene, ServerMessage, Snapshot, Token, Vec2,
};

use crate::camera::{clamp_coord, Camera, Pt};
use crate::draw::{color_for_id, DrawItem, DrawList};

/// Cell size used when a scene's grid size is missing or nonsense.
pub const DEFAULT_CELL: f64 = 70.0;
/// A move preview that the server has not answered within this long is dropped.
pub const PENDING_TIMEOUT_MS: f64 = 5000.0;
/// Pointer travel (CSS px) before a press on a token becomes a drag.
pub const DRAG_THRESHOLD_PX: f64 = 3.0;
/// Below this cell size on screen the grid is not drawn at all.
pub const MIN_GRID_CELL_PX: f64 = 6.0;
/// At or above this cell size on screen the grid is fully opaque.
pub const FULL_GRID_CELL_PX: f64 = 14.0;
/// Hard cap on grid lines per frame; over it the grid is skipped.
pub const MAX_GRID_LINES: usize = 1500;
/// Tokens are always at least this big (screen px radius) to hit-test.
pub const MIN_HIT_RADIUS_PX: f64 = 8.0;
const BACKGROUND: [f32; 4] = [0.09, 0.09, 0.11, 1.0];
const PLACEHOLDER: [f32; 4] = [0.15, 0.16, 0.19, 1.0];
const PLACEHOLDER_FAILED: [f32; 4] = [0.26, 0.12, 0.12, 1.0];
const MAX_URL_LEN: usize = 2048;

/// Load state of an image the model wants drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ImageState {
    Pending,
    Ready { width: u32, height: u32 },
    Failed,
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
    selected: Option<String>,
    interaction: Interaction,
    pending: Vec<Pending>,
    nonce: u64,
    next_intent: u64,
    last_rejection: Option<Rejection>,
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
            selected: None,
            interaction: Interaction::Idle,
            pending: Vec::new(),
            nonce,
            next_intent: 0,
            last_rejection: None,
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
                self.apply_rejection(&rej);
                Applied::Rejected(rej)
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
        self.last_seq = s.seq;
        self.selected = None;
        self.interaction = Interaction::Idle;
        self.pending.clear();
        if scene_changed {
            self.needs_fit = true;
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
                if matches!(&self.interaction, Interaction::Drag { token_id, .. } if *token_id == r.token_id)
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

    /// An intent was refused: drop its preview and remember why.
    pub fn apply_rejection(&mut self, r: &Rejection) {
        self.pending
            .retain(|p| p.client_intent_id != r.client_intent_id);
        self.last_rejection = Some(r.clone());
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
        {
            return;
        }
        let (min, max) = self.content_bounds();
        self.camera.fit(min, max, 24.0);
        self.needs_fit = false;
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

    // ---------------------------------------------------------------- input

    /// Pointer pressed at screen `(x, y)`. Button 0 on a token selects it and,
    /// if the viewer may move it, starts a drag. Anything else pans.
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
    /// the snapped destination.
    pub fn pointer_up(&mut self, x: f64, y: f64, now_ms: f64) -> Option<Vec<u8>> {
        self.pointer_move(x, y);
        let interaction = std::mem::replace(&mut self.interaction, Interaction::Idle);
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
    }

    pub fn image_state(&self, url: &str) -> Option<ImageState> {
        self.images.get(url).copied()
    }

    fn ready_image(&self, url: &str) -> Option<String> {
        matches!(self.images.get(url), Some(ImageState::Ready { .. })).then(|| url.to_owned())
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
        items.push(DrawItem::Ring {
            cx,
            cy,
            r,
            thickness: 2.0,
            color: [0.0, 0.0, 0.0, 0.6],
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
