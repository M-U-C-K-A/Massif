//! The 2D map editor: navigation, brush strokes and overlays.

use crate::doc::Document;
use crate::map_render::{CHUNK_PX, ChunkKey, MAX_LOD, MapCache, MapMode, MapStyle};
use crate::theme::{self, Icon, Palette};
use crate::tools::{self, Stroke, Tool, ToolState};
use egui::{
    Align2, Color32, CursorIcon, FontId, PointerButton, Pos2, Rect, Sense, Stroke as Line,
    StrokeKind, Ui, Vec2, pos2, vec2,
};
use massif_core::{Brush, BrushShape, TILE_SIZE, TerrainId};
use std::time::Duration;

/// Samples across the picture of a brush's strength.
const STRENGTH_PX: usize = 128;
/// Strengths at which the brush preview draws a line.
const STRENGTH_LEVELS: [f32; 3] = [0.25, 0.5, 0.75];
/// The most loops of strength drawn at a level: a speckled image has its
/// strength shown by its shading alone.
const STRENGTH_RINGS: usize = 12;
/// How white the strength is drawn where the brush presses hardest, of 255:
/// light enough for the map to show through.
const STRENGTH_ALPHA: f32 = 58.0;
/// Most columns the strength is drawn for one by one, close up.
const BLOCK_STRENGTH_MAX: usize = 160 * 160;

/// How strongly a brush presses over its footprint: a picture, white where
/// it presses hardest, and the lines where its strength is a quarter, a
/// half and three quarters. Made again when the brush changes shape.
struct BrushPreview {
    /// Shape, profile, rotation and image the picture was made for.
    key: (BrushShape, massif_core::Profile, u32, usize),
    texture: egui::TextureHandle,
    /// Closed lines, in fractions of the footprint's square.
    levels: [Vec<Vec<(f32, f32)>>; 3],
}

impl BrushPreview {
    fn key(brush: &Brush) -> (BrushShape, massif_core::Profile, u32, usize) {
        let image = brush.mask.as_ref().map_or(0, |m| std::sync::Arc::as_ptr(m) as usize);
        (brush.shape, brush.profile, brush.rotation.to_bits(), image)
    }

    fn new(ctx: &egui::Context, brush: &Brush) -> Self {
        let n = STRENGTH_PX;
        // The picture does not depend on the size: any radius will do.
        let mut b = brush.clone();
        b.radius = n as f32 / 2.0;
        b.filter = None;
        let reach = b.reach();
        let weights: Vec<f32> = (0..n * n)
            .map(|c| {
                let at = |k: usize| ((k as f32 + 0.5) / n as f32 - 0.5) * 2.0 * reach;
                b.weight(at(c % n), at(c / n)).clamp(0.0, 1.0)
            })
            .collect();
        let bytes: Vec<u8> = weights.iter().flat_map(|&w| [(w * STRENGTH_ALPHA) as u8; 4]).collect();
        let image = egui::ColorImage::from_rgba_premultiplied([n, n], &bytes);
        let texture = ctx.load_texture("brush strength", image, egui::TextureOptions::LINEAR);
        // A brush that presses all or nothing (a flat profile, a hard-edged
        // image) has all its lines on its outline: none drawn.
        let flat = weights.iter().all(|&w| w <= 0.0 || w >= STRENGTH_LEVELS[2]);
        // Nor one of many specks: the lines would be a tangle around each
        // (its shading still shows how hard it presses).
        let levels = STRENGTH_LEVELS.map(|level| {
            if flat {
                return Vec::new();
            }
            let rings = massif_core::brush::trace_cells(n, n, |i, j| weights[j * n + i] >= level);
            if rings.len() > STRENGTH_RINGS {
                return Vec::new();
            }
            rings.into_iter().map(|ring| ease(&ring, n)).collect()
        });
        Self {
            key: Self::key(brush),
            texture,
            levels,
        }
    }
}

/// A ring of cell corners eased into a line, in fractions of `n` cells:
/// each corner cut by half a cell at most, so that the steps of a curve
/// round off while long straight sides stay straight (a square's lines stay
/// a square, not the diamond the middles of its sides would make).
fn ease(ring: &[(i32, i32)], n: usize) -> Vec<(f32, f32)> {
    let k = ring.len();
    let mut out = Vec::with_capacity(2 * k);
    // The point half a cell (at most halfway) from corner `at` toward `to`.
    let cut = |at: (i32, i32), to: (i32, i32)| {
        let (dx, dy) = ((to.0 - at.0) as f32, (to.1 - at.1) as f32);
        let t = (0.5 / dx.hypot(dy).max(1e-6)).min(0.5);
        ((at.0 as f32 + dx * t) / n as f32, (at.1 as f32 + dy * t) / n as f32)
    };
    for a in 0..k {
        let (before, at, after) = (ring[(a + k - 1) % k], ring[a], ring[(a + 1) % k]);
        out.push(cut(at, before));
        out.push(cut(at, after));
    }
    out
}

/// What the strength by block was drawn for: the brush (shape, profile,
/// rotation, image), its radius and the pointer.
type BlockStrengthKey = ((BrushShape, massif_core::Profile, u32, usize), u32, u32, u32);

const MIN_ZOOM: f32 = 1.0 / 2048.0;
const MAX_ZOOM: f32 = 48.0;
pub const FOOTER: f32 = 28.0;
/// How high over the bottom of the map the bar of a river or a road being
/// laid out reaches (its middle 30 points up, 40 high).
pub const DRAFT_BAR: f32 = 50.0;
/// The least room, in points each way, the map is framed by itself in.
const FIT_ROOM: f32 = 64.0;
/// Points of scrolling per brush size step (about one wheel notch).
const WHEEL_STEP: f32 = 20.0;
/// Degrees the brush turns per wheel notch (one with Shift).
const TURN_STEP: f32 = 5.0;
/// Points of scrolling per line, for wheels that count in lines.
const LINE_POINTS: f32 = 40.0;
/// Screen pixels per block from which the brush is outlined block by block.
const EXACT_ZOOM: f32 = 3.0;
/// Largest brush (half its footprint, in blocks) outlined block by block.
const EXACT_REACH: f32 = 320.0;

/// Tabs for the display modes, or a drop-down when space is short.
pub fn mode_selector(ui: &mut Ui, mode: &mut MapMode, narrow: bool, id: &str) {
    if narrow {
        crate::widgets::combo(id)
            .width(84.0)
            .selected_text(crate::i18n::tr_name(mode.label()))
            .show_ui(ui, |ui| {
                for m in MapMode::ALL {
                    ui.selectable_value(mode, m, crate::i18n::tr_name(m.label()));
                }
            });
    } else {
        for m in MapMode::ALL {
            if ui.selectable_label(*mode == m, crate::i18n::tr_name(m.label())).clicked() {
                *mode = m;
            }
        }
    }
}

/// Per-area state of a map editor.
/// How near a handle of a river the pointer takes hold of it, in points:
/// on the handle drawn, or just by it.
const HANDLE_REACH: f32 = 10.0;

/// What the pointer is over, on a river being laid out.
#[derive(Debug, Clone, Copy, PartialEq)]
enum RiverHit {
    /// One of its anchors: the source, a point it passes, its end.
    Anchor(usize),
    /// The end of the way it runs by itself.
    Mouth,
    /// Its course, at this point of it.
    Course(usize),
}

/// What the automatic tools would change under the brush: the new terrain
/// (or biome) of each column, drawn over the map before painting.
struct AutoPreview {
    key: (Tool, i32, i32, u64, u64),
    /// The footprint, in blocks: x0, z0, x1, z1.
    rect: [i32; 4],
    texture: Option<egui::TextureHandle>,
    /// What it says under the map: what the columns become, and how much
    /// of the brush stays as it is (Auto biome and Auto paint only).
    summary: Option<AutoSummary>,
}

/// The columns under the brush of an automatic tool, counted: what they
/// would become, and those it leaves as they are.
#[derive(Debug, Clone, Default, PartialEq)]
struct AutoSummary {
    /// Each biome (or terrain) given, with its columns, most first.
    becomes: Vec<(u16, usize)>,
    /// Columns under the brush that already have what the land calls for.
    same: usize,
}

impl AutoSummary {
    fn of(cols: &[massif_core::ops::Preview]) -> Self {
        let mut counts: std::collections::HashMap<u16, usize> = std::collections::HashMap::new();
        let mut same = 0;
        for c in cols {
            match c {
                massif_core::ops::Preview::Becomes(id) => *counts.entry(*id).or_default() += 1,
                massif_core::ops::Preview::Same => same += 1,
                massif_core::ops::Preview::Outside => {}
            }
        }
        let mut becomes: Vec<(u16, usize)> = counts.into_iter().collect();
        becomes.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        Self { becomes, same }
    }

    /// The share of the brush left as it is, 0 to 1.
    fn same_share(&self) -> f32 {
        let all = self.same + self.becomes.iter().map(|b| b.1).sum::<usize>();
        if all == 0 { 0.0 } else { self.same as f32 / all as f32 }
    }
}

/// What the automatic tools give the land, kept while the map, its rules
/// and the tool stay the same: Auto biome (or paint), the map and its
/// revision, a hash of the rules.
type AutoClasses = ((bool, u64, usize, u64, u64), massif_core::auto::Classes);

/// A hash of what the automatic tools read the land with: their rules,
/// and the terrain the map was filled with.
fn auto_rules_key(doc: &Document, tools: &ToolState) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    format!("{:?}", tools.auto).hash(&mut h);
    doc.auto_base().map(|t| t.0).hash(&mut h);
    h.finish()
}

/// What the tag of the water tools was worked out for: the column, the
/// map's state, the tool and its settings, a stroke's level.
/// What a water tool says by the cursor, and where Flood's water would
/// run down to.
type WaterTag = (String, Option<(i32, i32)>);
type WaterTagKey = ((i32, i32), (u64, usize, u32), Tool, (tools::WaterHeight, tools::WaterHeight, i16, i32), (bool, i16, bool), Option<i16>);

pub struct MapView {
    /// The tool last seen, to follow a change of it (see
    /// [`MapView::follow_tool`]).
    followed: Option<Tool>,
    /// The view the map was in when picking a tool changed it, and the one
    /// it changed it to: put back on leaving the tool, unless changed since.
    followed_from: Option<(MapMode, MapMode)>,
    /// Block coordinates at the centre of the view.
    pub center: [f64; 2],
    /// Screen points per block.
    pub zoom: f32,
    pub mode: MapMode,
    pub contours: bool,
    /// Shading from the smooth heights rather than whole blocks.
    pub smooth: bool,
    pub grid: bool,
    /// The map cut through at this height, its caves as a plan (see
    /// [`MapStyle::cut`]); `None`: seen from above.
    pub cut: Option<i32>,
    /// The height of the last cut, to come back to.
    pub cut_y: i32,
    /// The map framed by itself (see [`MapView::should_fit`]): for which
    /// document, and the view it gave, with the size of the canvas then.
    fitted: Option<u64>,
    fit_view: Option<([f64; 2], f32, Vec2)>,
    stroke: Option<Stroke>,
    /// Flood held down, and whether by the right button.
    water: Option<(tools::WaterHold, bool)>,
    /// What the water tool in hand would do under the cursor, and what it
    /// was worked out for.
    water_tag: Option<(WaterTagKey, Option<WaterTag>)>,
    last_time: f64,
    probe: Option<((i32, i32, u64), Option<tools::Probe>)>,
    /// Scrolling not yet turned into brush size steps.
    wheel: f32,
    /// Scrolling not yet turned into brush rotation steps.
    turn: f32,
    /// Where the canvas was last drawn, for keyboard zooming.
    canvas: Rect,
    /// The picture of the brush's strength, under the pointer.
    strength: Option<BrushPreview>,
    /// Close up, the strength column by column over the columns the brush
    /// changes, and their corners `[x0, z0, x1, z1]`.
    block_strength: Option<(BlockStrengthKey, egui::TextureHandle, [i32; 4])>,
    /// The anchor of the river being laid out that is being dragged, and
    /// where it was from the pointer when taken hold of, in points: held
    /// by its edge, it does not jump to the pointer.
    river_grab: Option<usize>,
    river_offset: Vec2,
    /// The bar over a river being laid out, and its Dig river and Cancel
    /// buttons, as last drawn.
    river_bar: Option<[Rect; 3]>,
    /// Where water let go under the pointer would go (River tool, before a
    /// source is placed): for the block and the document's revision it was
    /// found for.
    spring: Option<((i32, i32, u64), massif_core::river::Descent)>,
    /// The block the pointer rests on, and since when.
    spring_rest: Option<((i32, i32, u64), f64)>,
    auto_preview: Option<AutoPreview>,
    auto_classes: Option<AutoClasses>,
    /// Where the view is gliding to: a zoom (around a point of the screen)
    /// or a framing, reached over a few frames.
    goal: Option<Goal>,
    /// The speed of the last pan, in points a second, and when the pointer
    /// last moved it: let go while moving, the map glides on.
    velocity: Vec2,
    moved_at: f64,
    gliding: bool,
    /// The view of the last frame, to tell when it moves.
    last_view: ([f64; 2], f32),
    /// A spot pointed out from elsewhere (the survival check): where, and
    /// since when (`None` until it is first drawn).
    ping: Option<([f64; 2], Option<f64>)>,
    /// The selection: the shape being drawn, its outline.
    select: crate::select::SelectView,
    /// The mark of the 3D view is held: the 3D view follows the pointer.
    view3d_grab: bool,
    /// The terrain generator's Live preview, drawn over the map without
    /// writing it: each picture, its blocks (west, north, east, south) and
    /// the part of it to show, coarsest first (see `terrain_live`).
    pub live: Vec<(egui::TextureId, [i32; 4], Rect)>,
    /// What the preview says it is, on a badge in the corner.
    pub live_note: Option<String>,
}

/// What the view glides to.
#[derive(Debug, Clone, Copy)]
struct Goal {
    zoom: f32,
    /// The centre to reach, or the point of the screen that stays put.
    center: Option<[f64; 2]>,
    anchor: Pos2,
}

/// How quickly the view reaches its goal (per second): 90 % in about 150 ms.
const GLIDE_RATE: f32 = 15.0;
/// How long a fling lasts: its speed falls by e every this many seconds.
const FLING_DECAY: f32 = 0.25;

impl Default for MapView {
    fn default() -> Self {
        Self {
            followed: None,
            followed_from: None,
            center: [0.0, 0.0],
            zoom: 1.0,
            mode: MapMode::Terrain,
            contours: true,
            smooth: false,
            grid: false,
            cut: None,
            cut_y: 0,
            fitted: None,
            fit_view: None,
            stroke: None,
            water: None,
            water_tag: None,
            last_time: 0.0,
            probe: None,
            wheel: 0.0,
            turn: 0.0,
            strength: None,
            block_strength: None,
            canvas: Rect::NOTHING,
            river_grab: None,
            river_offset: Vec2::ZERO,
            river_bar: None,
            spring: None,
            spring_rest: None,
            auto_preview: None,
            auto_classes: None,
            goal: None,
            velocity: Vec2::ZERO,
            moved_at: 0.0,
            gliding: false,
            last_view: ([0.0, 0.0], 1.0),
            ping: None,
            select: Default::default(),
            view3d_grab: false,
            live: Vec::new(),
            live_note: None,
        }
    }
}

/// What a map editor needs from the application for one frame.
pub struct MapInput<'a> {
    pub doc: &'a mut Document,
    pub tools: &'a mut ToolState,
    pub cache: &'a mut MapCache,
    pub solo: bool,
    pub hover: &'a mut Option<(i32, i32)>,
    pub feedback: &'a mut Option<String>,
    /// Space is held: the primary button pans.
    pub pan_key: bool,
    /// Direction of the light of the shading (see [`MapStyle::light`]).
    pub light: u8,
    /// A picture over the map and its block coordinates (west, north, east,
    /// south): where a save point compared differs (see `versions`).
    pub overlay: Option<(egui::TextureHandle, [i32; 4])>,
    /// The 3D View, when the workspace shows one: its mark on the map, to
    /// drag (see [`MapView::view3d_mark`]).
    pub view3d: Option<&'a mut crate::view3d::View3d>,
    /// The reference images' textures and handles (the images are the
    /// map's).
    pub references: &'a mut crate::references::References,
    /// How the contour lines show.
    pub overlays: Overlays,
}

/// How the map views draw the contour lines: one choice for all of them,
/// kept in the settings. (Whether the annotations show is the tools'
/// [`ToolState::show_annotations`].)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Overlays {
    pub contour: crate::map_render::ContourStyle,
}

impl Overlays {
    /// The contour lines as drawn: in high contrast, strong whatever the
    /// setting.
    pub fn contour(&self, high_contrast: bool) -> crate::map_render::ContourStyle {
        let mut c = self.contour;
        if high_contrast {
            c.opacity = c.opacity.max(100);
        }
        c
    }

    /// The settings of the contour lines, in a menu.
    pub fn contour_settings(&mut self, ui: &mut Ui) {
        let c = &mut self.contour;
        ui.set_min_width(250.0);
        crate::widgets::well(ui, |ui| {
            crate::widgets::row(ui, tr!("Interval"), |ui| {
                ui.add(crate::widgets::Field::new(&mut c.interval, 1..=500).suffix(tr!(" blocks")).width(96.0));
            });
            crate::widgets::row(ui, "", |ui| {
                let labels: Vec<String> = crate::map_render::CONTOUR_INTERVALS.iter().map(|n| n.to_string()).collect();
                let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
                let on: Vec<bool> = crate::map_render::CONTOUR_INTERVALS.iter().map(|&n| n == c.interval).collect();
                if let Some(k) = crate::widgets::button_group(ui, "contour interval", &labels, &on) {
                    c.interval = crate::map_render::CONTOUR_INTERVALS[k];
                }
            });
            crate::widgets::row(ui, tr!("Strength"), |ui| {
                ui.add(crate::widgets::Field::new(&mut c.opacity, 10..=100).suffix("%").width(96.0));
            });
            crate::widgets::row(ui, tr!("Lines"), |ui| {
                if let Some(k) = crate::widgets::button_group(ui, "contour ink", &[tr!("Black"), tr!("White")], &[!c.white, c.white]) {
                    c.white = k == 1;
                }
            });
            crate::widgets::row(ui, tr!("Edges"), |ui| {
                if let Some(k) = crate::widgets::button_group(ui, "contour edges", &[tr!("Crisp"), tr!("Smooth")], &[!c.smooth, c.smooth]) {
                    c.smooth = k == 1;
                }
            });
            crate::widgets::row(ui, tr!("Fine lines between"), |ui| {
                crate::widgets::switch(ui, &mut c.fine);
            });
        });
        if ui.button(tr!("Reset")).clicked() {
            *c = Default::default();
        }
    }
}

/// "Solo: <name>" in the map's header while layers are shown alone (a
/// layer's or a palette's Alt-click on its eye): a pill in the accent;
/// returns whether it was clicked, to show every layer as before.
pub fn solo_chip(ui: &mut Ui, reg: &massif_core::registry::Registry) -> bool {
    let Some(solo) = &reg.solo else { return false };
    let pal = Palette::of(ui);
    let text = egui::RichText::new(format!("{}  {}  {}", theme::glyph(Icon::ScanEye), tr!("Solo: {name}", name = solo.name), theme::glyph(Icon::X))).size(11.5).color(pal.accent);
    ui.add(egui::Button::new(text).stroke(Line::new(1.0, pal.accent)).fill(pal.accent_soft).corner_radius(theme::PILL).truncate())
        .on_hover_text(tr!("Layers shown alone. Click to show every layer as before"))
        .clicked()
}

impl MapView {
    /// The view that shows what `tool` paints: the biomes for Paint biome
    /// and Auto biome, the terrains for Paint terrain and the Gradient
    /// brush.
    fn view_for(tool: Tool) -> Option<MapMode> {
        match tool {
            Tool::PaintBiome | Tool::AutoBiome => Some(MapMode::Biome),
            Tool::PaintTerrain | Tool::Gradient => Some(MapMode::Terrain),
            _ => None,
        }
    }

    /// Picking a tool that paints biomes shows the biomes, one that paints
    /// terrains the terrains (Settings ▸ Appearance ▸ The map's view
    /// follows the tool); leaving it puts back the view the map was in,
    /// unless it was changed meanwhile.
    pub fn follow_tool(&mut self, tool: Tool, on: bool) {
        let before = self.followed.replace(tool);
        if !on || before.is_none_or(|b| b == tool) {
            return;
        }
        match Self::view_for(tool) {
            Some(view) if view != self.mode => {
                let from = self.followed_from.map_or(self.mode, |(from, _)| from);
                self.followed_from = Some((from, view));
                self.mode = view;
            }
            Some(_) => {}
            None => {
                if let Some((from, set)) = self.followed_from.take()
                    && self.mode == set
                {
                    self.mode = from;
                }
            }
        }
    }

    /// Display-mode tabs and toggles, shown in the area header.
    pub fn header(&mut self, ui: &mut Ui, overlays: &mut Overlays, tools: &mut ToolState) {
        let narrow = ui.available_width() < 420.0;
        mode_selector(ui, &mut self.mode, narrow, "map mode");
        ui.separator();
        ui.toggle_value(&mut self.contours, theme::glyph(Icon::Spline).to_string())
            .on_hover_text(tr!("Contour lines"))
            .context_menu(|ui| overlays.contour_settings(ui));
        crate::widgets::menu_button(ui, theme::glyph(Icon::ChevronDown).to_string(), |ui| overlays.contour_settings(ui))
            .on_hover_text(tr!("Contour lines: interval, strength, colour"));
        let mut steps = !self.smooth;
        if ui
            .toggle_value(&mut steps, theme::glyph(Icon::Blocks).to_string())
            .on_hover_text(tr!("Block steps: each block shaded at its own height, as in the game. Off: a smooth shading of the ground's shape"))
            .changed()
        {
            self.smooth = !steps;
        }
        ui.toggle_value(&mut self.grid, theme::glyph(Icon::Grid3x3).to_string())
            .on_hover_text(tr!("Grid of chunks, tiles and region files (r.x.z.mca, 512 blocks)"));
        ui.toggle_value(&mut tools.show_annotations, theme::glyph(Icon::PenLine).to_string())
            .on_hover_text(tr!("Annotations, on the map and in 3D: the marks painted with the Annotations tool. They are never exported"));
        ui.separator();
        let mut under = self.cut.is_some();
        if ui
            .toggle_value(&mut under, theme::glyph(Icon::SquareSplitVertical).to_string())
            .on_hover_text(tr!("Underground: the map cut through at a height, its caves, water and lava as a plan"))
            .changed()
        {
            self.cut = under.then_some(self.cut_y);
        }
        if let Some(y) = &mut self.cut {
            ui.add(crate::widgets::Field::new(y, -2032..=2031).suffix(" y")).on_hover_text(tr!("The height of the cut"));
            self.cut_y = *y;
        }
    }

    /// What the pointer at `p` is over, on the river being laid out.
    fn river_hit(&self, canvas: Rect, draft: &tools::RiverDraft, settings: &tools::RiverSettings, p: Pos2) -> Option<RiverHit> {
        let screen = |q: (f32, f32)| self.to_screen(canvas, q.0 as f64, q.1 as f64);
        let anchors = draft.layout.anchors();
        for (k, &a) in anchors.iter().enumerate().rev() {
            if screen(a).distance(p) <= HANDLE_REACH {
                return Some(RiverHit::Anchor(k));
            }
        }
        if draft.layout.end.is_none()
            && let Some(&m) = draft.plan.course.last()
            && screen(m).distance(p) <= HANDLE_REACH
        {
            return Some(RiverHit::Mouth);
        }
        let reach = (settings.width * (1.0 + 2.0 * settings.flow) * self.zoom / 2.0).max(7.0);
        let mut best: Option<(usize, f32)> = None;
        for (i, &c) in draft.plan.course.iter().enumerate() {
            let d = screen(c).distance(p);
            if d <= reach && best.is_none_or(|b| d < b.1) {
                best = Some((i, d));
            }
        }
        best.map(|(i, _)| RiverHit::Course(i))
    }

    /// Draw a river's course on the map: its banks, its water, the way it
    /// flows, its lakes; and the handles of a river being laid out.
    fn draw_river(
        &self,
        painter: &egui::Painter,
        canvas: Rect,
        plan: &massif_core::river::Descent,
        spec: &massif_core::river::RiverSpec,
        handles: Option<(&tools::RiverDraft, Option<RiverHit>, Option<usize>)>,
    ) {
        let water = Color32::from_rgb(56, 146, 255);
        for (cells, _) in &plan.lakes {
            let mut rows: Vec<(i32, i32)> = cells.iter().map(|&(x, z)| (z, x)).collect();
            rows.sort_unstable();
            let mut k = 0;
            while k < rows.len() {
                let (z, x0) = rows[k];
                let mut x1 = x0;
                while k + 1 < rows.len() && rows[k + 1] == (z, x1 + 1) {
                    k += 1;
                    x1 += 1;
                }
                let a = self.to_screen(canvas, x0 as f64, z as f64);
                let b = self.to_screen(canvas, x1 as f64 + 1.0, z as f64 + 1.0);
                painter.rect_filled(Rect::from_min_max(a, b), 0, water.gamma_multiply(0.5));
                k += 1;
            }
        }
        let n = plan.course.len();
        if n >= 2 {
            // Points a pixel and a half apart at least, with how far down
            // the river they are.
            let mut pts: Vec<(Pos2, f32)> = Vec::with_capacity(n.min(4096));
            for (k, &(x, z)) in plan.course.iter().enumerate() {
                let p = self.to_screen(canvas, x as f64, z as f64);
                if k + 1 == n || pts.last().is_none_or(|q| q.0.distance(p) >= 1.5) {
                    pts.push((p, k as f32 / (n - 1) as f32));
                }
            }
            let width = |t: f32| (spec.width + (spec.width_end - spec.width) * t) * self.zoom;
            // In pieces of steady width, the width growing downstream.
            let pieces: Vec<&[(Pos2, f32)]> = pts.chunks(24).collect();
            let joined = |k: usize| -> Vec<Pos2> {
                let mut v: Vec<Pos2> = pieces[k].iter().map(|p| p.0).collect();
                if let Some(next) = pieces.get(k + 1).and_then(|p| p.first()) {
                    v.push(next.0);
                }
                v
            };
            for (layer, color) in [
                (0, water.gamma_multiply(0.14)),
                (1, Color32::from_rgba_unmultiplied(6, 24, 56, 170)),
                (2, water.gamma_multiply(0.92)),
            ] {
                for k in 0..pieces.len() {
                    let t = pieces[k][pieces[k].len() / 2].1;
                    let w = width(t);
                    let stroke = match layer {
                        0 => w + 2.0 * spec.banks * self.zoom,
                        1 => w.max(2.5) + 2.0,
                        _ => w.max(2.5),
                    };
                    let line = joined(k);
                    if line.len() >= 2 {
                        painter.add(egui::Shape::line(line, Line::new(stroke, color)));
                    }
                }
            }
            // Which way it flows: a chevron every so often.
            let mut run = 0.0;
            for w in pts.windows(2) {
                let (a, b) = (w[0].0, w[1].0);
                let d = a.distance(b);
                run += d;
                if run >= 120.0 && d > 0.5 {
                    run = 0.0;
                    let dir = (b - a) / d;
                    let side = vec2(-dir.y, dir.x);
                    let tip = b + dir * 3.0;
                    let ink = Line::new(1.6, Color32::from_white_alpha(210));
                    painter.line_segment([tip - dir * 6.0 + side * 4.5, tip], ink);
                    painter.line_segment([tip - dir * 6.0 - side * 4.5, tip], ink);
                }
            }
        }
        let Some((draft, hover, grab)) = handles else {
            if let Some(&(x, z)) = plan.course.first() {
                let p = self.to_screen(canvas, x as f64, z as f64);
                painter.circle(p, 6.0, water, Line::new(2.0, Color32::WHITE));
            }
            return;
        };
        let lit = |k: usize| hover == Some(RiverHit::Anchor(k)) || grab == Some(k);
        let anchors = draft.layout.anchors();
        let last = anchors.len() - 1;
        if draft.layout.end.is_none()
            && let Some(&(x, z)) = plan.course.last()
        {
            // Where it runs to by itself: a ring to take hold of.
            let p = self.to_screen(canvas, x as f64, z as f64);
            let r = if hover == Some(RiverHit::Mouth) { 7.5 } else { 5.5 };
            painter.circle_stroke(p, r + 1.0, Line::new(3.0, Color32::from_black_alpha(90)));
            painter.circle_stroke(p, r, Line::new(2.0, Color32::WHITE));
        }
        // Each point numbered in the order the river runs through them:
        // the source blue, the points it passes white, the end it was
        // given dark.
        let deep = Color32::from_rgb(6, 24, 56);
        for (k, &(x, z)) in anchors.iter().enumerate() {
            let p = self.to_screen(canvas, x as f64, z as f64);
            let r = if lit(k) { 9.5 } else { 8.0 };
            painter.circle_filled(p + vec2(0.0, 1.0), r + 1.5, Color32::from_black_alpha(70));
            let (fill, ring, ink) = if k == 0 {
                (water, Color32::WHITE, Color32::WHITE)
            } else if k == last && draft.layout.end.is_some() {
                (deep, water, Color32::WHITE)
            } else {
                (Color32::WHITE, water, deep)
            };
            painter.circle(p, r, fill, Line::new(2.0, ring));
            painter.text(p + vec2(0.0, 0.5), Align2::CENTER_CENTER, (k + 1).to_string(), FontId::proportional(if k >= 9 { 9.0 } else { 10.5 }), ink);
        }
    }

    /// The bar over a river being laid out: how long it is and where it
    /// goes, and the buttons to dig it or drop it.
    #[allow(clippy::too_many_arguments)]
    fn river_bar(&mut self, ui: &mut Ui, painter: &egui::Painter, canvas: Rect, id: egui::Id, doc: &mut Document, tools: &mut ToolState, feedback: &mut Option<String>) {
        self.river_bar = None;
        let Some(draft) = &tools.river_draft else { return };
        let font = FontId::proportional(12.5);
        let text = painter.layout_no_wrap(draft.summary(), font.clone(), Color32::from_gray(225));
        let dig = painter.layout_no_wrap(tr!("Dig river").to_owned(), FontId::proportional(12.5), Color32::WHITE);
        let enter = painter.layout_no_wrap("⏎".to_owned(), FontId::proportional(11.0), Color32::from_white_alpha(170));
        let cancel = painter.layout_no_wrap(tr!("Cancel").to_owned(), font, Color32::from_gray(210));
        let esc = painter.layout_no_wrap("esc".to_owned(), FontId::proportional(10.5), Color32::from_gray(130));
        let (pad, gap, h) = (12.0, 8.0, 28.0);
        let dig_w = dig.size().x + enter.size().x + 26.0;
        let cancel_w = cancel.size().x + esc.size().x + 24.0;
        let width = pad + text.size().x + 16.0 + dig_w + gap + cancel_w + 6.0;
        let bar = Rect::from_center_size(canvas.center_bottom() - vec2(0.0, 30.0), vec2(width, h + 12.0));
        painter.rect_filled(bar.translate(vec2(0.0, 3.0)).expand(1.0), 12, Color32::from_black_alpha(70));
        painter.rect(bar, 12, Color32::from_rgba_unmultiplied(19, 21, 26, 242), Line::new(1.0, Color32::from_white_alpha(20)), StrokeKind::Inside);
        let mid = bar.center().y;
        painter.galley(pos2(bar.min.x + pad, mid - text.size().y / 2.0), text.clone(), Color32::from_gray(225));
        let dig_rect = Rect::from_min_size(pos2(bar.min.x + pad + text.size().x + 16.0, mid - h / 2.0), vec2(dig_w, h));
        let cancel_rect = Rect::from_min_size(pos2(dig_rect.max.x + gap, mid - h / 2.0), vec2(cancel_w, h));
        // The bar takes the pointer, under its buttons too: a click or a
        // drag begun on it never reaches the map (a button only takes
        // clicks, and its drag would go to the map under it).
        ui.interact(bar, id.with("river bar"), Sense::click_and_drag());
        self.river_bar = Some([bar, dig_rect, cancel_rect]);
        let dig_hit = ui.interact(dig_rect, id.with("dig river"), Sense::click()).on_hover_text(tr!("Dig it into the map (Enter)"));
        let cancel_hit = ui.interact(cancel_rect, id.with("drop river"), Sense::click()).on_hover_text(tr!("Drop this river (Esc)"));
        let accent = theme::accent();
        painter.rect_filled(dig_rect, 8, if dig_hit.hovered() { theme::mix(accent, Color32::WHITE, 0.15) } else { accent });
        painter.galley(pos2(dig_rect.min.x + 12.0, mid - dig.size().y / 2.0), dig, Color32::WHITE);
        painter.galley(pos2(dig_rect.max.x - 12.0 - enter.size().x, mid - enter.size().y / 2.0), enter, Color32::from_white_alpha(170));
        if cancel_hit.hovered() {
            painter.rect_filled(cancel_rect, 8, Color32::from_white_alpha(16));
        }
        painter.galley(pos2(cancel_rect.min.x + 10.0, mid - cancel.size().y / 2.0), cancel, Color32::from_gray(210));
        painter.galley(pos2(cancel_rect.max.x - 10.0 - esc.size().x, mid - esc.size().y / 2.0 + 0.5), esc, Color32::from_gray(130));
        if dig_hit.hovered() || cancel_hit.hovered() {
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        }
        if dig_hit.clicked() {
            *feedback = tools::dig_draft(doc, tools);
            self.river_grab = None;
        } else if cancel_hit.clicked() {
            tools.river_draft = None;
            self.river_grab = None;
            *feedback = Some(tr!("River dropped").into());
        }
    }

    /// Draw what Auto paint or Auto biome would change under the brush at
    /// `at`: the columns that would change, in their new colour.
    #[allow(clippy::too_many_arguments)]
    fn show_auto_preview(&mut self, ctx: &egui::Context, painter: &egui::Painter, land: &[egui::Painter], canvas: Rect, doc: &Document, tools: &ToolState, at: (f64, f64)) {
        use std::hash::{Hash, Hasher};
        let biomes = tools.tool == Tool::AutoBiome;
        let base = doc.auto_base();
        let rules = auto_rules_key(doc, tools);
        // A column a pixel at most: a preview need not be finer than the
        // screen, nor wider than 1024 columns.
        let brush = tools.stroke_brush();
        let side = 2.0 * brush.radius + 2.0;
        let per_pixel = 1.0 / (self.zoom * ctx.pixels_per_point()).max(1e-3);
        let step = (per_pixel.floor() as i32).max((side / 1024.0).ceil() as i32).max(1);
        let mut h = std::collections::hash_map::DefaultHasher::new();
        rules.hash(&mut h);
        BrushPreview::key(&tools.brush).hash(&mut h);
        tools.brush.radius.to_bits().hash(&mut h);
        tools.brush.strength.to_bits().hash(&mut h);
        step.hash(&mut h);
        let key = (tools.tool, at.0.floor() as i32, at.1.floor() as i32, doc.revision, h.finish());
        if self.auto_preview.as_ref().is_none_or(|a| a.key != key) {
            let reader = massif_core::auto::Reader::new(&tools.auto, &doc.world.registry, doc.world.meta.sea_level).with_base(base);
            // What the land calls for, read once for the map as it is.
            let classes_key = (biomes, doc.generation, doc.dim, doc.revision, rules);
            if self.auto_classes.as_ref().is_none_or(|(k, _)| *k != classes_key) {
                self.auto_classes = Some((classes_key, massif_core::auto::Classes::new(biomes)));
            }
            let classes = &mut self.auto_classes.as_mut().expect("just set").1;
            let (rect, cols) = massif_core::ops::auto_preview_ensured(doc.dimension(), &brush, at.0 as f32, at.1 as f32, &reader, classes, step);
            let reg = &doc.world.registry;
            let size = [((rect.width() + step - 1) / step) as usize, ((rect.depth() + step - 1) / step) as usize];
            // What changes, in its new colour; what already is as the land
            // calls for, hatched, so that the whole brush shows and it is
            // plain that the part left grey is not forgotten.
            let colour = |k: usize, p: &massif_core::ops::Preview| match p {
                massif_core::ops::Preview::Outside => Color32::TRANSPARENT,
                massif_core::ops::Preview::Same => {
                    let (i, j) = (k % size[0].max(1), k / size[0].max(1));
                    Color32::from_white_alpha(if (i + j) % 4 == 0 { 120 } else { 34 })
                }
                massif_core::ops::Preview::Becomes(id) => {
                    let c = if biomes { reg.biomes.get(*id as usize).map(|b| b.color) } else { reg.terrains.get(*id as usize).map(|t| t.color) };
                    let [r, g, b] = c.unwrap_or([255, 0, 255]);
                    Color32::from_rgb(r, g, b)
                }
            };
            let summary = AutoSummary::of(&cols);
            let texture = (!summary.becomes.is_empty() || summary.same > 0).then(|| {
                let pixels = cols.iter().enumerate().map(|(k, p)| colour(k, p)).collect();
                let image = egui::ColorImage::new(size, pixels);
                ctx.load_texture("auto preview", image, egui::TextureOptions::NEAREST)
            });
            self.auto_preview = Some(AutoPreview {
                key,
                rect: [rect.x0, rect.z0, rect.x0 + size[0] as i32 * step, rect.z0 + size[1] as i32 * step],
                texture,
                summary: Some(summary),
            });
        }
        if let Some(AutoPreview { rect, texture: Some(texture), .. }) = &self.auto_preview {
            let min = self.to_screen(canvas, rect[0] as f64, rect[1] as f64);
            let max = self.to_screen(canvas, rect[2] as f64, rect[3] as f64);
            let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
            for p in land {
                p.image(texture.id(), Rect::from_min_max(min, max), uv, Color32::WHITE);
            }
        }
        if let Some(summary) = self.auto_preview.as_ref().and_then(|a| a.summary.as_ref()) {
            Self::auto_legend(painter, canvas, doc, biomes, summary);
        }
    }

    /// Under the map, what the brush of Auto biome (or Auto paint) would
    /// do where it is: the biomes it gives, each with its colour, and how
    /// much it leaves as it is because the land has what it calls for.
    fn auto_legend(painter: &egui::Painter, canvas: Rect, doc: &Document, biomes: bool, summary: &AutoSummary) {
        let reg = &doc.world.registry;
        let text = Color32::from_gray(235);
        let weak = Color32::from_gray(165);
        let font = FontId::proportional(12.5);
        // Swatch and name of the (up to) three given most.
        let mut parts: Vec<(Option<Color32>, std::sync::Arc<egui::Galley>)> = Vec::new();
        let named = |id: u16| {
            if biomes {
                reg.biomes.get(id as usize).map(|b| (b.color, b.name.clone()))
            } else {
                reg.terrains.get(id as usize).map(|t| (t.color, t.name.clone()))
            }
        };
        if summary.becomes.is_empty() {
            let note = if biomes { tr!("Nothing to change here: the land already has the biomes it calls for") } else { tr!("Nothing to change here: the land already has the terrains it calls for") };
            parts.push((None, painter.layout_no_wrap(note.to_owned(), font.clone(), text)));
        } else {
            parts.push((None, painter.layout_no_wrap(tr!("Becomes").to_owned(), font.clone(), weak)));
            for &(id, _) in summary.becomes.iter().take(3) {
                if let Some((rgb, name)) = named(id) {
                    parts.push((Some(theme::rgb(rgb)), painter.layout_no_wrap(name, font.clone(), text)));
                }
            }
            if summary.becomes.len() > 3 {
                parts.push((None, painter.layout_no_wrap(tr!("+{n} more", n = summary.becomes.len() - 3), font.clone(), weak)));
            }
            let share = summary.same_share();
            if share >= 0.005 {
                let note = tr!("· hatched, {percent:.0}%: already as the land calls for", percent = share * 100.0);
                parts.push((None, painter.layout_no_wrap(note, font.clone(), weak)));
            }
        }
        let gap = 8.0;
        let swatch = 10.0;
        let width: f32 = parts.iter().map(|(c, g)| g.size().x + if c.is_some() { swatch + 5.0 } else { 0.0 }).sum::<f32>() + gap * (parts.len() as f32 - 1.0);
        let height = parts.iter().map(|(_, g)| g.size().y).fold(0.0, f32::max);
        let at = canvas.center_bottom() - vec2(0.0, 22.0);
        let back = Rect::from_center_size(at, vec2(width, height) + vec2(24.0, 12.0));
        painter.rect_filled(back.translate(vec2(0.0, 2.0)), 9, Color32::from_black_alpha(60));
        painter.rect(back, 9, Color32::from_rgba_unmultiplied(20, 22, 27, 235), Line::new(1.0, Color32::from_white_alpha(18)), StrokeKind::Inside);
        let mut x = back.min.x + 12.0;
        let mid = back.center().y;
        for (c, g) in parts {
            if let Some(c) = c {
                let r = Rect::from_center_size(pos2(x + swatch / 2.0, mid), Vec2::splat(swatch));
                painter.rect(r, 3, c, Line::new(1.0, Color32::from_white_alpha(60)), StrokeKind::Inside);
                x += swatch + 5.0;
            }
            let w = g.size().x;
            painter.galley(pos2(x, mid - g.size().y / 2.0), g, text);
            x += w + gap;
        }
    }

    /// Draw what the noise of the Paint tool would paint under the brush at
    /// `at`, in the colours of its terrains.
    fn show_pattern_preview(&mut self, ctx: &egui::Context, land: &[egui::Painter], canvas: Rect, doc: &Document, tools: &ToolState, at: (f64, f64)) {
        use std::hash::{Hash, Hasher};
        /// The most samples a side: a big brush is shown sampled.
        const SAMPLES: usize = 256;
        let mut h = std::collections::hash_map::DefaultHasher::new();
        format!("{:?}{:?}", tools.pattern.pattern, tools.pattern.terrains).hash(&mut h);
        format!("{:?}", tools.filter).hash(&mut h);
        BrushPreview::key(&tools.brush).hash(&mut h);
        tools.brush.radius.to_bits().hash(&mut h);
        let key = (Tool::PaintTerrain, at.0.floor() as i32, at.1.floor() as i32, doc.revision, h.finish());
        if self.auto_preview.as_ref().is_none_or(|a| a.key != key) {
            let reg = &doc.world.registry;
            let texture = tools.pattern.sampler(reg).and_then(|sampler| {
                let preview = massif_core::ops::pattern_preview(doc.dimension(), &tools.stroke_brush(), at.0 as f32, at.1 as f32, &sampler, SAMPLES);
                let pixels: Vec<Color32> = preview
                    .cells
                    .iter()
                    .map(|c| c.and_then(|t| reg.terrains.get(t.0 as usize)).map_or(Color32::TRANSPARENT, |t| theme::rgb(t.color)))
                    .collect();
                let rect = preview.rect;
                let covered = [rect.x0 + preview.size[0] as i32 * preview.step, rect.z0 + preview.size[1] as i32 * preview.step];
                pixels.iter().any(|c| c.a() > 0).then(|| {
                    let image = egui::ColorImage::new(preview.size, pixels);
                    ([rect.x0, rect.z0, covered[0], covered[1]], ctx.load_texture("noise preview", image, egui::TextureOptions::NEAREST))
                })
            });
            self.auto_preview = Some(AutoPreview {
                key,
                rect: texture.as_ref().map_or([0; 4], |t| t.0),
                texture: texture.map(|t| t.1),
                summary: None,
            });
        }
        if let Some(AutoPreview { rect, texture: Some(texture), .. }) = &self.auto_preview {
            let min = self.to_screen(canvas, rect[0] as f64, rect[1] as f64);
            let max = self.to_screen(canvas, rect[2] as f64, rect[3] as f64);
            let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
            // Nearly opaque: what the ground will look like, the map just
            // showing through.
            for p in land {
                p.image(texture.id(), Rect::from_min_max(min, max), uv, Color32::from_white_alpha(225));
            }
        }
    }

    /// The strength of the brush centred on `(x, z)` column by column, a
    /// picture over the columns it changes: the texture and its corners.
    /// `None` for a footprint too large to be worth it.
    fn block_strength(&mut self, ctx: &egui::Context, brush: &Brush, x: f64, z: f64) -> Option<(egui::TextureId, [i32; 4])> {
        let (cx, cz) = (x as f32, z as f32);
        let rect = massif_core::coords::BlockRect::around(cx, cz, brush.reach());
        let (w, d) = (rect.width().max(0) as usize, rect.depth().max(0) as usize);
        if w * d == 0 || w * d > BLOCK_STRENGTH_MAX {
            return None;
        }
        let key = (BrushPreview::key(brush), brush.radius.to_bits(), cx.to_bits(), cz.to_bits());
        let corners = [rect.x0, rect.z0, rect.x1, rect.z1];
        if self.block_strength.as_ref().is_none_or(|b| b.0 != key) {
            // As the tools weigh a column: at its centre.
            let pixels: Vec<Color32> = (0..w * d)
                .map(|k| {
                    let (i, j) = ((k % w) as i32, (k / w) as i32);
                    let weight = brush.weight((rect.x0 + i) as f32 + 0.5 - cx, (rect.z0 + j) as f32 + 0.5 - cz).clamp(0.0, 1.0);
                    let a = (weight * STRENGTH_ALPHA) as u8;
                    Color32::from_rgba_premultiplied(a, a, a, a)
                })
                .collect();
            let image = egui::ColorImage::new([w, d], pixels);
            match &mut self.block_strength {
                Some((k, texture, c)) => {
                    texture.set(image, egui::TextureOptions::NEAREST);
                    (*k, *c) = (key, corners);
                }
                None => self.block_strength = Some((key, ctx.load_texture("brush strength by block", image, egui::TextureOptions::NEAREST), corners)),
            }
        }
        self.block_strength.as_ref().map(|(_, texture, c)| (texture.id(), *c))
    }

    /// The block position under screen point `p` when it is over the map,
    /// as the last frame drew it.
    pub fn world_under(&self, p: Pos2) -> Option<(f64, f64)> {
        self.canvas.contains(p).then(|| self.to_world(self.canvas, p))
    }

    /// Where block coordinates `(x, z)` were on the screen, last frame.
    #[cfg(test)]
    pub fn screen_of(&self, x: f64, z: f64) -> Pos2 {
        self.to_screen(self.canvas, x, z)
    }

    /// The block position under screen point `p`, as the last frame drew
    /// the map.
    #[cfg(test)]
    pub fn world_at(&self, p: Pos2) -> (f64, f64) {
        self.to_world(self.canvas, p)
    }

    /// The tag a water tool shows by the cursor, and where Flood's water
    /// would run down to.
    #[cfg(test)]
    pub fn water_tag_shown(&self) -> Option<WaterTag> {
        self.water_tag.as_ref().and_then(|t| t.1.clone())
    }

    /// The map's area on screen, under its bar.
    #[cfg(test)]
    pub fn canvas(&self) -> Rect {
        self.canvas
    }

    /// The bar over a river being laid out, its Dig river button and its
    /// Cancel button, as last drawn.
    #[cfg(test)]
    pub fn river_bar_rects(&self) -> Option<[Rect; 3]> {
        self.river_bar
    }

    /// The blocks the view showed last frame (west, north, east, south),
    /// and its zoom in screen points a block; `None` before it is drawn.
    /// (The Live preview asks for the tiles of that view.)
    #[allow(dead_code)]
    pub fn visible(&self) -> Option<([f64; 4], f32)> {
        self.canvas.is_positive().then(|| {
            let (x0, z0) = self.to_world(self.canvas, self.canvas.min);
            let (x1, z1) = self.to_world(self.canvas, self.canvas.max);
            ([x0, z0, x1, z1], self.zoom)
        })
    }

    fn to_screen(&self, canvas: Rect, x: f64, z: f64) -> Pos2 {
        let c = canvas.center();
        pos2(
            c.x + ((x - self.center[0]) * self.zoom as f64) as f32,
            c.y + ((z - self.center[1]) * self.zoom as f64) as f32,
        )
    }

    fn to_world(&self, canvas: Rect, p: Pos2) -> (f64, f64) {
        let c = canvas.center();
        (
            self.center[0] + ((p.x - c.x) / self.zoom) as f64,
            self.center[1] + ((p.y - c.y) / self.zoom) as f64,
        )
    }

    /// The land of the map within `area` (points), as rectangles of the
    /// screen: its tiles, those side by side in a row joined, rows alike
    /// joined (one rectangle for a rectangular map). What the brush's fills,
    /// hatching and noise and the selection's ants are clipped to, so that
    /// none is drawn over the void round the map.
    fn land_rects(&self, canvas: Rect, doc: &Document, area: Rect) -> Vec<Rect> {
        let area = area.intersect(canvas);
        let level = doc.dimension().ground();
        if !area.is_positive() || level.is_empty() {
            return Vec::new();
        }
        let t = TILE_SIZE as f64;
        let (x0, z0) = self.to_world(canvas, area.min);
        let (x1, z1) = self.to_world(canvas, area.max);
        let (tx0, tz0, tx1, tz1) = ((x0 / t).floor() as i32, (z0 / t).floor() as i32, (x1 / t).floor() as i32, (z1 / t).floor() as i32);
        let screen = |[a, b, c, d]: [i32; 4]| Rect::from_min_max(self.to_screen(canvas, a as f64 * t, b as f64 * t), self.to_screen(canvas, c as f64 * t, d as f64 * t)).intersect(area);
        // Too many tiles to look at one by one, far out over a large map:
        // the map's bounds.
        if (tx1 - tx0 + 1) as i64 * (tz1 - tz0 + 1) as i64 > 1 << 14 {
            return level.bounds().map(|(lo, hi)| screen([lo.x, lo.z, hi.x + 1, hi.z + 1])).filter(|r| r.is_positive()).into_iter().collect();
        }
        let mut out: Vec<[i32; 4]> = Vec::new();
        let mut last: Vec<(i32, i32)> = Vec::new();
        let mut last_at = 0;
        for tz in tz0..=tz1 {
            let mut runs = Vec::new();
            let mut tx = tx0;
            while tx <= tx1 {
                if level.contains(massif_core::coords::TilePos::new(tx, tz)) {
                    let a = tx;
                    while tx <= tx1 && level.contains(massif_core::coords::TilePos::new(tx, tz)) {
                        tx += 1;
                    }
                    runs.push((a, tx));
                } else {
                    tx += 1;
                }
            }
            if !runs.is_empty() && runs == last && out[last_at..].iter().all(|r| r[3] == tz) {
                for r in &mut out[last_at..] {
                    r[3] = tz + 1;
                }
            } else {
                last_at = out.len();
                out.extend(runs.iter().map(|&(a, b)| [a, tz, b, tz + 1]));
            }
            last = runs;
        }
        out.into_iter().map(screen).filter(|r| r.is_positive()).collect()
    }

    fn zoom_at(&mut self, canvas: Rect, p: Pos2, factor: f32) {
        let before = self.to_world(canvas, p);
        self.zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        let c = canvas.center();
        self.center[0] = before.0 - ((p.x - c.x) / self.zoom) as f64;
        self.center[1] = before.1 - ((p.y - c.y) / self.zoom) as f64;
    }

    /// Zoom around the centre of the view, at once.
    #[cfg(test)]
    pub fn zoom_by(&mut self, factor: f32) {
        let canvas = self.canvas;
        if canvas.is_positive() {
            self.zoom_at(canvas, canvas.center(), factor);
        } else {
            self.zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        }
    }

    /// Glide to `factor` times the zoom (on top of a glide under way),
    /// around the centre of the view: the zoom buttons and keys.
    pub fn zoom_smoothly(&mut self, factor: f32) {
        let from = self.goal.filter(|g| g.center.is_none()).map_or(self.zoom, |g| g.zoom);
        self.goal = Some(Goal {
            zoom: (from * factor).clamp(MIN_ZOOM, MAX_ZOOM),
            center: None,
            anchor: self.canvas.center(),
        });
        self.gliding = false;
    }

    /// Glide to block `(x, z)`, close enough to see the land around it,
    /// and point it out with rings for a moment.
    pub fn show_spot(&mut self, x: f64, z: f64) {
        self.goal = Some(Goal {
            zoom: self.zoom.clamp(2.0, 6.0),
            center: Some([x, z]),
            anchor: self.canvas.center(),
        });
        self.velocity = Vec2::ZERO;
        self.gliding = false;
        self.ping = Some(([x, z], None));
    }

    /// The 3D View's mark on the screen: where it stands and the way it
    /// looks.
    fn view3d_mark(&self, canvas: Rect, v: &crate::view3d::View3d) -> (Pos2, Vec2, bool) {
        let (at, dir, player) = v.map_marker();
        (self.to_screen(canvas, at[0], at[1]), vec2(dir[0], dir[1]), player)
    }

    /// Whether `p` is over the 3D View's mark.
    fn on_view3d_mark(&self, canvas: Rect, v: &crate::view3d::View3d, p: Pos2) -> bool {
        let (at, _, _) = self.view3d_mark(canvas, v);
        canvas.contains(at) && p.distance(at) <= 11.0
    }

    /// The 3D View's mark: a dot in the accent, in a dark outline, with a
    /// wedge the way it looks; a ring round it for the player.
    fn draw_view3d_mark(&self, painter: &egui::Painter, canvas: Rect, v: &crate::view3d::View3d, pal: Palette, hot: bool) {
        let (at, dir, player) = self.view3d_mark(canvas, v);
        if !canvas.expand(16.0).contains(at) {
            return;
        }
        let side = vec2(-dir.y, dir.x);
        let r = if hot { 6.5 } else { 5.5 };
        // The wedge: a field of view of about 70°, 28 points long.
        let tip = |k: f32| at + (dir * 0.82 + side * k * 0.57).normalized() * 28.0;
        let wedge = vec![at, tip(-1.0), tip(-0.5), tip(0.0), tip(0.5), tip(1.0)];
        painter.add(egui::Shape::convex_polygon(wedge, pal.accent.gamma_multiply(0.45), Line::new(1.5, Color32::WHITE.gamma_multiply(0.85))));
        if player {
            painter.circle_stroke(at, r + 3.5, Line::new(3.0, Color32::from_black_alpha(140)));
            painter.circle_stroke(at, r + 3.5, Line::new(1.5, pal.accent));
        }
        painter.circle(at, r, pal.accent, Line::new(2.0, Color32::WHITE));
        painter.circle_stroke(at, r + 1.5, Line::new(1.0, Color32::from_black_alpha(140)));
    }

    /// One block per point.
    pub fn zoom_reset(&mut self) {
        self.goal = Some(Goal {
            zoom: 1.0,
            center: None,
            anchor: self.canvas.center(),
        });
    }

    /// Whether to frame the map by itself on `canvas`: a map just opened
    /// (another document), once the canvas has room to show it — framed on
    /// a canvas a few points high, it came out at 1 % and stayed so —, or
    /// the canvas grown well past the size it was framed on while the view
    /// is still as that framing left it.
    pub fn should_fit(&self, generation: u64, canvas: Vec2) -> bool {
        if canvas.x < FIT_ROOM || canvas.y < FIT_ROOM {
            return false;
        }
        if self.fitted != Some(generation) {
            return true;
        }
        self.goal.is_none() && self.fit_view.is_some_and(|(c, z, size)| c == self.center && z == self.zoom && (canvas.x > size.x * 1.5 || canvas.y > size.y * 1.5))
    }

    /// Glide to the whole map.
    pub fn fit_smoothly(&mut self, doc: &Document) {
        let (center, zoom) = (self.center, self.zoom);
        self.fit(doc, self.canvas);
        self.goal = Some(Goal {
            zoom: self.zoom,
            center: Some(self.center),
            anchor: self.canvas.center(),
        });
        (self.center, self.zoom) = (center, zoom);
    }

    /// A step toward the goal, `dt` seconds on (all the way when motion is
    /// reduced). Whether it goes on.
    fn glide(&mut self, canvas: Rect, dt: f32, still: bool) -> bool {
        let Some(mut goal) = self.goal else { return false };
        // Asked before the view was ever drawn: around its middle.
        if !goal.anchor.is_finite() {
            goal.anchor = canvas.center();
        }
        let t = if still { 1.0 } else { 1.0 - (-dt * GLIDE_RATE).exp() };
        let factor = (goal.zoom / self.zoom).powf(t);
        match goal.center {
            Some(c) => {
                self.zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
                for (v, g) in self.center.iter_mut().zip(c) {
                    *v += (g - *v) * t as f64;
                }
            }
            None => self.zoom_at(canvas, goal.anchor, factor),
        }
        let near = (goal.zoom / self.zoom).ln().abs() < 0.002
            && goal.center.is_none_or(|c| ((c[0] - self.center[0]).hypot(c[1] - self.center[1]) * self.zoom as f64) < 0.3);
        if near {
            match goal.center {
                Some(c) => (self.center, self.zoom) = (c, goal.zoom),
                None => self.zoom_at(canvas, goal.anchor, goal.zoom / self.zoom),
            }
            self.goal = None;
        }
        !near
    }

    /// Glide to the blocks of `rect` (west, north, east, south), framed
    /// with room around them.
    pub fn glide_to(&mut self, rect: [i32; 4]) {
        let (w, d) = ((rect[2] - rect[0]).max(1) as f32, (rect[3] - rect[1]).max(1) as f32);
        let canvas = if self.canvas.is_finite() && self.canvas.width() > 1.0 { self.canvas } else { Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0)) };
        let zoom = ((canvas.width() / w).min(canvas.height() / d) * 0.6).clamp(MIN_ZOOM, MAX_ZOOM);
        self.goal = Some(Goal {
            zoom,
            center: Some([(rect[0] + rect[2]) as f64 / 2.0, (rect[1] + rect[3]) as f64 / 2.0]),
            anchor: canvas.center(),
        });
    }

    /// Frame the whole map.
    pub fn fit(&mut self, doc: &Document, canvas: Rect) {
        let Some((lo, hi)) = doc.dimension().bounds() else {
            self.center = [0.0, 0.0];
            self.zoom = 1.0;
            return;
        };
        let t = TILE_SIZE as f64;
        let (x0, z0) = (lo.x as f64 * t, lo.z as f64 * t);
        let (x1, z1) = ((hi.x + 1) as f64 * t, (hi.z + 1) as f64 * t);
        self.center = [(x0 + x1) / 2.0, (z0 + z1) / 2.0];
        let fit = (canvas.width() / (x1 - x0) as f32).min(canvas.height() / (z1 - z0) as f32);
        self.zoom = (fit * 0.92).clamp(MIN_ZOOM, MAX_ZOOM);
    }

    pub fn ui(&mut self, ui: &mut Ui, input: MapInput) {
        let pal = Palette::of(ui);
        let full = ui.available_rect_before_wrap();
        let canvas = Rect::from_min_max(full.min, pos2(full.max.x, full.max.y - FOOTER));
        let footer = Rect::from_min_max(pos2(full.min.x, canvas.max.y), full.max);
        let response = ui.allocate_rect(canvas, Sense::click_and_drag());
        ui.advance_cursor_after_rect(full);
        let MapInput {
            doc,
            tools,
            cache,
            solo,
            hover,
            feedback,
            pan_key,
            light,
            overlay,
            mut view3d,
            references,
            overlays,
        } = input;
        // The reference images read, those just added placed.
        {
            let map = doc.dimension().bounds().map(|(lo, hi)| {
                let t = TILE_SIZE as f64;
                [lo.x as f64 * t, lo.z as f64 * t, (hi.x + 1) as f64 * t, (hi.z + 1) as f64 * t]
            });
            let dir = doc.path.as_ref().and_then(|p| p.parent()).map(|p| p.to_path_buf());
            if references.update(ui.ctx(), &mut doc.world.meta.references, dir.as_deref(), map) {
                doc.mark_changed();
            }
        }

        if self.should_fit(doc.generation, canvas.size()) {
            self.fit(doc, canvas);
            self.goal = None;
            self.fitted = Some(doc.generation);
            self.fit_view = Some((self.center, self.zoom, canvas.size()));
        }

        // ---- navigation, the WorldPainter way ----------------------------
        // The wheel sizes the brush (Shift: one block at a time); with ⌘ (the
        // Windows key of a PC keyboard) it turns the brush, with Ctrl or Alt,
        // or with a pinch, it zooms. The middle button (or Space, or the Pan
        // tool) moves the map.
        let (time, modifiers, pointer) = ui.input(|i| (i.time, i.modifiers, i.pointer.interact_pos()));
        let (pressed, down) = ui.input(|i| {
            let p = &i.pointer;
            (
                [p.button_pressed(PointerButton::Primary), p.button_pressed(PointerButton::Secondary)],
                [p.button_down(PointerButton::Primary), p.button_down(PointerButton::Secondary)],
            )
        });
        let dt = (time - self.last_time).clamp(0.0, 0.1) as f32;
        self.last_time = time;
        self.canvas = canvas;
        // Reduce motion (Settings ▸ Accessibility): no gliding on, zooms
        // straight to where they go.
        let still = crate::theme::reduce_motion(ui.ctx());
        let panning = tools.tool == Tool::Pan || pan_key;
        // The handles of the reference images: with the Pan tool, or while
        // one is placed (Edit placement). They take the press before the
        // tool, and before the Pan tool's drag.
        references.pan = tools.tool == Tool::Pan;
        let mut placed = false;
        let on_reference = self.stroke.is_none()
            && !self.view3d_grab
            && tools.filter_pick.is_none()
            && tools.region_pick.is_none()
            && references.interact(
                ui,
                &mut doc.world.meta.references,
                pointer.filter(|_| response.hovered() || down[0]),
                pressed[0] && response.is_pointer_button_down_on(),
                down[0],
                &|x, z| self.to_screen(canvas, x, z),
                &|p| self.to_world(canvas, p),
                &mut placed,
            );
        if placed {
            doc.mark_changed();
        }
        let pan_drag = response.dragged_by(PointerButton::Middle) || (panning && !on_reference && response.dragged_by(PointerButton::Primary) && self.stroke.is_none());
        if pan_drag {
            let d = response.drag_delta();
            self.center[0] -= (d.x / self.zoom) as f64;
            self.center[1] -= (d.y / self.zoom) as f64;
            // The speed of the hand, smoothed over the last frames.
            if d != Vec2::ZERO && dt > 0.0 {
                self.velocity = self.velocity * 0.5 + (d / dt.max(1.0 / 240.0)) * 0.5;
                self.moved_at = time;
            }
            self.goal = None;
            self.gliding = false;
        } else if response.drag_stopped() && !still && self.velocity.length() > 150.0 && time - self.moved_at < 0.06 {
            // Let go while moving: the map goes on, slowing down.
            self.gliding = true;
            self.velocity *= (5000.0 / self.velocity.length()).min(1.0);
        } else if !response.dragged() {
            self.velocity = Vec2::ZERO;
        }
        if pressed.iter().any(|p| *p) || ui.input(|i| i.pointer.button_pressed(PointerButton::Middle)) {
            self.gliding = false;
        }
        if self.gliding {
            self.center[0] -= (self.velocity.x * dt / self.zoom) as f64;
            self.center[1] -= (self.velocity.y * dt / self.zoom) as f64;
            self.velocity *= (-dt / FLING_DECAY).exp();
            if self.velocity.length() < 20.0 {
                self.gliding = false;
                self.velocity = Vec2::ZERO;
            }
            ui.ctx().request_repaint();
        }
        if self.glide(canvas, dt, still) {
            ui.ctx().request_repaint();
        }
        if response.hovered()
            && let Some(p) = response.hover_pos()
        {
            if turns_brush(modifiers) && tools.tool.uses_brush() {
                // egui reads ⌘ and the wheel as a zoom: take the wheel itself.
                let height = canvas.height();
                self.turn += ui.input(|i| {
                    i.events
                        .iter()
                        .filter_map(|e| match e {
                            egui::Event::MouseWheel { unit, delta, .. } => {
                                let k = match unit {
                                    egui::MouseWheelUnit::Point => 1.0,
                                    egui::MouseWheelUnit::Line => LINE_POINTS,
                                    egui::MouseWheelUnit::Page => height,
                                };
                                Some(if delta.y != 0.0 { delta.y } else { delta.x } * k)
                            }
                            _ => None,
                        })
                        .sum::<f32>()
                });
                let step = if modifiers.shift && cfg!(target_os = "macos") { 1.0 } else { TURN_STEP };
                while self.turn.abs() >= WHEEL_STEP {
                    // Scrolling up turns clockwise, like a knob.
                    let up = self.turn > 0.0;
                    self.turn -= WHEEL_STEP.copysign(self.turn);
                    tools.brush.rotate(if up { step } else { -step });
                }
            } else {
                self.turn = 0.0;
                let (zoom_delta, scroll) = ui.input(|i| (i.zoom_delta(), i.smooth_scroll_delta));
                if zoom_delta != 1.0 {
                    self.goal = None;
                    self.zoom_at(canvas, p, zoom_delta);
                } else if scroll != Vec2::ZERO {
                    // Shift turns vertical scrolling into horizontal on some systems.
                    let amount = if scroll.y != 0.0 { scroll.y } else { scroll.x };
                    if modifiers.ctrl || modifiers.command {
                        self.goal = None;
                        self.zoom_at(canvas, p, (amount * 0.004).exp());
                    } else if tools.tool.uses_brush() {
                        self.wheel += amount;
                        while self.wheel.abs() >= WHEEL_STEP {
                            // Scrolling up makes the brush smaller, as in WorldPainter.
                            let up = self.wheel > 0.0;
                            self.wheel -= WHEEL_STEP.copysign(self.wheel);
                            let r = tools.brush.radius;
                            let next = match (modifiers.shift, up) {
                                (true, true) => r - 1.0,
                                (true, false) => r + 1.0,
                                (false, true) => (r / 1.1).min(r - 1.0),
                                (false, false) => (r * 1.1).max(r + 1.0),
                            };
                            tools.brush.radius = next.round().clamp(1.0, massif_core::MAX_RADIUS);
                        }
                    }
                }
            }
        }

        // ---- editing -------------------------------------------------------
        // The left button applies the tool, the right button its inverse:
        // lower instead of raise, erase instead of paint, drain instead of
        // flood. Alt + click picks what is under the cursor.
        let world_pointer = pointer.map(|p| self.to_world(canvas, p));
        // The mark of the 3D view, taken with the primary button: the 3D
        // view follows it rather than the tool acting.
        let on_mark = match (&view3d, pointer) {
            (Some(v), Some(p)) if response.hovered() && !panning => self.on_view3d_mark(canvas, v, p),
            _ => false,
        };
        if pressed[0] && on_mark && self.stroke.is_none() && response.is_pointer_button_down_on() {
            self.view3d_grab = true;
        }
        if self.view3d_grab {
            match (view3d.as_deref_mut(), world_pointer) {
                (Some(v), Some((x, z))) if down[0] => v.go_to(doc, x, z, false),
                _ => self.view3d_grab = false,
            }
        }
        if on_mark || self.view3d_grab {
            ui.ctx().set_cursor_icon(if self.view3d_grab { CursorIcon::Grabbing } else { CursorIcon::Grab });
        }
        if let Some((x, z)) = world_pointer.filter(|_| response.hovered()) {
            *hover = Some((x.floor() as i32, z.floor() as i32));
        }
        let button = if pressed[0] && !panning {
            Some(PointerButton::Primary)
        } else if pressed[1] {
            Some(PointerButton::Secondary)
        } else {
            None
        };
        // A press let go within the same frame, on a slow one, is a click
        // all the same.
        if let Some(button) = button
            && (response.is_pointer_button_down_on() || response.clicked_by(button))
            && self.stroke.is_none()
            && self.water.is_none()
            && !self.view3d_grab
            && !(on_reference && button == PointerButton::Primary)
            && let Some((x, z)) = world_pointer
        {
            let secondary = button == PointerButton::Secondary;
            let inverse = secondary;
            let (bx, bz) = (x.floor() as i32, z.floor() as i32);
            if let Some(pick) = &mut tools.region_pick {
                // Test Export's regions: a click takes or leaves one.
                if !secondary {
                    pick.press(bx, bz);
                }
            } else if tools.filter_pick.is_some() {
                // The filters' eyedropper: what is here is offered to pick
                // from, the tool does nothing.
                if !secondary && let Some(p) = pointer {
                    tools.filter_pick_at = Some(((bx, bz), p));
                }
            } else if modifiers.alt {
                // Alt + click, either button: what is under the cursor, as
                // WorldPainter's eyedropper.
                *feedback = tools::pick(doc, tools, bx, bz).or(feedback.take());
            } else if tools.tool == Tool::Eyedropper && modifiers.shift {
                // Shift: everything here at once, back to the tool before.
                *feedback = tools::pick_all(doc, tools, bx, bz).or(feedback.take());
            } else if tools.tool == Tool::Annotate && tools.annotation_curve {
                if let Some(p) = pointer {
                    let screen = |q: [f32; 2]| self.to_screen(canvas, q[0] as f64, q[1] as f64);
                    *feedback = crate::curve_draft::press(tools, [x as f32, z as f32], p, secondary, &screen, self.zoom).or(feedback.take());
                }
            } else if tools.tool == Tool::Road {
                if let Some(p) = pointer {
                    let screen = |q: (f32, f32)| self.to_screen(canvas, q.0 as f64, q.1 as f64);
                    *feedback = crate::road_draft::press(doc, tools, (x as f32, z as f32), p, secondary, &screen, self.zoom).or(feedback.take());
                }
            } else if tools.tool == Tool::River {
                let at = (x as f32, z as f32);
                let hit = match (&tools.river_draft, pointer) {
                    (Some(draft), Some(p)) => self.river_hit(canvas, draft, &tools.river, p),
                    _ => None,
                };
                self.river_offset = Vec2::ZERO;
                match (&mut tools.river_draft, hit, secondary) {
                    // The first click: where it springs (dragged, it moves).
                    (None, _, false) => {
                        *feedback = tools::start_river(doc, tools, at.0, at.1);
                        if tools.river_draft.is_some() {
                            self.river_grab = Some(0);
                        }
                    }
                    // A point taken hold of: dragged, it moves by as much as
                    // the pointer, not to it.
                    (Some(draft), Some(RiverHit::Anchor(k)), false) => {
                        if let (Some(&a), Some(p)) = (draft.layout.anchors().get(k), pointer) {
                            self.river_offset = self.to_screen(canvas, a.0 as f64, a.1 as f64) - p;
                        }
                        self.river_grab = Some(k);
                    }
                    // A point taken away.
                    (Some(draft), Some(RiverHit::Anchor(k)), true) => {
                        if tools::remove_river_point(draft, k) {
                            draft.plan(doc, &tools.river, None);
                        } else {
                            tools.river_draft = None;
                            *feedback = Some(tr!("River dropped").into());
                        }
                    }
                    // Where it runs to by itself, held: its end, to move.
                    (Some(draft), Some(RiverHit::Mouth), false) => {
                        let mouth = draft.plan.course.last().copied().unwrap_or(at);
                        if let Some(p) = pointer {
                            self.river_offset = self.to_screen(canvas, mouth.0 as f64, mouth.1 as f64) - p;
                        }
                        draft.layout.end = Some(mouth);
                        self.river_grab = Some(draft.layout.anchors().len() - 1);
                    }
                    // The course itself: a point it must pass, between the
                    // two points that leg of it joins, where clicked.
                    (Some(draft), Some(RiverHit::Course(i)), false) => {
                        let leg = draft.stops.iter().rposition(|&s| s <= i).unwrap_or(0);
                        let k = leg.min(draft.layout.through.len());
                        draft.layout.through.insert(k, at);
                        draft.plan(doc, &tools.river, Some(k + 1));
                        self.river_grab = Some(k + 1);
                    }
                    // Anywhere else: a point more, where clicked, the last
                    // one: the river ends there.
                    (Some(draft), None, false) => {
                        if let Some(end) = draft.layout.end.replace(at) {
                            draft.layout.through.push(end);
                        }
                        let k = draft.layout.anchors().len() - 1;
                        draft.plan(doc, &tools.river, Some(k));
                        self.river_grab = Some(k);
                    }
                    _ => {}
                }
            } else if tools.tool.is_select() {
                let op = crate::select::combine_of(modifiers, secondary, tools.select_mode);
                self.select.begin(tools.tool, (x, z), op, secondary, &tools.brush);
            } else if tools.tool == Tool::Flood {
                // The right button lowers the water a block, Shift drains
                // it whole; held, it keeps going.
                let (hold, note) = tools::press_water(doc, tools, bx, bz, inverse, secondary && modifiers.shift);
                *feedback = note.or(feedback.take());
                self.water = hold.map(|h| (h, secondary));
            } else if tools.tool == Tool::Structure && !secondary {
                // Structures: a click or a drag, on a structure placed or
                // on free land.
                if let Some(p) = pointer {
                    let screen = |q: (f32, f32)| self.to_screen(canvas, q.0 as f64, q.1 as f64);
                    *feedback = crate::structure_tool::press(doc, tools, (x, z), p, &screen).or(feedback.take());
                }
            } else if tools.tool.is_click() {
                *feedback = tools::click(doc, tools, bx, bz, inverse);
            } else if tools.tool.uses_brush() {
                let (stroke, msg) = tools::begin_stroke(doc, tools, (x as f32, z as f32), inverse, secondary);
                *feedback = msg.or(feedback.take());
                if let Some(stroke) = stroke {
                    // What the preview read of the land, the stroke need not
                    // read again.
                    let mut stroke = stroke.with_classes(self.auto_classes.take().filter(|(k, _)| k.1 == doc.generation && k.2 == doc.dim && k.3 == doc.revision).map(|(_, c)| c));
                    tools::continue_stroke(doc, tools, &mut stroke, (x as f32, z as f32), 1.0 / 60.0);
                    if let Some(note) = stroke.take_note() {
                        *feedback = Some(note);
                    }
                    self.stroke = Some(stroke);
                }
            }
        }
        // Delete or Backspace over the map: the structure selected goes.
        if tools.tool == Tool::Structure
            && response.hovered()
            && let Some(id) = tools.structure.selected
            && ui.ctx().memory(|m| m.focused().is_none())
            && ui.input(|i| i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace))
        {
            *feedback = crate::structure_tool::remove(doc, tools, id).or(feedback.take());
        }
        // A drag of the Structures tool follows the pointer; let go, it
        // places, moves or resizes.
        if tools.structure.rt.gesture().is_some() {
            if down[0] && tools.tool == Tool::Structure {
                if let Some(at) = world_pointer {
                    crate::structure_tool::drag(tools, at);
                }
                ui.ctx().request_repaint();
            } else {
                *feedback = crate::structure_tool::release(doc, tools).or(feedback.take());
            }
        }
        // A held anchor of the river follows the pointer; let go, its
        // course is worked out finely.
        if let Some(k) = self.river_grab {
            match tools.river_draft.as_mut() {
                Some(draft) if tools.tool == Tool::River => {
                    if down[0] {
                        if let Some(p) = pointer {
                            let at = self.to_world(canvas, p + self.river_offset);
                            let at = (at.0 as f32, at.1 as f32);
                            if draft.layout.anchors().get(k) != Some(&at) {
                                draft.layout.set(k, at);
                                draft.plan(doc, &tools.river, Some(k));
                            }
                        }
                        ui.ctx().request_repaint();
                    } else {
                        self.river_grab = None;
                        draft.plan(doc, &tools.river, None);
                    }
                }
                _ => self.river_grab = None,
            }
        }
        if tools.tool == Tool::Annotate
            && tools.annotation_curve
            && let Some(msg) = crate::curve_draft::follow(doc, tools, world_pointer.map(|(x, z)| [x as f32, z as f32]), down[0], ui)
        {
            *feedback = Some(msg);
        }
        if tools.tool == Tool::Road
            && let Some(msg) = crate::road_draft::follow(doc, tools, world_pointer.map(|(x, z)| (x as f32, z as f32)), down[0], ui)
        {
            *feedback = Some(msg);
        }
        if tools.tool == Tool::River
            && tools.river_draft.is_some()
            && !ui.ctx().egui_wants_keyboard_input()
        {
            let (enter, escape) = (ui.input(|i| i.key_pressed(egui::Key::Enter)), crate::keymap::peek(ui.ctx(), crate::keymap::Action::Cancel));
            if enter {
                *feedback = tools::dig_draft(doc, tools);
            } else if escape {
                tools.river_draft = None;
                self.river_grab = None;
                *feedback = Some(tr!("River dropped").into());
            }
        }
        if crate::keymap::peek(ui.ctx(), crate::keymap::Action::Cancel)
            && let Some(stroke) = self.stroke.take()
        {
            // Escape abandons the stroke being drawn.
            let _ = tools::end_stroke(doc, stroke);
            doc.undo();
            *feedback = Some(tr!("Stroke cancelled").into());
        }
        // Test Export's regions: a drag goes on taking or leaving them.
        if let Some(pick) = &mut tools.region_pick {
            match world_pointer {
                Some((x, z)) if down[0] && pick.drag.is_some() => pick.press(x.floor() as i32, z.floor() as i32),
                _ if !down[0] => pick.drag = None,
                _ => {}
            }
        }
        // The Pencil follows every place the pointer went through
        // this frame, not only the last.
        let passed: Vec<(f64, f64)> = if tools.pencil {
            ui.input(|i| i.events.iter().filter_map(|e| if let egui::Event::PointerMoved(p) = e { Some(*p) } else { None }).collect::<Vec<_>>()).into_iter().map(|p| self.to_world(canvas, p)).collect()
        } else {
            Vec::new()
        };
        if let Some(stroke) = &mut self.stroke {
            let held = if stroke.secondary() { down[1] } else { down[0] };
            if held {
                let parts = passed.len().max(1) as f32;
                for (x, z) in passed.iter().take(passed.len().saturating_sub(1)) {
                    tools::continue_stroke(doc, tools, stroke, (*x as f32, *z as f32), dt / parts);
                }
                if let Some((x, z)) = world_pointer {
                    tools::continue_stroke(doc, tools, stroke, (x as f32, z as f32), dt / parts);
                    if let Some(note) = stroke.take_note() {
                        *feedback = Some(note);
                    }
                }
                ui.ctx().request_repaint();
            } else if let Some(mut stroke) = self.stroke.take() {
                // Biomes do not change what the land calls for: what Auto
                // biome read still holds once it has painted them.
                let classes = stroke.take_classes().filter(|c| c.biomes());
                if let Some(note) = tools::end_stroke(doc, stroke) {
                    *feedback = Some(note);
                }
                if let Some(classes) = classes {
                    self.auto_classes = Some(((true, doc.generation, doc.dim, doc.revision, auto_rules_key(doc, tools)), classes));
                }
            }
        }
        // Flood held: a block more at each beat, until let go.
        if let Some((hold, secondary)) = &mut self.water {
            let held = if *secondary { down[1] } else { down[0] };
            if held && tools.tool == Tool::Flood {
                tools::hold_water(doc, tools, hold, dt);
                if !hold.ended() {
                    ui.ctx().request_repaint();
                }
            } else if let Some((hold, _)) = self.water.take() {
                *feedback = tools::release_water(doc, hold).or(feedback.take());
            }
        }

        // A selection being drawn follows the pointer; let go, it is made.
        // Esc drops the one being drawn, or else, with a selection tool in
        // hand, clears the selection.
        if !self.select.is_drawing()
            && tools.tool.is_select()
            && doc.selection().is_some()
            && crate::keymap::peek(ui.ctx(), crate::keymap::Action::Cancel)
            && crate::select::deselect(doc)
        {
            *feedback = Some(tr!("Deselected").into());
        }
        if self.select.is_drawing() {
            if crate::keymap::peek(ui.ctx(), crate::keymap::Action::Cancel) {
                self.select.cancel();
                *feedback = Some(tr!("Selection cancelled").into());
            } else if down[0] || down[1] {
                if let Some(at) = world_pointer {
                    self.select.update(at, self.zoom, &tools.brush);
                }
                ui.ctx().request_repaint();
            } else if let Some(note) = self.select.finish(doc, tools.select_feather) {
                *feedback = Some(note);
            }
        }

        // ---- drawing -------------------------------------------------------
        let painter = ui.painter_at(canvas);
        painter.rect_filled(canvas, 0, pal.canvas);
        // The map lies on the canvas, a soft shadow under it.
        if let Some((lo, hi)) = doc.dimension().bounds() {
            let t = TILE_SIZE as f64;
            let under = Rect::from_min_max(
                self.to_screen(canvas, lo.x as f64 * t, lo.z as f64 * t),
                self.to_screen(canvas, (hi.x + 1) as f64 * t, (hi.z + 1) as f64 * t),
            );
            let shadow = egui::epaint::Shadow {
                offset: [0, 8],
                blur: 36,
                spread: 2,
                color: Color32::from_black_alpha(if pal.dark { 170 } else { 70 }),
            };
            painter.add(shadow.as_shape(under, 3));
            // The shadow fills its rectangle: covered with the canvas, a
            // part of the map not drawn yet (or no tile) shows as nothing
            // rather than as a black hole.
            painter.rect_filled(under, 0, pal.canvas);
        }

        let style = MapStyle {
            mode: self.mode,
            contours: self.contours,
            smooth: self.smooth,
            solo: solo.then_some(tools.terrain),
            highlight: (doc.active_level > 0).then_some(doc.active_level.min(7) as u8),
            light,
            lava: doc.world.meta.lava,
            top: doc.dimension().max_y,
            seed: doc.world.meta.seed,
            annotations: tools.show_annotations,
            notes: doc.world.registry.annotations.fingerprint(),
            contour: overlays.contour(theme::high_contrast(ui.ctx())),
            sizes: tools.show_sizes || tools.tool == tools::Tool::TreeSize,
            pockets: tools.tool == tools::Tool::Pockets,
            slabs: tools.tool == tools::Tool::Slabs,
            palette: if self.mode == MapMode::TrueColour { crate::true_color::key(&doc.world.registry) } else { 0 },
            cut: self.cut,
        };
        let ppp = ui.ctx().pixels_per_point();
        let blocks_per_px = 1.0 / (self.zoom * ppp);
        let lod = if blocks_per_px <= 1.0 {
            0
        } else {
            (blocks_per_px.log2().floor() as u8).min(MAX_LOD)
        };

        let dim = doc.dimension();
        if let Some((lo, hi)) = dim.bounds() {
            let span = (CHUNK_PX as f64) * (1u64 << lod) as f64;
            let (wx0, wz0) = self.to_world(canvas, canvas.min);
            let (wx1, wz1) = self.to_world(canvas, canvas.max);
            let cx0 = ((wx0 / span).floor() as i64).max((lo.x >> lod) as i64);
            let cz0 = ((wz0 / span).floor() as i64).max((lo.z >> lod) as i64);
            let cx1 = ((wx1 / span).floor() as i64).min((hi.x >> lod) as i64);
            let cz1 = ((wz1 / span).floor() as i64).min((hi.z >> lod) as i64);
            let mut keys = Vec::new();
            for cz in cz0..=cz1 {
                for cx in cx0..=cx1 {
                    keys.push(ChunkKey {
                        lod,
                        x: cx as i32,
                        z: cz as i32,
                    });
                }
            }
            let mid = ((self.center[0] / span) - 0.5, (self.center[1] / span) - 0.5);
            keys.sort_by(|a, b| {
                let d = |k: &ChunkKey| (k.x as f64 - mid.0).powi(2) + (k.z as f64 - mid.1).powi(2);
                d(a).total_cmp(&d(b))
            });
            if keys.len() <= 4096 {
                // While the view moves, less of each frame goes to drawing
                // more of the map (the coarser images stand in): it keeps
                // up with the hand.
                if self.last_view != (self.center, self.zoom) || response.dragged() {
                    crate::map_render::mark_moving(ui.ctx());
                }
                self.last_view = (self.center, self.zoom);
                let budget = Duration::from_millis(if crate::map_render::moved_lately(ui.ctx()) { 5 } else { 12 });
                let more = cache.update(ui.ctx(), dim, &doc.world.registry, doc.world.meta.sea_level as f32, style, &keys, budget);
                if more {
                    ui.ctx().request_repaint();
                }
            }
            // Reference images under the relief, which fades over them.
            let refs = &doc.world.meta.references;
            references.draw(&painter, refs, false, &|x, z| self.to_screen(canvas, x, z));
            let tint = Color32::WHITE.gamma_multiply(crate::references::References::map_alpha(refs));
            for key in &keys {
                let (ox, oz) = key.origin();
                let b = key.blocks() as f64;
                let rect = Rect::from_min_max(
                    self.to_screen(canvas, ox as f64, oz as f64),
                    self.to_screen(canvas, ox as f64 + b, oz as f64 + b),
                );
                if let Some(tex) = cache.texture(style, *key) {
                    painter.image(tex.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), tint);
                    continue;
                }
                // Fall back on a coarser chunk while this one is built, as
                // coarse as need be, then on the four finer ones (zooming
                // out, they are there), then on nothing.
                let mut parent = *key;
                let mut shown = false;
                let mut up = 0u8;
                while parent.lod < MAX_LOD {
                    parent = parent.parent();
                    up += 1;
                    if let Some(tex) = cache.texture(style, parent) {
                        let n = (1i64 << up) as f32;
                        let u = (key.x as i64 - ((parent.x as i64) << up)) as f32 / n;
                        let v = (key.z as i64 - ((parent.z as i64) << up)) as f32 / n;
                        let uv = Rect::from_min_size(pos2(u, v), vec2(1.0 / n, 1.0 / n));
                        painter.image(tex.id(), rect, uv, tint);
                        shown = true;
                        break;
                    }
                }
                if !shown && key.lod > 0 {
                    for (dx, dz) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        let child = ChunkKey { lod: key.lod - 1, x: key.x * 2 + dx, z: key.z * 2 + dz };
                        if let Some(tex) = cache.texture(style, child) {
                            let half = rect.size() * 0.5;
                            let r = Rect::from_min_size(rect.min + vec2(dx as f32 * half.x, dz as f32 * half.y), half);
                            painter.image(tex.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), tint);
                        }
                    }
                }
            }
            // Over the relief.
            references.draw(&painter, refs, true, &|x, z| self.to_screen(canvas, x, z));
            references.draw_handles(ui, &painter, refs, &|x, z| self.to_screen(canvas, x, z));
            references.done_pill(ui, canvas);

            if let Some((tex, r)) = &overlay {
                let rect = Rect::from_min_max(self.to_screen(canvas, r[0] as f64, r[1] as f64), self.to_screen(canvas, r[2] as f64, r[3] as f64));
                painter.image(tex.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            }
            // The terrain generator's Live preview, over the map as it is.
            for (tex, r, uv) in &self.live {
                let rect = Rect::from_min_max(self.to_screen(canvas, r[0] as f64, r[1] as f64), self.to_screen(canvas, r[2] as f64, r[3] as f64));
                if rect.intersects(canvas) {
                    painter.image(*tex, rect, *uv, Color32::WHITE);
                }
            }

            // Map outline.
            let t = TILE_SIZE as f64;
            let outline = Rect::from_min_max(
                self.to_screen(canvas, lo.x as f64 * t, lo.z as f64 * t),
                self.to_screen(canvas, (hi.x + 1) as f64 * t, (hi.z + 1) as f64 * t),
            );
            painter.rect_stroke(outline, 0, Line::new(1.0, theme::scale(pal.border, 0.85)), StrokeKind::Outside);
        }

        // The regions picked for a Test Export, over the region files' grid.
        if let Some(pick) = &tools.region_pick {
            let size = REGION_SIZE as f64;
            for &(rx, rz) in &pick.regions {
                let r = Rect::from_min_max(self.to_screen(canvas, rx as f64 * size, rz as f64 * size), self.to_screen(canvas, (rx + 1) as f64 * size, (rz + 1) as f64 * size));
                if r.intersects(canvas) {
                    painter.rect_filled(r.intersect(canvas), 0, pal.accent.gamma_multiply(0.28));
                    painter.rect_stroke(r, 0, Line::new(2.0, pal.accent), StrokeKind::Inside);
                }
            }
        }
        if self.grid || tools.region_pick.is_some() {
            self.draw_grid(&painter, canvas, pal);
        }

        // The selection, its ants marching round it.
        {
            let (x0, z0) = self.to_world(canvas, canvas.min);
            let (x1, z1) = self.to_world(canvas, canvas.max);
            let view = massif_core::BlockRect::new(x0.floor() as i32, z0.floor() as i32, x1.ceil() as i32, z1.ceil() as i32);
            // Its ants on the land only.
            let land: Vec<egui::Painter> = self.land_rects(canvas, doc, canvas).into_iter().map(|r| painter.with_clip_rect(r)).collect();
            let mut select = std::mem::take(&mut self.select);
            select.draw(ui, &painter, &land, canvas, &|x, z| self.to_screen(canvas, x, z), view, self.zoom, doc.dimension().selection.as_ref());
            self.select = select;
        }

        // The world's origin (x 0, z 0), where the game counts blocks
        // from, with the grid only (alone, it was taken for the pointer): a
        // small faint plus, light in a dark outline to read on any land.
        let o = self.to_screen(canvas, 0.0, 0.0);
        if self.grid && canvas.contains(o) {
            let halo = Color32::from_black_alpha(90);
            for (a, b) in [(vec2(0.0, 4.0), vec2(0.0, -4.0)), (vec2(4.0, 0.0), vec2(-4.0, 0.0))] {
                painter.line_segment([o + a, o + b], Line::new(2.5, halo));
            }
            for (a, b) in [(vec2(0.0, 3.5), vec2(0.0, -3.5)), (vec2(3.5, 0.0), vec2(-3.5, 0.0))] {
                painter.line_segment([o + a, o + b], Line::new(1.0, Color32::WHITE.gamma_multiply(0.7)));
            }
            if pointer.is_some_and(|p| response.hovered() && p.distance(o) < 8.0) && !self.view3d_grab {
                response.clone().on_hover_text(tr!("The world's origin: x 0, z 0"));
            }
        }

        // Where the 3D View is and the way it looks.
        if let Some(v) = &view3d {
            self.draw_view3d_mark(&painter, canvas, v, pal, on_mark || self.view3d_grab);
            if on_mark && !self.view3d_grab {
                response.clone().on_hover_text(tr!("The 3D View: drag to move it"));
            }
        }

        // A spot pointed out: rings spreading from it for a few seconds.
        if let Some((at, since)) = self.ping {
            let now = ui.input(|i| i.time);
            let since = since.unwrap_or(now);
            self.ping = Some((at, Some(since)));
            let t = (now - since) as f32;
            if t > 3.2 {
                self.ping = None;
            } else {
                let p = self.to_screen(canvas, at[0], at[1]);
                let still = crate::theme::reduce_motion(ui.ctx());
                let fade = if still { 1.0 } else { (1.0 - (t - 2.4).max(0.0) / 0.8).clamp(0.0, 1.0) };
                // Rings spreading out; one ring, still, with less motion.
                for k in 0..if still { 1 } else { 2 } {
                    let phase = if still { 0.45 } else { (t / 1.1 + k as f32 * 0.5).fract() };
                    let r = 7.0 + phase * 34.0;
                    let alpha = if still { 1.0 } else { (1.0 - phase) * fade };
                    painter.circle_stroke(p, r, Line::new(3.5, Color32::from_black_alpha((70.0 * alpha) as u8)));
                    painter.circle_stroke(p, r, Line::new(2.0, pal.accent.gamma_multiply(alpha)));
                }
                painter.circle(p, 5.5, pal.accent.gamma_multiply(fade), Line::new(2.0, Color32::WHITE.gamma_multiply(fade)));
                ui.ctx().request_repaint();
            }
        }

        // The river being laid out, with its handles; before a source is
        // placed, where water let go under the pointer would go.
        if tools.tool == Tool::River {
            let spec = tools.river.spring_spec(&doc.world.registry);
            if let Some(draft) = &tools.river_draft {
                self.spring = None;
                let hover = match (self.river_grab, pointer.filter(|_| response.hovered())) {
                    (None, Some(p)) => self.river_hit(canvas, draft, &tools.river, p),
                    _ => None,
                };
                self.draw_river(&painter, canvas, &draft.plan, &spec, Some((draft, hover, self.river_grab)));
                if response.hovered() {
                    ui.ctx().set_cursor_icon(match (self.river_grab, hover) {
                        (Some(_), _) => CursorIcon::Grabbing,
                        (None, Some(RiverHit::Anchor(_) | RiverHit::Mouth)) => CursorIcon::Grab,
                        (None, Some(RiverHit::Course(_))) => CursorIcon::PointingHand,
                        (None, None) => CursorIcon::Crosshair,
                    });
                }
            } else {
                // Worked out once the pointer rests, the way the river goes.
                if let Some((x, z)) = world_pointer.filter(|_| response.hovered()) {
                    let key = (x.floor() as i32, z.floor() as i32, doc.revision ^ (tools.river.lakes.to_bits() as u64) << 32);
                    let now = ui.input(|i| i.time);
                    match self.spring_rest {
                        Some((k, since)) if k == key => {
                            if now - since > 0.12 && self.spring.as_ref().is_none_or(|s| s.0 != key) {
                                let sea = doc.world.meta.sea_level as f32;
                                let d = massif_core::river::descend(doc.dimension().ground(), (key.0, key.1), sea, 30_000, 600_000, tools.river.lake_depth());
                                self.spring = Some((key, d));
                            }
                        }
                        _ => self.spring_rest = Some((key, now)),
                    }
                    ui.ctx().request_repaint_after(std::time::Duration::from_millis(130));
                }
                if let Some((_, d)) = &self.spring {
                    self.draw_river(&painter, canvas, d, &spec, None);
                }
                let hint = match self.spring.as_ref().map(|s| &s.1) {
                    Some(d) if d.course.len() >= 3 => tr!("Click: a river springs here and runs to {map_or}", map_or = d.mouth.map_or("", |m| crate::i18n::tr_name(m.label()))),
                    _ => tr!("Click on land: where the river springs").to_owned(),
                };
                let at = canvas.center_bottom() - vec2(0.0, 22.0);
                let galley = painter.layout_no_wrap(hint, FontId::proportional(12.5), Color32::from_gray(235));
                let back = Rect::from_center_size(at, galley.size() + vec2(24.0, 12.0));
                painter.rect_filled(back.translate(vec2(0.0, 2.0)), 9, Color32::from_black_alpha(60));
                painter.rect(back, 9, Color32::from_rgba_unmultiplied(20, 22, 27, 235), Line::new(1.0, Color32::from_white_alpha(18)), StrokeKind::Inside);
                painter.galley(back.min + vec2(12.0, 6.0), galley, Color32::from_gray(235));
            }
        } else {
            self.spring = None;
        }
        if tools.tool == Tool::Annotate && tools.annotation_curve {
            let screen = |q: [f32; 2]| self.to_screen(canvas, q[0] as f64, q[1] as f64);
            let hover = pointer.filter(|_| response.hovered());
            crate::curve_draft::draw(ui, &painter, canvas, response.id, &screen, self.zoom, hover, doc, tools, feedback);
        }
        if tools.tool == Tool::Road {
            let screen = |q: (f32, f32)| self.to_screen(canvas, q.0 as f64, q.1 as f64);
            let hover = pointer.filter(|_| response.hovered());
            crate::road_draft::draw(ui, &painter, canvas, response.id, &screen, self.zoom, hover, doc, tools, feedback);
        }
        if tools.tool == Tool::River && tools.river_draft.is_some() {
            self.river_bar(ui, &painter, canvas, response.id, doc, tools, feedback);
        } else {
            self.river_bar = None;
        }
        if matches!(tools.tool, Tool::Structure | Tool::Village) {
            let screen = |q: (f32, f32)| self.to_screen(canvas, q.0 as f64, q.1 as f64);
            let hover = pointer.filter(|_| response.hovered()).map(|p| self.to_world(canvas, p)).map(|(x, z)| (x.floor() as i32, z.floor() as i32));
            crate::structure_tool::draw(ui.ctx(), &painter, canvas, &screen, self.zoom, hover, doc, tools);
            if hover.is_some() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
            }
        }

        // Brush outline.
        if let Some(p) = pointer.filter(|p| canvas.contains(*p))
            && (response.hovered() || self.stroke.is_some())
        {
            if tools.region_pick.is_some() && !panning {
                ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
            } else if tools.filter_pick.is_some() && !panning {
                ui.ctx().set_cursor_icon(CursorIcon::Crosshair);
            } else if panning {
                ui.ctx().set_cursor_icon(if down[0] { CursorIcon::Grabbing } else { CursorIcon::Grab });
            } else if tools.tool.uses_brush() && !(tools.tool == Tool::Annotate && tools.annotation_curve) {
                let shown = tools.shown_brush().into_owned();
                let brush = &shown;
                let shadow = Line::new(3.0, Color32::from_black_alpha(90));
                let line = Line::new(1.4, Color32::WHITE);
                // Its fill, hatching and noise over the land only: over the
                // void round the map, its outline alone, faint.
                let faint = Line::new(1.0, Color32::from_white_alpha(80));
                let land: Vec<egui::Painter> = self.land_rects(canvas, doc, Rect::from_center_size(p, Vec2::splat(2.0 * brush.reach() * self.zoom + 8.0))).into_iter().map(|r| painter.with_clip_rect(r)).collect();
                // The automatic tools show what they would change, a noise
                // what it would paint.
                let previewing = !modifiers.alt && self.stroke.is_none();
                let noise = tools.tool == Tool::PaintTerrain && tools.pattern.on;
                if previewing && let Some((x, z)) = world_pointer {
                    if matches!(tools.tool, Tool::AutoPaint | Tool::AutoBiome) {
                        self.show_auto_preview(ui.ctx(), &painter, &land, canvas, doc, tools, (x, z));
                    } else if noise {
                        self.show_pattern_preview(ui.ctx(), &land, canvas, doc, tools, (x, z));
                    }
                }
                // How hard the brush presses: lighter where it presses
                // more, with a line at a quarter, a half and three quarters
                // of its strength.
                let key = BrushPreview::key(brush);
                if self.strength.as_ref().is_none_or(|s| s.key != key) {
                    self.strength = Some(BrushPreview::new(ui.ctx(), brush));
                }
                let automatic = matches!(tools.tool, Tool::AutoPaint | Tool::AutoBiome);
                // Over the colours of a noise, its lines alone; switched off
                // (Brush Settings), the lines alone too.
                let fill = tools.show_strength && !automatic && !(noise && previewing);
                // Close enough to see the blocks, the strength is drawn
                // column by column, as the outline is: they keep together
                // as the pointer moves between blocks.
                let by_block = match world_pointer {
                    Some((x, z)) if fill && self.zoom * ppp >= EXACT_ZOOM && brush.reach() <= EXACT_REACH => self.block_strength(ui.ctx(), brush, x, z),
                    _ => None,
                };
                if let Some(preview) = self.strength.as_ref().filter(|_| !automatic) {
                    let side = 2.0 * brush.reach() * self.zoom;
                    if side >= 12.0 {
                        let square = Rect::from_center_size(p, Vec2::splat(side));
                        let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
                        match by_block {
                            Some((texture, [x0, z0, x1, z1])) => {
                                let r = Rect::from_min_max(self.to_screen(canvas, x0 as f64, z0 as f64), self.to_screen(canvas, x1 as f64, z1 as f64));
                                for lp in &land {
                                    lp.image(texture, r, uv, Color32::WHITE);
                                }
                            }
                            None if fill => {
                                for lp in &land {
                                    lp.image(preview.texture.id(), square, uv, Color32::WHITE);
                                }
                            }
                            None => {}
                        }
                        for (k, rings) in preview.levels.iter().enumerate() {
                            let ink = Line::new(1.1, Color32::from_white_alpha(105 + 45 * k as u8));
                            let under = Line::new(2.4, Color32::from_black_alpha(55));
                            for ring in rings {
                                if let Some(points) = crate::brushes::drawable(ring.iter().map(|&(u, v)| square.min + vec2(u, v) * side), 1.5, true) {
                                    for lp in &land {
                                        lp.add(egui::Shape::closed_line(points.clone(), under));
                                        lp.add(egui::Shape::closed_line(points.clone(), ink));
                                    }
                                }
                            }
                        }
                    }
                }
                let mut rings: Vec<Vec<Pos2>> = Vec::new();
                if let Some((x, z)) = world_pointer
                    && self.zoom * ppp >= EXACT_ZOOM
                    && brush.reach() <= EXACT_REACH
                {
                    // Close enough to see the blocks: exactly the columns the
                    // brush changes.
                    for ring in brush.footprint_outline(x as f32, z as f32) {
                        rings.push(ring.iter().map(|&(bx, bz)| self.to_screen(canvas, bx as f64, bz as f64)).collect());
                    }
                } else if let Some(mask) = &brush.mask {
                    // A custom brush: where its image paints, pixel by pixel
                    // (by squares of pixels where a pixel is smaller than one
                    // of the screen).
                    let traced = crate::brushes::outline_for(mask, 2.0 * brush.radius.max(0.5) * self.zoom);
                    for ring in brush.place_outline(&traced) {
                        rings.extend(crate::brushes::drawable(ring.iter().map(|&(dx, dz)| p + vec2(dx * self.zoom, dz * self.zoom)), 0.75, true));
                    }
                } else {
                    match brush.shape {
                        BrushShape::Circle => {
                            let r = brush.radius * self.zoom;
                            painter.circle_stroke(p, r, faint);
                            for lp in &land {
                                lp.circle_stroke(p, r, shadow);
                                lp.circle_stroke(p, r, line);
                            }
                        }
                        BrushShape::Square => rings.push(
                            brush
                                .square_corners(0.0, 0.0)
                                .iter()
                                .map(|(x, z)| p + vec2(x * self.zoom, z * self.zoom))
                                .collect(),
                        ),
                    }
                }
                rings.retain(|r| r.len() > 2);
                for ring in &rings {
                    painter.add(egui::Shape::closed_line(ring.clone(), faint));
                }
                for lp in &land {
                    for ring in &rings {
                        lp.add(egui::Shape::closed_line(ring.clone(), shadow));
                    }
                    for ring in &rings {
                        lp.add(egui::Shape::closed_line(ring.clone(), line));
                    }
                }
                pointer_mark(ui.ctx(), &painter, p);
            } else if (tools.tool != Tool::River || tools.river_draft.is_none()) && (tools.tool != Tool::Road || tools.road_draft.is_none()) && !(tools.tool == Tool::Annotate && tools.annotation_curve) {
                pointer_mark(ui.ctx(), &painter, p);
            }
        }

        // The water tools say what they would do under the cursor: the
        // level they fill to, or why it stays dry.
        if matches!(tools.tool, Tool::Water | Tool::Flood)
            && !panning
            && let Some(p) = pointer.filter(|p| canvas.contains(*p))
            && (response.hovered() || self.stroke.is_some() || self.water.is_some())
            && let Some((x, z)) = world_pointer
        {
            let (bx, bz) = (x.floor() as i32, z.floor() as i32);
            let stroke = self.stroke.as_ref().and_then(|s| s.water_to());
            let held = self.water.as_ref().map_or(0, |(h, _)| h.blocks());
            let key: WaterTagKey = (
                (bx, bz),
                (doc.revision, doc.dim, held),
                tools.tool,
                (tools.flood_height, tools.brush_height, tools.water_level, doc.world.meta.sea_level),
                (tools.water_limit_on, tools.water_limit, tools.flood_warn),
                stroke,
            );
            if self.water_tag.as_ref().is_none_or(|(k, _)| *k != key) {
                self.water_tag = Some((key, tools::water_tag(doc, tools, bx, bz, stroke)));
            }
            if let Some((_, Some((text, target)))) = &self.water_tag {
                // Where the water poured here would run down to: a ring on
                // the bottom of the hollow it fills, a dotted line to it.
                if let Some((tx, tz)) = *target {
                    let t = self.to_screen(canvas, tx as f64 + 0.5, tz as f64 + 0.5);
                    if t.distance(p) > 10.0 {
                        let ink = Color32::from_rgb(56, 146, 255);
                        let dots = egui::Shape::dotted_line(&[p, t], Color32::from_white_alpha(200), 6.0, 1.3);
                        painter.extend(dots);
                        painter.circle_stroke(t, 7.0, Line::new(3.0, Color32::from_black_alpha(90)));
                        painter.circle(t, 6.0, ink.gamma_multiply(0.85), Line::new(2.0, Color32::WHITE));
                    }
                }
                // Folded past a width, so that a long reason stays a note.
                let (pad_x, pad_y) = (theme::FLOAT_PAD_X - 2.0, theme::FLOAT_PAD_Y - 3.0);
                let galley = painter.layout(text.clone(), FontId::proportional(12.0), Color32::from_gray(235), (theme::FLOAT_MAX_W - 2.0 * pad_x).min(canvas.width() - 2.0 * theme::PAD - 2.0 * pad_x).max(80.0));
                let size = galley.size() + vec2(2.0 * pad_x, 2.0 * pad_y);
                let mut at = p + vec2(18.0, 18.0);
                // Kept on the canvas, PAD from its edges: to the left or above
                // the cursor near them.
                let edge = canvas.shrink(theme::PAD);
                if at.x + size.x > edge.right() {
                    at.x = p.x - 18.0 - size.x;
                }
                if at.y + size.y > edge.bottom() {
                    at.y = p.y - 18.0 - size.y;
                }
                at = at.max(edge.min);
                // Clear of the ring where the water would go, on another
                // side of the cursor.
                if let Some((tx, tz)) = *target {
                    let ring = Rect::from_center_size(self.to_screen(canvas, tx as f64 + 0.5, tz as f64 + 0.5), vec2(20.0, 20.0));
                    let sides = [at, pos2(p.x - 18.0 - size.x, at.y), pos2(at.x, p.y - 18.0 - size.y), pos2(p.x - 18.0 - size.x, p.y - 18.0 - size.y)];
                    if let Some(side) = sides.into_iter().find(|s| !Rect::from_min_size(*s, size).intersects(ring) && edge.contains_rect(Rect::from_min_size(*s, size))) {
                        at = side;
                    }
                }
                let back = Rect::from_min_size(at, size);
                let radius = theme::TILE_RADIUS;
                painter.rect_filled(back.translate(vec2(0.0, 1.5)), radius, Color32::from_black_alpha(55));
                painter.rect(back, radius, Color32::from_rgba_unmultiplied(20, 22, 27, 235), Line::new(1.0, Color32::from_white_alpha(18)), StrokeKind::Inside);
                painter.galley(back.min + vec2(pad_x, pad_y), galley, Color32::from_gray(235));
            }
        }

        if doc.dimension().bounds().is_none() {
            painter.text(
                canvas.center(),
                Align2::CENTER_CENTER,
                tr!("This world has no map yet: create one with File ▸ New World"),
                FontId::proportional(14.0),
                pal.weak,
            );
        }

        self.draw_scale(&painter, canvas, pal);
        if self.mode == MapMode::TrueColour {
            Self::true_colour_note(&painter, canvas, pal, doc);
        }
        if let Some(note) = &self.live_note {
            let font = egui::FontId::proportional(11.5);
            let galley = painter.layout_no_wrap(note.clone(), font, pal.text);
            let at = canvas.left_top() + egui::vec2(theme::PAD, theme::PAD);
            let pill = Rect::from_min_size(at, galley.size() + egui::vec2(16.0, 8.0));
            painter.rect_filled(pill, theme::PILL, theme::scale(pal.panel, 0.96));
            painter.rect_stroke(pill, theme::PILL, Line::new(1.0, pal.accent), StrokeKind::Inside);
            painter.galley(at + egui::vec2(8.0, 4.0), galley, pal.text);
        }
        self.footer(ui, footer, doc, pal, *hover);
    }

    fn draw_grid(&self, painter: &egui::Painter, canvas: Rect, pal: Palette) {
        let (wx0, wz0) = self.to_world(canvas, canvas.min);
        let (wx1, wz1) = self.to_world(canvas, canvas.max);
        let lines = |step: f64, color: Color32| {
            if step * self.zoom as f64 * 1.0 < 12.0 {
                return;
            }
            let mut x = (wx0 / step).floor() * step;
            while x <= wx1 {
                let a = self.to_screen(canvas, x, wz0);
                painter.vline(a.x, canvas.y_range(), Line::new(1.0, color));
                x += step;
            }
            let mut z = (wz0 / step).floor() * step;
            while z <= wz1 {
                let a = self.to_screen(canvas, wx0, z);
                painter.hline(canvas.x_range(), a.y, Line::new(1.0, color));
                z += step;
            }
        };
        let base = if pal.dark { Color32::WHITE } else { Color32::BLACK };
        lines(16.0, base.gamma_multiply(0.08));
        lines(TILE_SIZE as f64, base.gamma_multiply(0.22));
        // The region files' borders (`r.x.z.mca`, 512 blocks a side), over
        // the others, each named in its corner while there is room.
        let region = REGION_SIZE as f64;
        if region * (self.zoom as f64) < 6.0 {
            return;
        }
        let ink = theme::mix(base, pal.accent, 0.35).gamma_multiply(0.75);
        let mut x = (wx0 / region).floor() * region;
        while x <= wx1 {
            let a = self.to_screen(canvas, x, wz0);
            painter.vline(a.x, canvas.y_range(), Line::new(1.5, ink));
            x += region;
        }
        let mut z = (wz0 / region).floor() * region;
        while z <= wz1 {
            let a = self.to_screen(canvas, wx0, z);
            painter.hline(canvas.x_range(), a.y, Line::new(1.5, ink));
            z += region;
        }
        if region * (self.zoom as f64) < 70.0 {
            return;
        }
        let font = FontId::proportional(10.5);
        for rz in (wz0 / region).floor() as i64..=(wz1 / region).floor() as i64 {
            for rx in (wx0 / region).floor() as i64..=(wx1 / region).floor() as i64 {
                // In the corner of the region, or of the canvas where the
                // corner is off it.
                let corner = self.to_screen(canvas, rx as f64 * region, rz as f64 * region);
                let end = self.to_screen(canvas, (rx + 1) as f64 * region, (rz + 1) as f64 * region);
                let at = pos2(corner.x.max(canvas.left()) + 5.0, corner.y.max(canvas.top()) + 4.0);
                let galley = painter.layout_no_wrap(region_name(rx, rz), font.clone(), pal.text);
                let tag = Rect::from_min_size(at, galley.size() + vec2(8.0, 2.0));
                if tag.right() > end.x - 2.0 || tag.bottom() > end.y - 2.0 {
                    continue;
                }
                painter.rect_filled(tag, theme::TILE_RADIUS / 2, pal.panel.gamma_multiply(0.82));
                painter.galley(tag.min + vec2(4.0, 1.0), galley, pal.text);
            }
        }
    }

    /// Over the scale, in True Colour: whose colours the map shows, the
    /// game's or the resource packs' read with the blocks.
    fn true_colour_note(painter: &egui::Painter, canvas: Rect, pal: Palette, doc: &Document) {
        let table = crate::true_color::current(&doc.world.registry);
        let text = if !crate::true_color::has_catalogue() {
            tr!("The game's blocks are not read yet: colours are Massif's guesses").to_owned()
        } else if table.packs.is_empty() {
            tr!("The game's own colours").to_owned()
        } else {
            tr!("Colours of {packs}", packs = table.packs.join(", "))
        };
        let font = FontId::proportional(11.0);
        let galley = painter.layout(text, font, pal.text, (canvas.width() - 24.0).max(60.0));
        let at = pos2(canvas.left() + 8.0, canvas.bottom() - 46.0 - galley.size().y);
        let bg = Rect::from_min_size(at, galley.size() + vec2(12.0, 6.0));
        painter.rect_filled(bg, theme::TILE_RADIUS, pal.panel.gamma_multiply(0.86));
        painter.galley(at + vec2(6.0, 3.0), galley, pal.text);
    }

    fn draw_scale(&self, painter: &egui::Painter, canvas: Rect, pal: Palette) {
        // A round length close to 110 points.
        let target = 110.0 / self.zoom;
        let pow = 10f32.powf(target.log10().floor());
        let len = [1.0, 2.0, 5.0, 10.0]
            .into_iter()
            .map(|m| m * pow)
            .rfind(|l| *l <= target)
            .unwrap_or(pow);
        let px = len * self.zoom;
        let base = pos2(canvas.left() + 12.0, canvas.bottom() - 12.0);
        let bg = Rect::from_min_max(base - vec2(4.0, 30.0), base + vec2(px + 40.0, 6.0));
        painter.rect_filled(bg, 6, pal.panel.gamma_multiply(0.86));
        // North arrow.
        painter.text(base + vec2(6.0, -17.0), Align2::CENTER_CENTER, "N", FontId::proportional(10.0), pal.text);
        let tip = base + vec2(6.0, -9.0);
        painter.add(egui::Shape::convex_polygon(
            vec![tip, tip + vec2(4.0, 8.0), tip + vec2(-4.0, 8.0)],
            pal.accent,
            Line::NONE,
        ));
        let x0 = base.x + 22.0;
        let y = base.y - 2.0;
        let line = Line::new(1.5, pal.text);
        painter.hline(x0..=x0 + px, y, line);
        painter.vline(x0, (y - 5.0)..=y, line);
        painter.vline(x0 + px, (y - 5.0)..=y, line);
        let label = if len >= 1000.0 {
            tr!("{value} km", value = len / 1000.0)
        } else {
            tr!("{len} blocks", len)
        };
        painter.text(pos2(x0 + 4.0, y - 4.0), Align2::LEFT_BOTTOM, label, FontId::proportional(10.5), pal.text);
    }

    fn footer(&mut self, ui: &mut Ui, rect: Rect, doc: &Document, pal: Palette, hover: Option<(i32, i32)>) {
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0, pal.header);
        painter.hline(rect.x_range(), rect.top(), Line::new(1.0, pal.border));
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(rect.shrink2(vec2(8.0, 2.0)))
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        let location = match hover {
            Some((x, z)) => {
                let key = (x, z, doc.revision ^ doc.generation.rotate_left(32));
                let probe = match self.probe {
                    Some((k, p)) if k == key => p,
                    _ => {
                        let p = tools::probe(doc, x, z);
                        self.probe = Some((key, p));
                        p
                    }
                };
                match probe {
                    Some(p) => {
                        let reg = &doc.world.registry;
                        let terrain = &reg.terrain(TerrainId(p.terrain)).name;
                        let water = if p.fluid != massif_core::NO_FLUID {
                            tr!(" · water {fluid}", fluid = p.fluid)
                        } else {
                            String::new()
                        };
                        let cover = reg.cover(p.cover).map(|c| format!(" · {}", c.name)).unwrap_or_default();
                        // The biome the map's Biome view shows: painted, or
                        // the one export picks.
                        let sample = crate::map_render::Sample {
                            height: p.height,
                            terrain: p.terrain,
                            fluid: p.fluid,
                            biome: p.biome,
                            cover: p.cover,
                            objects: p.objects,
                            annotation: 0,
                            understory: 0,
                            size: 0,
                            pockets: 0,
                            flags: 0,
                            levels: 0,
                        };
                        let biome = crate::map_render::AutoBiome::new(reg).resolve(reg, &sample, doc.world.meta.sea_level as f32);
                        let biome = format!(" · {}{}", reg.biome(massif_core::registry::BiomeId(biome)).name, if p.biome == 0 { tr!(" (automatic)") } else { "" });
                        // In whole blocks, as the game has the land.
                        tr!("Location: {x}, {z} · height {block_top} · {terrain}{cover}{water}{biome}", block_top = massif_core::compose::block_top(p.height), x, z, terrain, cover, water, biome)
                    }
                    None => tr!("Location: {x}, {z}", x, z),
                }
            }
            None => tr!("Location: –").to_owned(),
        };
        // With the grid, the region file the column is written to.
        let location = match hover {
            Some((x, z)) if self.grid => format!("{location} · {}", region_name(x.div_euclid(REGION_SIZE) as i64, z.div_euclid(REGION_SIZE) as i64)),
            _ => location,
        };
        // The zoom's controls first, from the right; the location in the
        // room they leave, cut with an ellipsis (whole on hover).
        child.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.small_button(theme::glyph(Icon::Maximize2).to_string()).on_hover_text(tr!("Fit the whole map")).clicked() {
                self.fit_smoothly(doc);
            }
            if ui.small_button("+").clicked() {
                self.zoom_smoothly(2.0);
            }
            ui.label(egui::RichText::new(format!("{:.0}%", self.zoom * 100.0)).size(11.5));
            if ui.small_button("−").clicked() {
                self.zoom_smoothly(0.5);
            }
            let mut log = self.zoom.log2();
            ui.spacing_mut().interact_size.y = 16.0;
            let slider = egui::Slider::new(&mut log, MIN_ZOOM.log2()..=MAX_ZOOM.log2()).show_value(false);
            if ui.add_sized([90.0, 16.0], slider).changed() {
                self.goal = None;
                let canvas = self.canvas;
                self.zoom_at(canvas, canvas.center(), log.exp2() / self.zoom);
            }
            ui.add_space(8.0);
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.add(egui::Label::new(egui::RichText::new(location).size(11.5).color(pal.weak)).truncate());
            });
        });
    }
}

/// Whether the wheel turns the brush: with ⌘ on a Mac (the Windows key of a
/// PC keyboard), with Ctrl + Shift elsewhere, where the Windows key does not
/// reach applications.
/// The pointer over the map: the crosshair of Settings ▸ Appearance in
/// place of the system's (a thin black cross, lost on dark land), white in
/// a dark outline so that it reads on snow as on deep water. With None
/// chosen, the system's crosshair and a white dot in a dark ring.
fn pointer_mark(ctx: &egui::Context, painter: &egui::Painter, p: Pos2) {
    let reticle = crate::reticle::get(ctx);
    if reticle.shape == crate::reticle::Shape::None || reticle.opacity <= 0.0 {
        painter.circle_filled(p, 2.6, Color32::from_black_alpha(140));
        painter.circle_filled(p, 1.5, Color32::WHITE);
        ctx.set_cursor_icon(CursorIcon::Crosshair);
    } else {
        crate::reticle::paint(painter, p, &reticle);
        ctx.set_cursor_icon(CursorIcon::None);
    }
}

/// Blocks along each side of a region file of the game.
pub const REGION_SIZE: i32 = 512;

/// The region file a region is written to, as the game names it.
pub fn region_name(rx: i64, rz: i64) -> String {
    format!("r.{rx}.{rz}.mca")
}

fn turns_brush(m: egui::Modifiers) -> bool {
    // Alt, as in WorldPainter; ⌘ (Ctrl+Shift elsewhere) as before.
    m.alt || m.mac_cmd || (!cfg!(target_os = "macos") && m.ctrl && m.shift)
}

#[cfg(test)]
mod fit_tests {
    use super::*;

    /// A map is framed once the canvas has room to show it: not on a canvas
    /// a few points high, where it came out at 1 % and stayed; again when
    /// the canvas grows well past the size it was framed on while the view
    /// is still as the framing left it, but not once the user moved it.
    #[test]
    fn the_map_is_framed_once_there_is_room() {
        let mut view = MapView::default();
        assert!(!view.should_fit(1, vec2(900.0, 12.0)), "no room yet");
        assert!(view.should_fit(1, vec2(900.0, 600.0)));
        view.fitted = Some(1);
        view.fit_view = Some((view.center, view.zoom, vec2(300.0, 80.0)));
        assert!(view.should_fit(1, vec2(900.0, 600.0)), "framed small, the canvas grown: again");
        assert!(!view.should_fit(1, vec2(320.0, 90.0)), "much the same size: as it is");
        view.center[0] += 40.0;
        assert!(!view.should_fit(1, vec2(900.0, 600.0)), "moved by the user: left alone");
        assert!(view.should_fit(2, vec2(900.0, 600.0)), "another map: framed");
    }
}
