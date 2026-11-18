//! How sockets and nodes look: the colour and shape of what a socket
//! carries (today's colours for heights, masks and colours, Blender's for
//! the new types), and the colour of each family of nodes.

use crate::theme::{self, Palette};
use egui::{Color32, Painter, Pos2, Shape, Stroke, pos2, vec2};
use massif_terrain::registry::{Family, PortKind};

/// The colour of what a socket carries.
pub fn color(kind: PortKind) -> Color32 {
    match kind {
        PortKind::Height => Color32::from_rgb(176, 188, 206),
        PortKind::Mask => Color32::from_rgb(236, 192, 64),
        PortKind::Color => Color32::from_rgb(236, 96, 164),
        PortKind::Vector => Color32::from_rgb(99, 99, 199),
        PortKind::Terrain => Color32::from_rgb(99, 199, 99),
        PortKind::Biome => Color32::from_rgb(0, 214, 163),
    }
}

/// The colour as drawn on the panel: in high contrast, pushed until it
/// stands out from the node's fill.
pub fn color_on(kind: PortKind, pal: &Palette) -> Color32 {
    let c = color(kind);
    if pal.high { theme::legible(c, pal.panel, 3.0) } else { c }
}

/// The shape of a socket: circles carry continuous fields, squares
/// categories, diamonds settings driven by a field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape_ {
    Circle,
    Square,
    Diamond,
}

pub fn shape(kind: PortKind) -> Shape_ {
    if kind.categorical() { Shape_::Square } else { Shape_::Circle }
}

/// A socket at `c`, `r` points across its half; `filled` when connected
/// (an empty one is a ring, as in Blender's optional inputs).
pub fn draw(painter: &Painter, c: Pos2, r: f32, kind: PortKind, form: Shape_, filled: bool, pal: &Palette) {
    let fill = color_on(kind, pal);
    let rim = Stroke::new(1.0, if pal.dark { Color32::from_black_alpha(160) } else { Color32::from_black_alpha(90) });
    let inside = if filled { fill } else { theme::mix(pal.panel, fill, 0.25) };
    let ring = if filled { rim } else { Stroke::new(1.5, fill) };
    match form {
        Shape_::Circle => {
            painter.circle(c, r, inside, ring);
        }
        Shape_::Square => {
            let rect = egui::Rect::from_center_size(c, vec2(r * 1.8, r * 1.8));
            painter.rect(rect, 1.5, inside, ring, egui::StrokeKind::Middle);
        }
        Shape_::Diamond => {
            let pts = vec![pos2(c.x, c.y - r * 1.25), pos2(c.x + r * 1.25, c.y), pos2(c.x, c.y + r * 1.25), pos2(c.x - r * 1.25, c.y)];
            painter.add(Shape::convex_polygon(pts, inside, ring));
        }
    }
}

/// The colour of a family of nodes: the band across a node's top.
pub fn family(f: Family) -> Color32 {
    match f {
        Family::Input => Color32::from_rgb(150, 160, 178),
        Family::Primitive => Color32::from_rgb(52, 186, 176),
        Family::Shape => Color32::from_rgb(86, 196, 120),
        Family::Adjust => Color32::from_rgb(86, 146, 255),
        Family::Simulate => Color32::from_rgb(245, 146, 64),
        Family::Climate => Color32::from_rgb(92, 196, 236),
        Family::Mask => Color32::from_rgb(236, 192, 64),
        Family::Color => Color32::from_rgb(236, 96, 164),
        Family::Combine => Color32::from_rgb(168, 124, 255),
        Family::Output => Color32::from_rgb(244, 84, 104),
        Family::Internal => Color32::GRAY,
    }
}

/// The icon of a family, in the library and the search.
pub fn family_icon(f: Family) -> crate::theme::Icon {
    use crate::theme::Icon;
    match f {
        Family::Input => Icon::FileInput,
        Family::Primitive => Icon::Waves,
        Family::Shape => Icon::Mountain,
        Family::Adjust => Icon::SlidersHorizontal,
        Family::Simulate => Icon::Droplets,
        Family::Climate => Icon::CloudSun,
        Family::Mask => Icon::SquareDashed,
        Family::Color => Icon::Palette,
        Family::Combine => Icon::Layers,
        Family::Output => Icon::Flag,
        Family::Internal => Icon::Circle,
    }
}

/// The name of what a socket carries.
pub fn kind_label(kind: PortKind) -> &'static str {
    match kind {
        PortKind::Height => t!("Height"),
        PortKind::Mask => t!("Mask"),
        PortKind::Color => t!("Colour"),
        PortKind::Vector => t!("Vector"),
        PortKind::Terrain => t!("Terrain"),
        PortKind::Biome => t!("Biome"),
    }
}
