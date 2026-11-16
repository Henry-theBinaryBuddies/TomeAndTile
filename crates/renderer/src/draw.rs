//! The backend-independent draw list. Everything is in screen space (CSS
//! pixels, origin top-left, y down), in painter's order: later items cover
//! earlier ones. A backend turns this into GPU calls; tests inspect it.

/// Straight (non-premultiplied) RGBA, 0..1.
pub type Color = [f32; 4];

#[derive(Clone, Debug, PartialEq)]
pub enum DrawItem {
    /// An image stretched over a screen rectangle (the map).
    Image {
        url: String,
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
    },
    /// A flat rectangle (placeholders, grid lines).
    Rect {
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
        color: Color,
    },
    /// A filled disc, optionally textured with an image already loaded.
    Disc {
        cx: f32,
        cy: f32,
        r: f32,
        color: Color,
        image: Option<String>,
    },
    /// An outline of `thickness` pixels whose outer edge is at radius `r`.
    Ring {
        cx: f32,
        cy: f32,
        r: f32,
        thickness: f32,
        color: Color,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct DrawList {
    pub background: Color,
    pub items: Vec<DrawItem>,
}

/// A stable colour for an id: FNV-1a hash to a hue, fixed saturation and
/// lightness. The same id always gives the same colour.
pub fn color_for_id(id: &str) -> Color {
    let mut h: u32 = 0x811c_9dc5;
    for b in id.bytes() {
        h ^= u32::from(b);
        h = h.wrapping_mul(0x0100_0193);
    }
    // Spread the low entropy of short ids across the hue circle.
    let hue = (h ^ (h >> 16)) % 360;
    hsl_to_rgb(hue as f32, 0.55, 0.52)
}

fn hsl_to_rgb(h: f32, s: f32, l: f32) -> Color {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = h / 60.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    [r + m, g + m, b + m, 1.0]
}
