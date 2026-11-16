//! Pan and zoom. Pure maths, no browser.
//!
//! Screen space is CSS pixels with the origin at the canvas top-left, y down.
//! World space is scene units (pixels of the source map image), y down.
//! `zoom` is screen pixels per world unit.

/// Smallest zoom: one map pixel is a fiftieth of a screen pixel.
pub const MIN_ZOOM: f64 = 0.02;
/// Largest zoom: one map pixel is sixteen screen pixels.
pub const MAX_ZOOM: f64 = 16.0;
/// World coordinates beyond this magnitude are treated as hostile and clamped.
pub const COORD_LIMIT: f64 = 1.0e7;

/// A point in either space (the caller knows which).
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Pt {
    pub x: f64,
    pub y: f64,
}

impl Pt {
    pub const fn new(x: f64, y: f64) -> Self {
        Pt { x, y }
    }

    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite()
    }
}

/// Clamp a coordinate into `[-COORD_LIMIT, COORD_LIMIT]`. Callers check
/// finiteness first; NaN maps to 0 so this can never propagate it.
pub fn clamp_coord(v: f64) -> f64 {
    if v.is_nan() {
        0.0
    } else {
        v.clamp(-COORD_LIMIT, COORD_LIMIT)
    }
}

/// The view onto the world.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    /// World point shown at the screen's top-left corner.
    pub origin: Pt,
    /// Screen pixels per world unit, always within `[MIN_ZOOM, MAX_ZOOM]`.
    pub zoom: f64,
    /// Viewport size in CSS pixels.
    pub width: f64,
    pub height: f64,
}

impl Default for Camera {
    fn default() -> Self {
        Camera {
            origin: Pt::new(0.0, 0.0),
            zoom: 1.0,
            width: 0.0,
            height: 0.0,
        }
    }
}

impl Camera {
    pub fn world_to_screen(&self, w: Pt) -> Pt {
        Pt::new(
            (w.x - self.origin.x) * self.zoom,
            (w.y - self.origin.y) * self.zoom,
        )
    }

    pub fn screen_to_world(&self, s: Pt) -> Pt {
        Pt::new(
            s.x / self.zoom + self.origin.x,
            s.y / self.zoom + self.origin.y,
        )
    }

    /// Set the viewport size. Non-finite or negative sizes are ignored.
    pub fn set_viewport(&mut self, width: f64, height: f64) {
        if width.is_finite() && height.is_finite() && width >= 0.0 && height >= 0.0 {
            self.width = width;
            self.height = height;
        }
    }

    /// Move the view so the world appears dragged by `(dx, dy)` screen pixels.
    pub fn pan_by_screen(&mut self, dx: f64, dy: f64) {
        if !(dx.is_finite() && dy.is_finite()) {
            return;
        }
        self.origin.x = clamp_coord(self.origin.x - dx / self.zoom);
        self.origin.y = clamp_coord(self.origin.y - dy / self.zoom);
    }

    /// Multiply the zoom by `factor` (clamped), keeping the world point under
    /// `screen` fixed on screen.
    pub fn zoom_about(&mut self, screen: Pt, factor: f64) {
        if !(screen.is_finite() && factor.is_finite() && factor > 0.0) {
            return;
        }
        self.set_zoom_about(screen, self.zoom * factor);
    }

    /// Set an absolute zoom (clamped) about a screen point.
    pub fn set_zoom_about(&mut self, screen: Pt, zoom: f64) {
        if !(screen.is_finite() && zoom.is_finite()) {
            return;
        }
        let anchor = self.screen_to_world(screen);
        self.zoom = zoom.clamp(MIN_ZOOM, MAX_ZOOM);
        self.origin = Pt::new(
            clamp_coord(anchor.x - screen.x / self.zoom),
            clamp_coord(anchor.y - screen.y / self.zoom),
        );
    }

    /// Frame the world rectangle `min..max` with a margin in screen pixels.
    /// Does nothing while the viewport has no area.
    pub fn fit(&mut self, min: Pt, max: Pt, margin_px: f64) {
        if !(min.is_finite() && max.is_finite()) || self.width <= 0.0 || self.height <= 0.0 {
            return;
        }
        let w = (max.x - min.x).max(1e-6);
        let h = (max.y - min.y).max(1e-6);
        let avail_w = (self.width - 2.0 * margin_px).max(1.0);
        let avail_h = (self.height - 2.0 * margin_px).max(1.0);
        self.zoom = (avail_w / w).min(avail_h / h).clamp(MIN_ZOOM, MAX_ZOOM);
        let cx = (min.x + max.x) * 0.5;
        let cy = (min.y + max.y) * 0.5;
        self.origin = Pt::new(
            clamp_coord(cx - self.width * 0.5 / self.zoom),
            clamp_coord(cy - self.height * 0.5 / self.zoom),
        );
    }

    /// The world rectangle currently on screen, as `(min, max)`.
    pub fn visible_world(&self) -> (Pt, Pt) {
        (
            self.screen_to_world(Pt::new(0.0, 0.0)),
            self.screen_to_world(Pt::new(self.width, self.height)),
        )
    }
}
