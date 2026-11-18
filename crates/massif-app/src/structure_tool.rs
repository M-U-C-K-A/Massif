//! The Structure tool: the game's own structures — a village, an Ancient
//! City, Trial Chambers, a shipwreck — one click each, read from the
//! player's game and assembled the way the game assembles them
//! ([`massif_structures`]). Its settings in Tool Settings; on the map, the
//! outline of what a click would place and of what is placed; the job that
//! builds one off the interface's thread.

use crate::doc::Document;
use crate::theme::{self, Icon, Palette};
use crate::tools::{Feedback, ToolState};
use egui::{Color32, FontId, Pos2, Rect, Stroke, StrokeKind, Ui, pos2, vec2};
use massif_structures::Library;
use massif_structures::geom::BBox;
use massif_structures::vanilla::{self, Group, Height, KINDS, Kind};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

/// The names of the structures offered (see [`vanilla::KINDS`]), as the
/// language files know them.
#[allow(dead_code)]
const NAMES: [&str; 18] = [
    tn!("Plains village"),
    tn!("Desert village"),
    tn!("Savanna village"),
    tn!("Taiga village"),
    tn!("Snowy village"),
    tn!("Pillager outpost"),
    tn!("Desert temple"),
    tn!("Jungle temple"),
    tn!("Witch hut"),
    tn!("Igloo"),
    tn!("Shipwreck"),
    tn!("Ocean ruins"),
    tn!("Warm ocean ruins"),
    tn!("Ancient City"),
    tn!("Trial Chambers"),
    tn!("Stronghold"),
    tn!("Trail ruins"),
    tn!("Bastion remnant"),
];

/// The settings of the Structure tool.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StructureSettings {
    /// The structure placed, by its key (`ancient_city`).
    pub kind: String,
    /// The draw it is assembled from: the same draw at the same place
    /// assembles it the same.
    pub seed: u64,
    /// Quarter turns clockwise; `None`: drawn with the rest.
    pub turn: Option<u8>,
    /// The height of each structure placed at a depth, by key, where it is
    /// not the game's own.
    pub depths: Vec<(String, i32)>,
    /// Flatten the land under its buildings.
    pub flatten: bool,
    #[serde(skip)]
    pub rt: Runtime,
}

impl Default for StructureSettings {
    fn default() -> Self {
        Self { kind: "village_plains".into(), seed: 1, turn: None, depths: Vec::new(), flatten: true, rt: Runtime::default() }
    }
}

impl StructureSettings {
    pub fn kind(&self) -> &'static Kind {
        vanilla::kind(&self.kind).unwrap_or(&KINDS[0])
    }

    /// The height a structure placed at a depth goes at: the one set, or
    /// the game's.
    pub fn depth(&self, kind: &Kind) -> Option<i32> {
        match kind.height {
            Height::Depth(d) => Some(self.depths.iter().find(|(k, _)| k == kind.id).map_or(d, |e| e.1)),
            _ => None,
        }
    }

    fn set_depth(&mut self, kind: &Kind, y: i32) {
        self.depths.retain(|(k, _)| k != kind.id);
        if kind.height != Height::Depth(y) {
            self.depths.push((kind.id.to_owned(), y));
        }
    }

    /// What a click at column `(x, z)` asks for, numbered `id`.
    pub fn request(&self, x: i32, z: i32, id: u32) -> vanilla::Request {
        let kind = self.kind();
        let y = self.depth(kind).unwrap_or(0);
        vanilla::Request { kind, at: [x, y, z], turn: self.turn, seed: self.seed, flatten: self.flatten && kind.flattens, id }
    }
}

/// What the tool keeps while it works: the game's files read, the click
/// waiting to be built, the outline shown before a click.
#[derive(Clone, Default)]
pub struct Runtime(Arc<Mutex<Inner>>);

impl PartialEq for Runtime {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl std::fmt::Debug for Runtime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Runtime")
    }
}

/// The outline of a structure before it is placed: by which draw, the
/// boxes of its pieces around the column clicked.
type LayoutKey = (String, u64, Option<u8>, Option<i32>);

#[derive(Default)]
struct Inner {
    library: Option<Arc<Library>>,
    loading: bool,
    error: Option<String>,
    /// A click to build at, and the structure it takes the place of, if
    /// any (Draw again).
    pending: Option<(i32, i32, Option<u32>)>,
    /// Being built.
    busy: bool,
    layout: Option<(LayoutKey, Arc<Vec<BBox>>)>,
    layout_busy: Option<LayoutKey>,
    /// The last structure the tool placed: where it was clicked and its
    /// number, for Draw again.
    last: Option<(i32, i32, u32)>,
    note: Option<String>,
}

impl Runtime {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn library(&self) -> Option<Arc<Library>> {
        self.lock().library.clone()
    }

    /// Use `lib` as the game's structures (the tests', without the game).
    #[cfg(test)]
    pub fn set_library(&self, lib: Arc<Library>) {
        let mut inner = self.lock();
        inner.library = Some(lib);
        inner.error = None;
    }

    /// Read the game's structures (from `jar`, or the newest game found,
    /// with the mods' `mods`), once, on another thread.
    pub fn ensure_library(&self, ctx: &egui::Context, jar: Option<PathBuf>, mods: Vec<PathBuf>) {
        {
            let mut inner = self.lock();
            if inner.library.is_some() || inner.loading || inner.error.is_some() {
                return;
            }
            inner.loading = true;
        }
        let rt = self.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let jars = massif_assets::find_jars();
            let chosen = jar.filter(|p| p.is_file()).map(|p| {
                let version = jars.iter().find(|j| j.path == p).map(|j| j.version.clone()).unwrap_or_else(|| p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default());
                (p, version)
            });
            let chosen = chosen.or_else(|| jars.first().map(|j| (j.path.clone(), j.version.clone())));
            let result = match chosen {
                Some((path, version)) => Library::open(&path, &version, &mods).map(Arc::new).map_err(|e| format!("{}: {e}", path.display())),
                None => Err(tr!("No Minecraft installation was found: the structures are read from your own game. Install it with the launcher, or choose its jar in Blocks.").to_owned()),
            };
            let mut inner = rt.lock();
            inner.loading = false;
            match result {
                Ok(lib) => inner.library = Some(lib),
                Err(e) => inner.error = Some(e),
            }
            drop(inner);
            ctx.request_repaint();
        });
    }

    /// The click waiting to be built, once the game's files are read and
    /// nothing else is being built.
    pub fn take_pending(&self) -> Option<(i32, i32, Option<u32>, Arc<Library>)> {
        let mut inner = self.lock();
        let lib = inner.library.clone()?;
        if inner.busy {
            return None;
        }
        let (x, z, replace) = inner.pending.take()?;
        inner.busy = true;
        Some((x, z, replace, lib))
    }

    /// The building is over: placed (its number, where it was clicked) or
    /// not, and what to say of it.
    pub fn done(&self, placed: Option<(i32, i32, u32)>, note: Option<String>) {
        let mut inner = self.lock();
        inner.busy = false;
        if placed.is_some() {
            inner.last = placed;
        }
        inner.note = note;
    }

    pub fn is_busy(&self) -> bool {
        let inner = self.lock();
        inner.busy || inner.pending.is_some()
    }

    /// The boxes of the pieces of the structure `settings` would place,
    /// around the column clicked: worked out on flat land on another
    /// thread, `None` until they are.
    fn layout(&self, ctx: &egui::Context, settings: &StructureSettings) -> Option<Arc<Vec<BBox>>> {
        let kind = settings.kind();
        let key: LayoutKey = (kind.id.to_owned(), settings.seed, settings.turn, settings.depth(kind));
        let mut inner = self.lock();
        if let Some((k, boxes)) = &inner.layout
            && *k == key
        {
            return Some(boxes.clone());
        }
        let lib = inner.library.clone()?;
        if inner.layout_busy.is_some() {
            return inner.layout.as_ref().map(|l| l.1.clone());
        }
        inner.layout_busy = Some(key.clone());
        let req = settings.request(0, 0, 0);
        let rt = self.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let boxes = layout_of(&lib, &req);
            let mut inner = rt.lock();
            inner.layout_busy = None;
            inner.layout = Some((key, Arc::new(boxes)));
            drop(inner);
            ctx.request_repaint();
        });
        inner.layout.as_ref().map(|l| l.1.clone())
    }
}

/// The boxes of the pieces of `req`'s structure placed on flat land at
/// column (0, 0): its jigsaw layout alone when it is a jigsaw structure,
/// else the whole of it built.
fn layout_of(lib: &Library, req: &vanilla::Request) -> Vec<BBox> {
    use massif_structures::ground::Flat;
    let ground = Flat { height: 64, water: if req.kind.group == Group::Water { Some(80) } else { None } };
    if let vanilla::Recipe::Jigsaw(id) = req.kind.recipe
        && let Some((spec, height, _, _)) = vanilla::jigsaw_spec(lib, id)
    {
        let start = match (req.kind.height, spec.project) {
            (_, Some(_)) => [0, height, 0],
            _ => [0, req.at[1], 0],
        };
        let mut rng = massif_structures::rng::Rng::new(req.seed);
        let pieces = massif_structures::jigsaw::assemble(lib, &spec, start, req.turn, &mut rng, &ground);
        return pieces.iter().filter(|p| p.element.location().is_some()).map(|p| p.bbox).collect();
    }
    match vanilla::build(lib, &ground, req) {
        Ok(built) => {
            let s = &built.structure;
            if s.pieces.is_empty() { vec![BBox { min: s.min, max: s.max }] } else { s.pieces.iter().map(|p| BBox { min: p.min, max: p.max }).collect() }
        }
        Err(_) => Vec::new(),
    }
}

/// A click of the tool at column `(x, z)`: build the structure there; with
/// the right button, take away the structure placed there.
pub fn click(doc: &mut Document, tools: &mut ToolState, x: i32, z: i32, inverse: bool) -> Feedback {
    let settings = &tools.structure;
    if inverse {
        return remove_at(doc, x, z);
    }
    let kind = settings.kind();
    let mut inner = settings.rt.lock();
    if let Some(e) = &inner.error {
        return Some(e.clone());
    }
    inner.pending = Some((x, z, None));
    Some(tr!("Assembling {name}…", name = crate::i18n::tr_name(kind.name)))
}

/// Take away the structure whose columns hold `(x, z)` (the last placed,
/// where several do), as one step to undo.
pub fn remove_at(doc: &mut Document, x: i32, z: i32) -> Feedback {
    let dim = doc.dim;
    let found = doc.world.dims[dim].structures.iter().rposition(|s| s.columns().contains(x, z))?;
    let name = doc.world.dims[dim].structures[found].name.clone();
    let shown = crate::i18n::tr_name(&name).to_owned();
    doc.history.begin(tr!("Remove {name}", name = shown.clone()), dim);
    doc.history.edit(&mut doc.world, 0).structures_mut().remove(found);
    doc.commit();
    Some(tr!("{name} taken away", name = shown))
}

/// Draw the structure again where the tool last placed it, of a new draw:
/// in its place, as one step to undo.
fn draw_again(doc: &Document, settings: &mut StructureSettings) {
    let last = settings.rt.lock().last;
    let Some((x, z, id)) = last else { return };
    if !doc.dimension().structures.iter().any(|s| s.id == id) {
        settings.rt.lock().last = None;
        return;
    }
    settings.seed = settings.seed.wrapping_add(1);
    settings.rt.lock().pending = Some((x, z, Some(id)));
}

/// The colours of a structure's blocks: the game's, where the catalogue
/// read them.
pub fn colors_of(s: &mut massif_core::structures::Structure, catalogue: Option<&massif_assets::Catalogue>) {
    s.colors = s
        .palette
        .iter()
        .map(|state| {
            let name = state.split('[').next().unwrap_or(state);
            catalogue.and_then(|cat| cat.block(name)).map_or_else(|| massif_core::objects::guess_color(state), |b| b.color)
        })
        .collect();
}

/// The settings of the tool, in Tool Settings.
pub fn settings(ui: &mut Ui, tools: &mut ToolState, doc: &mut Document) {
    use crate::panels::{hint, section};
    use crate::widgets::{button, checkbox, combo, row, well};
    let s = &mut tools.structure;
    let kind = s.kind();
    well(ui, |ui| {
        row(ui, tr!("Structure"), |ui| {
            combo("structure kind").selected_text(crate::i18n::tr_name(kind.name)).width((ui.available_width() * 0.62).min(190.0)).truncate().show_ui(ui, |ui| {
                for (group, title) in [(Group::Villages, tn!("Villages")), (Group::Surface, tn!("On the land")), (Group::Water, tn!("In the water")), (Group::Underground, tn!("Underground")), (Group::Nether, tn!("From the Nether"))] {
                    ui.label(egui::RichText::new(crate::i18n::tr_name(title)).small().weak());
                    for k in KINDS.iter().filter(|k| k.group == group) {
                        if ui.selectable_label(k.id == s.kind, crate::i18n::tr_name(k.name)).clicked() {
                            s.kind = k.id.to_owned();
                        }
                    }
                }
            });
        });
        if let Some(y) = s.depth(kind) {
            row(ui, tr!("Height"), |ui| {
                let mut v = y;
                if crate::widgets::drag(ui, egui::DragValue::new(&mut v).speed(0.5).range(doc.dimension().min_y..=doc.dimension().max_y - 1).prefix("y ")).changed() {
                    s.set_depth(kind, v);
                }
            });
        }
        row(ui, tr!("Turn"), |ui| {
            let label = |t: Option<u8>| t.map_or_else(|| tr!("Any").to_owned(), |t| format!("{}°", t as u32 * 90));
            combo("structure turn").selected_text(label(s.turn)).width((ui.available_width() * 0.62).min(190.0)).show_ui(ui, |ui| {
                for t in [None, Some(0), Some(1), Some(2), Some(3)] {
                    if ui.selectable_label(s.turn == t, label(t)).clicked() {
                        s.turn = t;
                    }
                }
            });
        });
        row(ui, tr!("Draw"), |ui| {
            crate::widgets::drag(ui, egui::DragValue::new(&mut s.seed).speed(1.0));
            if button(ui, theme::with_icon(Icon::Dices, "")).on_hover_text(tr!("Another draw: other pieces, another layout")).clicked() {
                s.seed = s.seed.wrapping_mul(0x9e37_79b9_7f4a_7c15).wrapping_add(1) % 1_000_000;
            }
        });
    });
    if kind.flattens {
        checkbox(ui, &mut s.flatten, tr!("Flatten the land under its buildings"));
    }
    match kind.height {
        Height::Depth(_) => hint(ui, tr!("Placed at the height set, under the column clicked: its halls are carved out of the rock.")),
        Height::Floor => hint(ui, tr!("Placed on the bottom of the water clicked.")),
        Height::Surface => hint(ui, tr!("Placed on the land clicked.")),
    }
    hint(ui, tr!("Click on the map or in the 3D view to place it · right-click a structure: take it away."));
    let (loading, error, busy, note, last) = {
        let inner = s.rt.lock();
        (inner.loading, inner.error.clone(), inner.busy || inner.pending.is_some(), inner.note.clone(), inner.last)
    };
    if loading {
        hint(ui, tr!("Reading the game's structures…"));
    }
    if let Some(e) = error {
        ui.colored_label(Palette::of(ui).danger, e);
    }
    if busy {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(tr!("Assembling…"));
        });
    } else if let Some(note) = note {
        hint(ui, &note);
    }
    if let Some(lib) = s.rt.library() {
        hint(ui, &tr!("Read from Minecraft {version}.", version = lib.version.clone()));
    }
    if last.is_some() && !busy && button(ui, theme::with_icon(Icon::Dices, tr!("Draw it again"))).on_hover_text(tr!("The last one placed, of another draw, in its place")).clicked() {
        draw_again(doc, s);
    }

    // The structures placed on the map.
    let placed: Vec<(u32, String, [i32; 3], [i32; 3])> = doc.dimension().structures.iter().map(|s| (s.id, s.name.clone(), s.min, s.max)).collect();
    if !placed.is_empty() {
        section(ui, Icon::Castle, tr!("Placed"));
        let mut remove = None;
        well(ui, |ui| {
            for (id, name, min, max) in placed.iter().rev() {
                row(ui, crate::i18n::tr_name(name), |ui| {
                    ui.label(egui::RichText::new(format!("{} {}", (min[0] + max[0]) / 2, (min[2] + max[2]) / 2)).weak().small());
                    if button(ui, theme::with_icon(Icon::Trash2, "")).on_hover_text(tr!("Take it away")).clicked() {
                        remove = Some(*id);
                    }
                });
            }
        });
        if let Some(id) = remove
            && let Some(s) = doc.dimension().structures.iter().find(|s| s.id == id)
        {
            let (x, z) = ((s.min[0] + s.max[0]) / 2, (s.min[2] + s.max[2]) / 2);
            let r = s.columns();
            // The middle may stand in another's columns: take this one.
            let at = if r.contains(x, z) { (x, z) } else { (r.x0, r.z0) };
            let dim = doc.dim;
            if let Some(k) = doc.world.dims[dim].structures.iter().position(|t| t.id == id) {
                let shown = crate::i18n::tr_name(&doc.world.dims[dim].structures[k].name).to_owned();
                doc.history.begin(tr!("Remove {name}", name = shown), dim);
                doc.history.edit(&mut doc.world, 0).structures_mut().remove(k);
                doc.commit();
            }
            let _ = at;
        }
    }
}

/// Draw over the map: the outline of each structure placed, and of what a
/// click would place under the pointer (`hover`, in blocks).
#[allow(clippy::too_many_arguments)]
pub fn draw(ctx: &egui::Context, painter: &egui::Painter, canvas: Rect, screen: &dyn Fn((f32, f32)) -> Pos2, zoom: f32, hover: Option<(i32, i32)>, doc: &Document, tools: &ToolState) {
    let rect_of = |b: &BBox, dx: i32, dz: i32| -> Rect {
        Rect::from_two_pos(screen(((b.min[0] + dx) as f32, (b.min[2] + dz) as f32)), screen(((b.max[0] + 1 + dx) as f32, (b.max[2] + 1 + dz) as f32)))
    };
    let last = tools.structure.rt.lock().last.map(|l| l.2);
    // Those placed: their pieces' boxes, faint, their whole outline, named.
    for s in &doc.dimension().structures {
        let r = s.columns();
        let outline = Rect::from_two_pos(screen((r.x0 as f32, r.z0 as f32)), screen((r.x1 as f32, r.z1 as f32)));
        if !outline.intersects(canvas) {
            continue;
        }
        let color = if Some(s.id) == last { Color32::from_rgb(250, 214, 120) } else { Color32::from_white_alpha(170) };
        for p in &s.pieces {
            let b = BBox { min: p.min, max: p.max };
            painter.rect_stroke(rect_of(&b, 0, 0), 0.0, Stroke::new(1.0, color.gamma_multiply(0.35)), StrokeKind::Inside);
        }
        painter.rect_stroke(outline, 0.0, Stroke::new(1.5, Color32::from_black_alpha(110)), StrokeKind::Outside);
        painter.rect_stroke(outline, 0.0, Stroke::new(1.2, color), StrokeKind::Inside);
        if outline.width() > 60.0 {
            label(painter, outline.left_top() + vec2(4.0, 4.0), &format!("{} · y {}…{}", crate::i18n::tr_name(&s.name), s.min[1], s.max[1]));
        }
    }
    // What a click would place.
    let Some((x, z)) = hover else { return };
    let Some(boxes) = tools.structure.rt.layout(ctx, &tools.structure) else {
        return;
    };
    if boxes.is_empty() {
        return;
    }
    let fill = Color32::from_rgba_unmultiplied(250, 214, 120, 38);
    let line = Stroke::new(1.0, Color32::from_rgb(250, 214, 120));
    let mut all: Option<BBox> = None;
    for b in boxes.iter() {
        let r = rect_of(b, x, z);
        painter.rect_filled(r, 0.0, fill);
        painter.rect_stroke(r, 0.0, line, StrokeKind::Inside);
        all = Some(all.map_or(*b, |a| a.union(b)));
    }
    if let Some(a) = all {
        let r = rect_of(&a, x, z);
        painter.rect_stroke(r, 0.0, Stroke::new(1.5, Color32::from_black_alpha(140)), StrokeKind::Outside);
        let kind = tools.structure.kind();
        let size = format!("{} · {} × {} × {}", crate::i18n::tr_name(kind.name), a.span(0), a.span(2), a.span(1));
        label(painter, pos2(r.left(), r.bottom() + 4.0), &size);
    }
    let _ = zoom;
}

/// A short text on a dark pill, legible over any land.
fn label(painter: &egui::Painter, at: Pos2, text: &str) {
    let galley = painter.layout_no_wrap(text.to_owned(), FontId::proportional(11.5), Color32::from_gray(240));
    let back = Rect::from_min_size(at, galley.size() + vec2(10.0, 4.0));
    painter.rect_filled(back, theme::PILL, Color32::from_black_alpha(165));
    painter.galley(back.min + vec2(5.0, 2.0), galley, Color32::from_gray(240));
}

// ------------------------------------------------------- the village brush --

/// The names of the game's villages as themes (see
/// [`massif_structures::village::built_in`]), as the language files know
/// them.
#[allow(dead_code)]
const THEMES: [&str; 5] = [tn!("Plains"), tn!("Desert"), tn!("Savanna"), tn!("Taiga"), tn!("Snowy")];

/// The settings of the village brush.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VillageSettings {
    /// The theme, by its pack and name (`Countryside`, or the game's
    /// `Plains`).
    pub theme: (String, String),
    /// The folders of the packs added.
    pub packs: Vec<PathBuf>,
    pub walls: bool,
    /// How built up, 20 % to 100 %.
    pub density: f32,
    pub seed: u64,
    #[serde(skip)]
    pub rt: VillageRuntime,
}

impl Default for VillageSettings {
    fn default() -> Self {
        Self { theme: (String::new(), "Plains".into()), packs: Vec::new(), walls: false, density: 0.7, seed: 1, rt: VillageRuntime::default() }
    }
}

/// The themes read, the click waiting to be built.
#[derive(Clone, Default)]
pub struct VillageRuntime(Arc<Mutex<VillageInner>>);

impl PartialEq for VillageRuntime {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl std::fmt::Debug for VillageRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("VillageRuntime")
    }
}

#[derive(Default)]
pub struct VillageInner {
    /// The themes offered, and the packs they were read from.
    themes: Option<(Vec<PathBuf>, Vec<massif_structures::village::Theme>)>,
    errors: Vec<String>,
    pending: bool,
    busy: bool,
    note: Option<String>,
}

impl VillageRuntime {
    fn lock(&self) -> MutexGuard<'_, VillageInner> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The themes offered: the game's, then those of the packs `packs`
    /// (read again when they change).
    pub fn themes(&self, packs: &[PathBuf]) -> Vec<massif_structures::village::Theme> {
        let mut inner = self.lock();
        if inner.themes.as_ref().is_none_or(|(p, _)| p != packs) {
            let mut all = massif_structures::village::built_in();
            let mut errors = Vec::new();
            for p in packs {
                match massif_structures::village::read_pack(p) {
                    Ok(t) => all.extend(t),
                    Err(e) => errors.push(e),
                }
            }
            inner.themes = Some((packs.to_vec(), all));
            inner.errors = errors;
        }
        inner.themes.as_ref().map(|t| t.1.clone()).unwrap_or_default()
    }

    /// The fill asked for, once nothing else is being built.
    pub fn take_pending(&self) -> bool {
        let mut inner = self.lock();
        if inner.busy || !inner.pending {
            return false;
        }
        inner.pending = false;
        inner.busy = true;
        true
    }

    pub fn is_busy(&self) -> bool {
        let inner = self.lock();
        inner.busy || inner.pending
    }

    pub fn done(&self, note: Option<String>) {
        let mut inner = self.lock();
        inner.busy = false;
        inner.note = note;
    }

    fn ask(&self) {
        self.lock().pending = true;
    }
}

impl VillageSettings {
    /// The theme chosen, among those offered.
    pub fn theme(&self) -> Option<massif_structures::village::Theme> {
        let themes = self.rt.themes(&self.packs);
        themes.iter().find(|t| t.pack == self.theme.0 && t.name == self.theme.1).or_else(|| themes.first()).cloned()
    }
}

/// A click of the village brush: fill the selection, if the click is in it.
pub fn village_click(doc: &mut Document, tools: &mut ToolState, x: i32, z: i32) -> Feedback {
    let Some(sel) = doc.selection() else {
        return Some(tr!("Select an area first: the village fills the selection.").to_owned());
    };
    if sel.get(x, z) < 128 {
        return Some(tr!("Click inside the selection: the village fills it.").to_owned());
    }
    tools.village.rt.ask();
    Some(tr!("Building the village…").to_owned())
}

/// The settings of the village brush, in Tool Settings.
pub fn village_settings(ui: &mut Ui, tools: &mut ToolState, doc: &mut Document) {
    use crate::panels::hint;
    use crate::widgets::{Field, button, checkbox, combo, row, well};
    let v = &mut tools.village;
    let themes = v.rt.themes(&v.packs);
    let chosen = v.theme();
    well(ui, |ui| {
        row(ui, tr!("Theme"), |ui| {
            let shown = chosen.as_ref().map_or_else(String::new, |t| if t.pack.is_empty() { crate::i18n::tr_name(&t.name).to_owned() } else { format!("{} · {}", t.pack, t.name) });
            combo("village theme").selected_text(shown).width((ui.available_width() * 0.62).min(190.0)).truncate().show_ui(ui, |ui| {
                ui.label(egui::RichText::new(tr!("The game's villages")).small().weak());
                for t in themes.iter().filter(|t| t.pack.is_empty()) {
                    if ui.selectable_label(v.theme == (t.pack.clone(), t.name.clone()), crate::i18n::tr_name(&t.name)).clicked() {
                        v.theme = (t.pack.clone(), t.name.clone());
                    }
                }
                if themes.iter().any(|t| !t.pack.is_empty()) {
                    ui.label(egui::RichText::new(tr!("Packs")).small().weak());
                }
                for t in themes.iter().filter(|t| !t.pack.is_empty()) {
                    if ui.selectable_label(v.theme == (t.pack.clone(), t.name.clone()), format!("{} · {}", t.pack, t.name)).clicked() {
                        v.theme = (t.pack.clone(), t.name.clone());
                    }
                }
            });
        });
        row(ui, tr!("Density"), |ui| {
            ui.add(Field::new(&mut v.density, 0.2..=1.0).custom_formatter(|x, _| format!("{:.0}%", x * 100.0)).custom_parser(|s| s.trim_end_matches('%').trim().parse::<f64>().ok().map(|x| x / 100.0)).width((ui.available_width() * 0.55).min(120.0)));
        });
        row(ui, tr!("Draw"), |ui| {
            crate::widgets::drag(ui, egui::DragValue::new(&mut v.seed).speed(1.0));
            if button(ui, theme::with_icon(Icon::Dices, "")).on_hover_text(tr!("Another draw: other pieces, another layout")).clicked() {
                v.seed = v.seed.wrapping_mul(0x9e37_79b9_7f4a_7c15).wrapping_add(1) % 1_000_000;
            }
        });
    });
    checkbox(ui, &mut v.walls, tr!("Walls, with towers and gates"));
    if button(ui, theme::with_icon(Icon::FolderPlus, tr!("Add a pack…"))).on_hover_text(tr!("A folder of schematics by theme and type: see Village packs in the guide")).clicked()
        && let Some(dir) = rfd::FileDialog::new().set_title(tr!("A village pack")).pick_folder()
        && !v.packs.contains(&dir)
    {
        v.packs.push(dir);
    }
    for e in v.rt.lock().errors.clone() {
        ui.colored_label(Palette::of(ui).danger, e);
    }
    let selection = doc.selection().map(|s| s.area());
    match selection {
        Some(area) => {
            // As large as a square of its area.
            let side = area.sqrt().round() as i64;
            let size = match side {
                ..160 => tr!("a hamlet"),
                160..480 => tr!("a village"),
                _ => tr!("a town of several districts"),
            };
            hint(ui, &tr!("The selection, as large as {side} × {side} blocks: {size}.", side, size));
            let busy = v.rt.is_busy();
            if ui.add_enabled(!busy, egui::Button::new(theme::with_icon(Icon::House, tr!("Fill the selection"))).truncate()).clicked() {
                v.rt.ask();
            }
        }
        None => hint(ui, tr!("Select an area first (Select group): the village fills it, sized by it.")),
    }
    let (busy, note) = {
        let inner = v.rt.lock();
        (inner.busy || inner.pending, inner.note.clone())
    };
    if busy {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(tr!("Building the village…"));
        });
    } else if let Some(note) = note {
        hint(ui, &note);
    }
}
