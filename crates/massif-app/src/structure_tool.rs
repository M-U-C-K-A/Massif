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
use massif_structures::placed::Recipe;
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
    /// What a click places: a village of the village settings' theme, or
    /// the game's structure `kind`.
    pub village: bool,
    /// The structure placed that is selected on the map (its number).
    #[serde(skip)]
    pub selected: Option<u32>,
    #[serde(skip)]
    pub rt: Runtime,
}

impl Default for StructureSettings {
    fn default() -> Self {
        Self { kind: "stronghold".into(), seed: 1, turn: None, depths: Vec::new(), flatten: true, village: true, selected: None, rt: Runtime::default() }
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

    /// The recipe of the game's structure a click at `(x, z)` places.
    pub fn recipe(&self, x: i32, z: i32) -> Recipe {
        let kind = self.kind();
        Recipe::Game { kind: kind.id.to_owned(), at: [x, self.depth(kind).unwrap_or(0), z], turn: self.turn, seed: self.seed, flatten: self.flatten && kind.flattens }
    }
}

/// What a recipe of the game's structure asks for, numbered `id`.
pub fn request_of(recipe: &Recipe, id: u32) -> Option<vanilla::Request> {
    let Recipe::Game { kind, at, turn, seed, flatten } = recipe else { return None };
    let kind = vanilla::kind(kind)?;
    Some(vanilla::Request { kind, at: *at, turn: *turn, seed: *seed, flatten: *flatten, id })
}

/// A drag of the tool on the map, under way.
#[derive(Debug, Clone, PartialEq)]
pub enum Gesture {
    /// A new one: from where it was pressed to where the pointer is.
    Place { from: (f64, f64), to: (f64, f64) },
    /// The selected one carried along by the pointer.
    Move { id: u32, recipe: Recipe, from: (f64, f64), to: (f64, f64) },
    /// The selected village's corner `corner` (0 north-west, 1 north-east,
    /// 2 south-east, 3 south-west) dragged.
    Resize { id: u32, recipe: Recipe, corner: usize, to: (f64, f64) },
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
    pending: Option<(Recipe, Option<u32>)>,
    /// Being built.
    busy: bool,
    layout: Option<(LayoutKey, Arc<Vec<BBox>>)>,
    layout_busy: Option<LayoutKey>,
    /// The last structure the tool placed: where it was clicked and its
    /// number, for Draw again.
    last: Option<(i32, i32, u32)>,
    note: Option<String>,
    /// A drag on the map under way.
    gesture: Option<Gesture>,
    /// A village to draw again with new settings once they stop changing:
    /// which, with what, since when.
    regen: Option<(u32, Recipe, std::time::Instant)>,
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
    pub fn take_pending(&self) -> Option<(Recipe, Option<u32>, Arc<Library>)> {
        let mut inner = self.lock();
        let lib = inner.library.clone()?;
        if inner.busy {
            return None;
        }
        let (recipe, replace) = inner.pending.take()?;
        inner.busy = true;
        Some((recipe, replace, lib))
    }

    /// The drag under way, if one.
    pub fn gesture(&self) -> Option<Gesture> {
        self.lock().gesture.clone()
    }

    fn set_gesture(&self, g: Option<Gesture>) {
        self.lock().gesture = g;
    }

    /// Draw village `id` again as `recipe` once its settings rest.
    pub fn regenerate(&self, id: u32, recipe: Recipe) {
        self.lock().regen = Some((id, recipe, std::time::Instant::now()));
    }

    /// The village whose settings have rested a third of a second, to draw
    /// again.
    pub fn due_regen(&self) -> Option<(u32, Recipe)> {
        let mut inner = self.lock();
        let due = inner.regen.as_ref().is_some_and(|r| r.2.elapsed() >= std::time::Duration::from_millis(350));
        if due { inner.regen.take().map(|(id, r, _)| (id, r)) } else { None }
    }

    /// Whether a village waits to be drawn again.
    pub fn regen_waiting(&self) -> bool {
        self.lock().regen.is_some()
    }

    pub fn say(&self, note: impl Into<String>) {
        self.lock().note = Some(note.into());
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

    /// Why the game's structures cannot be read, if they cannot.
    pub fn error(&self) -> Option<String> {
        self.lock().error.clone()
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

/// The recipe a structure placed by the tool keeps, if it has one.
pub fn recipe_of(s: &massif_core::structures::Structure) -> Option<Recipe> {
    s.recipe.as_deref().and_then(Recipe::from_json)
}

/// The rectangle a structure placed takes on the map: its village's area,
/// else its blocks' columns.
pub fn footprint(s: &massif_core::structures::Structure) -> massif_core::BlockRect {
    recipe_of(s).and_then(|r| r.area()).unwrap_or_else(|| s.columns())
}

/// The structure placed whose rectangle holds `(x, z)`: the selected one
/// first, else the last placed.
pub fn hit(doc: &Document, selected: Option<u32>, x: i32, z: i32) -> Option<u32> {
    let all = &doc.dimension().structures;
    if let Some(id) = selected
        && all.iter().any(|s| s.id == id && footprint(s).contains(x, z))
    {
        return Some(id);
    }
    all.iter().rev().find(|s| footprint(s).contains(x, z)).map(|s| s.id)
}

/// A click of the tool at column `(x, z)` (in the 3D view, or on the map
/// without dragging): select the structure there, else place what the
/// tool holds; with the right button, take away the structure there.
pub fn click(doc: &mut Document, tools: &mut ToolState, x: i32, z: i32, inverse: bool) -> Feedback {
    if inverse {
        return remove_at(doc, tools, x, z);
    }
    if let Some(id) = hit(doc, tools.structure.selected, x, z) {
        tools.structure.selected = Some(id);
        return doc.dimension().structures.iter().find(|s| s.id == id).map(|s| tr!("{name} selected: drag it to move it, its corners to resize it", name = crate::i18n::tr_name(&s.name).to_owned()));
    }
    place(doc, tools, x, z, None)
}

/// Place what the tool holds at `(x, z)`: a village of the size chosen
/// round it (over `rect` when one was dragged), or the game's structure.
fn place(doc: &Document, tools: &mut ToolState, x: i32, z: i32, rect: Option<massif_core::BlockRect>) -> Feedback {
    let _ = doc;
    tools.structure.selected = None;
    if tools.structure.village {
        let side = tools.village.size.side();
        let rect = rect.unwrap_or_else(|| massif_core::BlockRect::new(x - side / 2, z - side / 2, x - side / 2 + side, z - side / 2 + side));
        let recipe = tools.village.recipe(rect);
        tools.village.rt.ask(VillageJob { recipe, mask: None, replace: None });
        return Some(tr!("Building the village…").to_owned());
    }
    let settings = &tools.structure;
    let kind = settings.kind();
    let mut inner = settings.rt.lock();
    if let Some(e) = &inner.error {
        return Some(e.clone());
    }
    inner.pending = Some((settings.recipe(x, z), None));
    Some(tr!("Assembling {name}…", name = crate::i18n::tr_name(kind.name)))
}

/// Fill the selection with a village of the settings chosen.
pub fn fill_selection(doc: &Document, tools: &mut ToolState) -> Feedback {
    let Some(sel) = doc.selection().cloned() else { return Some(tr!("Nothing is selected").to_owned()) };
    let Some(rect) = sel.bounds() else { return Some(tr!("Nothing is selected").to_owned()) };
    let recipe = tools.village.recipe(rect);
    tools.structure.selected = None;
    tools.village.rt.ask(VillageJob { recipe, mask: Some(sel), replace: None });
    Some(tr!("Building the village…").to_owned())
}

/// Make structure `id` again as `recipe` (moved, resized, of another
/// draw), in its place.
pub fn redo(tools: &mut ToolState, id: u32, recipe: Recipe) {
    match &recipe {
        Recipe::Village { .. } => tools.village.rt.ask(VillageJob { recipe, mask: None, replace: Some(id) }),
        Recipe::Game { .. } => tools.structure.rt.lock().pending = Some((recipe, Some(id))),
    }
}

/// Take away structure `id`, the land it levelled given back, as one step
/// to undo.
pub fn remove(doc: &mut Document, tools: &mut ToolState, id: u32) -> Feedback {
    let dim = doc.dim;
    let found = doc.world.dims[dim].structures.iter().position(|s| s.id == id)?;
    let s = doc.world.dims[dim].structures[found].clone();
    let shown = crate::i18n::tr_name(&s.name).to_owned();
    doc.history.begin(tr!("Remove {name}", name = shown.clone()), dim);
    {
        let mut edit = doc.history.edit(&mut doc.world, 0);
        massif_core::structures::restore_ground(&mut edit, &s.ground);
        edit.structures_mut().remove(found);
    }
    doc.commit();
    if tools.structure.selected == Some(id) {
        tools.structure.selected = None;
    }
    Some(tr!("{name} taken away", name = shown))
}

/// Take away the structure at `(x, z)`, as one step to undo.
pub fn remove_at(doc: &mut Document, tools: &mut ToolState, x: i32, z: i32) -> Feedback {
    let id = hit(doc, tools.structure.selected, x, z)?;
    remove(doc, tools, id)
}

/// The corners of a rectangle, north-west first, clockwise.
fn corners(r: massif_core::BlockRect) -> [(f32, f32); 4] {
    [(r.x0 as f32, r.z0 as f32), (r.x1 as f32, r.z0 as f32), (r.x1 as f32, r.z1 as f32), (r.x0 as f32, r.z1 as f32)]
}

/// The pointer pressed on the map at `at` (blocks; `pointer` on the
/// screen): a corner of the selected village grabbed, the selected one or
/// another grabbed to move, or a new one begun.
pub fn press(doc: &Document, tools: &mut ToolState, at: (f64, f64), pointer: Pos2, screen: &dyn Fn((f32, f32)) -> Pos2) -> Feedback {
    let (x, z) = (at.0.floor() as i32, at.1.floor() as i32);
    let rt = tools.structure.rt.clone();
    if let Some(id) = tools.structure.selected
        && let Some(s) = doc.dimension().structures.iter().find(|s| s.id == id)
        && let Some(recipe) = recipe_of(s)
    {
        if let Some(area) = recipe.area()
            && let Some(corner) = corners(area).iter().position(|c| screen(*c).distance(pointer) <= 10.0)
        {
            rt.set_gesture(Some(Gesture::Resize { id, recipe, corner, to: at }));
            return None;
        }
        if footprint(s).contains(x, z) {
            rt.set_gesture(Some(Gesture::Move { id, recipe, from: at, to: at }));
            return None;
        }
    }
    if let Some(id) = hit(doc, None, x, z) {
        tools.structure.selected = Some(id);
        let s = doc.dimension().structures.iter().find(|s| s.id == id)?;
        if let Some(recipe) = recipe_of(s) {
            rt.set_gesture(Some(Gesture::Move { id, recipe, from: at, to: at }));
        }
        return Some(tr!("{name} selected: drag it to move it, its corners to resize it", name = crate::i18n::tr_name(&s.name).to_owned()));
    }
    tools.structure.selected = None;
    rt.set_gesture(Some(Gesture::Place { from: at, to: at }));
    None
}

/// The pointer, held, at `at`.
pub fn drag(tools: &ToolState, at: (f64, f64)) {
    let mut inner = tools.structure.rt.lock();
    match &mut inner.gesture {
        Some(Gesture::Place { to, .. } | Gesture::Move { to, .. } | Gesture::Resize { to, .. }) => *to = at,
        None => {}
    }
}

/// What a resize leaves: the rectangle with corner `corner` at `to`, the
/// opposite one where it was, at least 32 blocks a side.
fn resized(area: massif_core::BlockRect, corner: usize, to: (f64, f64)) -> massif_core::BlockRect {
    let (x, z) = (to.0.round() as i32, to.1.round() as i32);
    let (mut x0, mut z0, mut x1, mut z1) = (area.x0, area.z0, area.x1, area.z1);
    match corner {
        0 => (x0, z0) = (x.min(x1 - 32), z.min(z1 - 32)),
        1 => (x1, z0) = (x.max(x0 + 32), z.min(z1 - 32)),
        2 => (x1, z1) = (x.max(x0 + 32), z.max(z0 + 32)),
        _ => (x0, z1) = (x.min(x1 - 32), z.max(z0 + 32)),
    }
    massif_core::BlockRect::new(x0, z0, x1, z1)
}

/// A village's recipe over another rectangle.
fn over(recipe: &Recipe, r: massif_core::BlockRect) -> Recipe {
    recipe.over(r)
}

/// The pointer let go: what the drag did.
pub fn release(doc: &Document, tools: &mut ToolState) -> Feedback {
    let gesture = tools.structure.rt.lock().gesture.take()?;
    match gesture {
        Gesture::Place { from, to } => {
            let (x0, z0, x1, z1) = (from.0.min(to.0).floor() as i32, from.1.min(to.1).floor() as i32, from.0.max(to.0).ceil() as i32, from.1.max(to.1).ceil() as i32);
            if tools.structure.village && x1 - x0 >= 32 && z1 - z0 >= 32 {
                place(doc, tools, (x0 + x1) / 2, (z0 + z1) / 2, Some(massif_core::BlockRect::new(x0, z0, x1, z1)))
            } else if tools.structure.village && (x1 - x0 >= 8 || z1 - z0 >= 8) {
                Some(tr!("Drag a larger rectangle for a village: 32 × 32 blocks at least").to_owned())
            } else {
                place(doc, tools, from.0.floor() as i32, from.1.floor() as i32, None)
            }
        }
        Gesture::Move { id, recipe, from, to } => {
            let (dx, dz) = ((to.0 - from.0).round() as i32, (to.1 - from.1).round() as i32);
            if dx == 0 && dz == 0 {
                return None;
            }
            redo(tools, id, recipe.moved(dx, dz));
            Some(tr!("Moving it…").to_owned())
        }
        Gesture::Resize { id, recipe, corner, to } => {
            let area = recipe.area()?;
            let r = resized(area, corner, to);
            if r == area {
                return None;
            }
            redo(tools, id, over(&recipe, r));
            Some(tr!("Building the village…").to_owned())
        }
    }
}

/// Place `built` in the map as one edit: the structure it takes the place
/// of (`replace`) gone with the land it levelled given back, the land
/// under `built` levelled, its heights before kept with it.
pub fn apply_placed(edit: &mut massif_core::history::Edit, built: &mut vanilla::Built, replace: Option<u32>) {
    if let Some(r) = replace
        && let Some(k) = edit.dim().structures.iter().position(|s| s.id == r)
    {
        let old = edit.dim().structures[k].clone();
        massif_core::structures::restore_ground(edit, &old.ground);
        edit.structures_mut().remove(k);
    }
    built.structure.ground = massif_core::structures::heights_before(edit.dim(), &built.heights);
    vanilla::apply(edit, built);
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

/// The name of a type of building, as the panel shows `n` of them.
fn type_name(t: &str, n: u32) -> String {
    let one = n == 1;
    let s = match t {
        "house" => if one { tr!("house") } else { tr!("houses") },
        "manor" => if one { tr!("manor") } else { tr!("manors") },
        "church" => if one { tr!("church") } else { tr!("churches") },
        "forge" => if one { tr!("forge") } else { tr!("forges") },
        "tavern" => if one { tr!("tavern") } else { tr!("taverns") },
        "watchtower" => if one { tr!("watchtower") } else { tr!("watchtowers") },
        "farm" => if one { tr!("farm") } else { tr!("farms") },
        "stable" => if one { tr!("stable") } else { tr!("stables") },
        "mill" => if one { tr!("mill") } else { tr!("mills") },
        "well" => if one { tr!("well") } else { tr!("wells") },
        "market" => if one { tr!("market stall") } else { tr!("market stalls") },
        other => return other.replace('_', " "),
    };
    s.to_owned()
}

/// How many of each type a village of `houses` houses has: those asked
/// for, else what the pack's shares give (at most its max), market stalls
/// round the square.
fn estimate(theme: &massif_structures::village::Theme, houses: u32, counts: &[(String, u32)]) -> Vec<(String, u32)> {
    let house_share = theme.kind("house").map_or(0.55, |t| t.share).max(0.05);
    let mut out = Vec::new();
    for t in &theme.types {
        if t.name == "house" {
            continue;
        }
        let asked = counts.iter().find(|(k, _)| *k == t.name).map(|c| c.1);
        let drawn = if t.name == "market" && t.share <= 0.0 {
            t.max.unwrap_or(4)
        } else if t.name == "mill" {
            1
        } else {
            let n = (houses as f32 * t.share / house_share).round() as u32;
            let n = if t.centre { n.max(1) } else { n };
            t.max.map_or(n, |m| n.min(m))
        };
        out.push((t.name.clone(), asked.unwrap_or(drawn)));
    }
    out
}

/// How the land of `r` suits a village: the share too steep, the share
/// under water, and about how many houses the rest holds.
fn site_check(doc: &Document, r: massif_core::BlockRect) -> (f32, f32, u32) {
    let dim = doc.dimension();
    let h = |x: i32, z: i32| massif_core::ops::height_at(dim, 0, x, z, massif_core::ops::Surface::Top).unwrap_or(0.0);
    let (mut all, mut steep, mut wet) = (0u32, 0u32, 0u32);
    for z in (r.z0..r.z1).step_by(4) {
        for x in (r.x0..r.x1).step_by(4) {
            all += 1;
            if massif_core::ops::water_at(dim, x, z).is_some() {
                wet += 1;
            } else if (h(x + 4, z) - h(x, z)).abs() > 3.0 || (h(x, z + 4) - h(x, z)).abs() > 3.0 {
                steep += 1;
            }
        }
    }
    let all = all.max(1);
    let good = (all - steep - wet) as f32 * 16.0;
    (steep as f32 / all as f32, wet as f32 / all as f32, (good / 340.0) as u32)
}

/// What a village will be, in detail: a line saying it whole, its
/// buildings type by type (counts a pack's village takes as asked), the
/// pack's schematics, and how its land suits it. Whether a count changed.
#[allow(clippy::too_many_arguments)]
fn village_details(ui: &mut Ui, summary: bool, id: &str, doc: &Document, theme: Option<&massif_structures::village::Theme>, size: VillageSize, houses: u32, road: massif_structures::village::RoadType, walls: bool, counts: &mut Vec<(String, u32)>, built: Option<&massif_core::structures::Structure>, area: Option<massif_core::BlockRect>) -> bool {
    use crate::panels::hint;
    use crate::widgets::{Field, row, well};
    use massif_structures::village::RoadType;
    let mut changed = false;
    let pack = theme.is_some_and(|t| t.vanilla.is_none());
    // What it has: as built, or as it will be.
    let have: Vec<(String, u32)> = match built {
        Some(s) => {
            let mut m: Vec<(String, u32)> = Vec::new();
            for p in s.pieces.iter().filter(|p| !p.template.starts_with("prop:")) {
                let k = if pack { p.template.clone() } else { "building".into() };
                match m.iter_mut().find(|e| e.0 == k) {
                    Some(e) => e.1 += 1,
                    None => m.push((k, 1)),
                }
            }
            m
        }
        None => {
            let mut m = vec![("house".to_owned(), houses)];
            if let Some(t) = theme.filter(|_| pack) {
                m.extend(estimate(t, houses, counts));
            }
            m
        }
    };
    let roads = match road {
        RoadType::Theme => tr!("the pack's roads"),
        RoadType::Dirt => tr!("dirt roads"),
        RoadType::Premium => tr!("gravel roads"),
        RoadType::Stone => tr!("stone roads"),
        RoadType::Planks => tr!("plank roads"),
    };
    let mut parts = vec![tr!("{size} village", size = size.label()).to_owned()];
    for (k, n) in &have {
        if *n > 0 {
            parts.push(if k == "building" { tr!("{n} buildings", n) } else { format!("{n} {}", type_name(k, *n)) });
        }
    }
    parts.push(roads.to_owned());
    if walls {
        parts.push(tr!("walls").to_owned());
    }
    if summary {
        ui.add(egui::Label::new(egui::RichText::new(parts.join(" · ")).strong()).wrap());
        return false;
    }
    if !pack {
        hint(ui, tr!("The game's own villages grow as the game grows them: their houses, farms and wells are its own."));
    }
    // Each type of the pack, how many: as asked, or as its shares draw.
    if let Some(t) = theme.filter(|_| pack) {
        ui.label(egui::RichText::new(tr!("Buildings")).small().weak());
        well(ui, |ui| {
            for (k, n) in estimate(t, houses, counts) {
                let max = if k == "market" { 12 } else { 20 };
                let mut v = n;
                row(ui, &type_name(&k, 2), |ui| {
                    if ui.add(Field::new(&mut v, 0..=max).width((ui.available_width() * 0.55).min(120.0))).changed() {
                        counts.retain(|c| c.0 != k);
                        counts.push((k.clone(), v));
                        changed = true;
                    }
                });
            }
        });
        hint(ui, tr!("How many of each the village has, as far as its land has room: the church and the wells by the square, the farms and towers toward the edge."));
        egui::CollapsingHeader::new(tr!("The pack's buildings ({n})", n = t.types.iter().map(|k| k.variants.len()).sum::<usize>())).id_salt(format!("{id} pack list")).show(ui, |ui| {
            for k in &t.types {
                let sides: Vec<i32> = k.variants.iter().map(|v| { let f = v.footprint(); f[0].max(f[1]) }).collect();
                let (lo, hi) = (sides.iter().min().copied().unwrap_or(0), sides.iter().max().copied().unwrap_or(0));
                hint(ui, &format!("{} · {} · {lo}–{hi} {}", type_name(&k.name, 2), k.variants.len(), tr!("blocks")));
                let names: Vec<String> = k.variants.iter().map(|v| { let f = v.footprint(); format!("{} {}×{}", v.file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(), f[0], f[1]) }).collect();
                ui.label(egui::RichText::new(names.join(", ")).small().weak());
            }
        });
    }
    // The land: what it needs, how the area suits it.
    let need = ((houses as f32 * 340.0).sqrt() * 1.15) as i32;
    match area {
        Some(r) => {
            let (steep, wet, fits) = site_check(doc, r);
            hint(ui, &tr!("Its land: {flat:.0}% flat and dry, {steep:.0}% too steep, {wet:.0}% under water.", flat = (1.0 - steep - wet) * 100.0, steep = steep * 100.0, wet = wet * 100.0));
            if pack && fits < houses {
                ui.colored_label(Palette::of(ui).danger, tr!("This land holds about {fits} houses, not {houses}: make the village larger, or ask for fewer.", fits, houses));
            }
        }
        None => hint(ui, &tr!("It needs about {need} × {need} blocks of flat, dry land: the houses stand on slopes of at most 10 blocks, never in water.", need)),
    }
    changed
}

/// The settings of a village: size, houses, roads, walls, draw. `pack`:
/// whether its theme is a pack's (which may have roads of its own).
/// Whether one changed.
#[allow(clippy::too_many_arguments)]
fn village_controls(ui: &mut Ui, id: &str, size: &mut VillageSize, houses: &mut u32, by_hand: &mut bool, road: &mut massif_structures::village::RoadType, walls: &mut bool, seed: &mut u64, pack: bool) -> bool {
    let has_own_roads = pack;
    use crate::widgets::{Field, button, row, well};
    use massif_structures::village::RoadType;
    let mut changed = false;
    ui.label(egui::RichText::new(tr!("Size")).small().weak());
    let on: Vec<bool> = VillageSize::ALL.iter().map(|s| s == size).collect();
    let labels: Vec<&str> = VillageSize::ALL.iter().map(|s| s.label()).collect();
    if let Some(k) = crate::widgets::button_group(ui, &format!("{id} size"), &labels, &on) {
        *size = VillageSize::ALL[k];
        if !*by_hand {
            *houses = size.houses();
        }
        changed = true;
    }
    crate::panels::hint(ui, tr!("How close its houses stand: a small village's few and far apart, a large one's many and close. How many follows its area."));
    ui.label(egui::RichText::new(tr!("Roads")).small().weak());
    let mut kinds = vec![RoadType::Dirt, RoadType::Premium, RoadType::Stone];
    let mut labels = vec![tr!("Dirt"), tr!("Gravel"), tr!("Stone")];
    if pack && has_own_roads {
        kinds.insert(0, RoadType::Theme);
        labels.insert(0, tr!("Pack's"));
    }
    let on: Vec<bool> = kinds.iter().map(|k| k == road).collect();
    if let Some(k) = crate::widgets::button_group(ui, &format!("{id} roads"), &labels, &on) {
        *road = kinds[k];
        changed = true;
    }
    crate::panels::hint(ui, tr!("The blocks of its streets; stone climbs in steps."));
    well(ui, |ui| {
        // A pack's village holds as many houses as asked; the game's own
        // grow as the game grows them.
        if pack {
            row(ui, tr!("Houses"), |ui| {
                if ui.add(Field::new(houses, 3..=120).width((ui.available_width() * 0.55).min(120.0))).changed() {
                    *by_hand = true;
                    changed = true;
                }
                if *by_hand && button(ui, theme::with_icon(Icon::RotateCcw, "")).on_hover_text(tr!("Set by hand: let them follow its area again")).clicked() {
                    *by_hand = false;
                    *houses = size.houses();
                    changed = true;
                }
            });
        }
        row(ui, tr!("Walls"), |ui| {
            changed |= crate::widgets::switch(ui, walls).changed();
        });
        row(ui, tr!("Draw"), |ui| {
            changed |= crate::widgets::drag(ui, egui::DragValue::new(seed).speed(1.0)).changed();
            if button(ui, theme::with_icon(Icon::Dices, "")).on_hover_text(tr!("Another draw: other houses, another layout")).clicked() {
                *seed = seed.wrapping_mul(0x9e37_79b9_7f4a_7c15).wrapping_add(1) % 1_000_000;
                changed = true;
            }
        });
    });
    crate::panels::hint(ui, tr!("Walls: ramparts round it, towers at the corners, gates where its roads leave. Draw: the same number gives the same village."));
    changed
}

/// The settings of the tool, in Tool Settings: what a click places (a
/// village of a theme, or one of the game's structures) and how, then the
/// structure selected on the map.
pub fn settings(ui: &mut Ui, tools: &mut ToolState, doc: &mut Document) {
    use crate::panels::{hint, section};
    use crate::widgets::{button, checkbox, combo, row, well};
    // The one selected on the map.
    let selected = tools.structure.selected.and_then(|id| doc.dimension().structures.iter().find(|s| s.id == id).cloned());
    if tools.structure.selected.is_some() && selected.is_none() {
        tools.structure.selected = None;
    }
    if let Some(s) = selected {
        section(ui, Icon::MousePointer2, tr!("Selected"));
        hint(ui, &format!("{} · {} × {}", crate::i18n::tr_name(&s.name), s.max[0] - s.min[0] + 1, s.max[2] - s.min[2] + 1));
        let recipe = recipe_of(&s);
        ui.horizontal_wrapped(|ui| {
            if let Some(r) = &recipe
                && button(ui, theme::with_icon(Icon::Dices, tr!("Draw it again"))).on_hover_text(tr!("Of another draw, in its place")).clicked()
            {
                let seed = r.seed().wrapping_mul(0x9e37_79b9_7f4a_7c15).wrapping_add(1) % 1_000_000;
                match r {
                    Recipe::Village { .. } => tools.structure.rt.regenerate(s.id, r.with_seed(seed)),
                    Recipe::Game { .. } => redo(tools, s.id, r.with_seed(seed)),
                }
            }
            if button(ui, theme::with_icon(Icon::Trash2, tr!("Take it away"))).clicked() {
                let note = remove(doc, tools, s.id);
                if let Some(n) = note {
                    tools.structure.rt.say(n);
                }
            }
        });
        match recipe {
            Some(Recipe::Village { pack, theme, centre, size, params, seed }) => {
                let mut vsize = VillageSize::of_density(params.density);
                let area_now = Recipe::Village { pack: pack.clone(), theme: theme.clone(), centre, size, params: params.clone(), seed }.area();
                let fallback = vsize.houses();
                let follow = |d: f32| area_now.map_or(fallback, |r| massif_structures::village::houses_for(r, d));
                let mut by_hand = params.houses_by_hand;
                let mut houses = params.houses.unwrap_or_else(|| follow(params.density));
                let (mut road, mut walls, mut seed2) = (params.road, params.walls, seed);
                let mut counts = params.counts.clone();
                let th = tools.village.theme_named(&pack, &theme);
                let area = area_now;
                village_details(ui, true, "selected village", doc, th.as_ref(), vsize, houses, road, walls, &mut counts, Some(&s), area);
                let mut changed = village_controls(ui, "selected village", &mut vsize, &mut houses, &mut by_hand, &mut road, &mut walls, &mut seed2, !pack.is_empty());
                changed |= village_details(ui, false, "selected village", doc, th.as_ref(), vsize, houses, road, walls, &mut counts, Some(&s), area);
                if changed {
                    // A size is a density over the same land: its houses
                    // follow the area unless typed by hand.
                    let density = if vsize != VillageSize::of_density(params.density) { vsize.density() } else { params.density };
                    if !by_hand {
                        houses = follow(density);
                    }
                    let params = massif_structures::village::Params { houses: Some(houses), houses_by_hand: by_hand, density, road, walls, counts, ..params };
                    tools.structure.rt.regenerate(s.id, Recipe::Village { pack, theme, centre, size, params, seed: seed2 });
                }
                hint(ui, tr!("Drag it to move it, its corners to resize it: it is built again."));
            }
            Some(Recipe::Game { .. }) => hint(ui, tr!("Drag it to move it.")),
            None => hint(ui, tr!("Placed before it could be moved: take it away and place it again to move it.")),
        }
    }

    let themes = tools.village.rt.themes(&tools.village.packs);
    // The village selected on the map, whose theme the list shows and a
    // click in it changes: it is built again of the theme picked.
    let picked = tools.structure.selected.and_then(|id| doc.dimension().structures.iter().find(|s| s.id == id).and_then(|s| Some((id, recipe_of(s)?)))).filter(|(_, r)| matches!(r, Recipe::Village { .. }));
    // What a click places: the list.
    let village_now = tools.structure.village;
    section(ui, Icon::House, tr!("Villages"));
    well(ui, |ui| {
        for t in &themes {
            let name = if t.pack.is_empty() { crate::i18n::tr_name(&t.name).to_owned() } else { format!("{} · {}", t.pack, t.name) };
            let on = match &picked {
                Some((_, Recipe::Village { pack, theme, .. })) => *pack == t.pack && *theme == t.name,
                _ => tools.structure.village && tools.village.theme == (t.pack.clone(), t.name.clone()),
            };
            if ui.add(egui::Button::selectable(on, name).frame_when_inactive(false).truncate()).clicked() {
                tools.structure.village = true;
                tools.village.theme = (t.pack.clone(), t.name.clone());
                if !t.pack.is_empty() && tools.village.road == massif_structures::village::RoadType::Dirt {
                    tools.village.road = massif_structures::village::RoadType::Theme;
                }
                if let Some((id, Recipe::Village { pack, theme, centre, size, params, seed })) = &picked
                    && (*pack != t.pack || *theme != t.name)
                {
                    use massif_structures::village::RoadType;
                    // A pack's roads where it has its own; the game's
                    // themes have none.
                    let road = match (t.pack.is_empty(), params.road) {
                        (true, RoadType::Theme) => RoadType::Dirt,
                        (false, RoadType::Dirt) => RoadType::Theme,
                        (_, r) => r,
                    };
                    let params = massif_structures::village::Params { road, counts: Vec::new(), ..params.clone() };
                    tools.structure.rt.regenerate(*id, Recipe::Village { pack: t.pack.clone(), theme: t.name.clone(), centre: *centre, size: *size, params, seed: *seed });
                }
            }
        }
    });
    if button(ui, theme::with_icon(Icon::FolderPlus, tr!("Add a pack…"))).on_hover_text(tr!("A folder of schematics by theme and type: see Village packs in the guide")).clicked()
        && let Some(dir) = rfd::FileDialog::new().set_title(tr!("A village pack")).pick_folder()
        && !tools.village.packs.contains(&dir)
    {
        tools.village.packs.push(dir);
    }
    // The other list folded away while a village is picked, so that its
    // settings come close under the list.
    if village_now {
        if button(ui, theme::with_icon(Icon::Castle, tr!("The game's structures…"))).clicked() {
            tools.structure.village = false;
        }
    } else {
        section(ui, Icon::Castle, tr!("The game's structures"));
        well(ui, |ui| {
        for (group, title) in [(Group::Surface, tn!("On the land")), (Group::Water, tn!("In the water")), (Group::Underground, tn!("Underground")), (Group::Nether, tn!("From the Nether"))] {
            ui.label(egui::RichText::new(crate::i18n::tr_name(title)).small().weak());
            for k in KINDS.iter().filter(|k| k.group == group) {
                let on = !tools.structure.village && k.id == tools.structure.kind;
                if ui.add(egui::Button::selectable(on, crate::i18n::tr_name(k.name)).frame_when_inactive(false).truncate()).clicked() {
                    tools.structure.village = false;
                    tools.structure.kind = k.id.to_owned();
                }
            }
        }
        });
    }

    // How: the settings of what a click places.
    if tools.structure.village {
        let v = &mut tools.village;
        let pack = !v.theme.0.is_empty();
        let mut houses = v.houses.unwrap_or_else(|| v.size.houses());
        let mut by_hand = v.houses.is_some();
        let (mut size, mut road, mut walls, mut seed) = (v.size, v.road, v.walls, v.seed);
        let th = v.theme_named(&v.theme.0, &v.theme.1);
        let mut counts = v.counts.clone();
        village_details(ui, true, "new village", doc, th.as_ref(), size, houses, road, walls, &mut counts, None, None);
        let controls = village_controls(ui, "new village", &mut size, &mut houses, &mut by_hand, &mut road, &mut walls, &mut seed, pack);
        if village_details(ui, false, "new village", doc, th.as_ref(), size, houses, road, walls, &mut counts, None, None) {
            v.counts = counts;
        }
        if controls {
            (v.size, v.road, v.walls, v.seed) = (size, road, walls, seed);
            v.houses = by_hand.then_some(houses);
        }
        hint(ui, &tr!("Click on the map: a village {side} blocks across. Drag a rectangle: a village over it.", side = v.size.side()));
        if doc.selection().is_some() && !v.rt.is_busy() && button(ui, theme::with_icon(Icon::House, tr!("Fill the selection"))).clicked() {
            let note = fill_selection(doc, tools);
            if let Some(n) = note {
                tools.structure.rt.say(n);
            }
        }
    } else {
        let s = &mut tools.structure;
        let kind = s.kind();
        well(ui, |ui| {
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
        hint(ui, tr!("Click on the map or in the 3D view to place it."));
    }

    // Why nothing happens, and what is happening.
    let (loading, error, busy, note) = {
        let inner = tools.structure.rt.lock();
        (inner.loading, inner.error.clone(), inner.busy || inner.pending.is_some(), inner.note.clone())
    };
    let village_busy = tools.village.rt.is_busy() || tools.structure.rt.regen_waiting();
    let needs_game = !tools.structure.village || tools.village.theme.0.is_empty();
    if loading && needs_game {
        hint(ui, tr!("Reading the game's structures…"));
    }
    if let Some(e) = error.filter(|_| needs_game) {
        ui.colored_label(Palette::of(ui).danger, e);
    }
    for e in tools.village.rt.lock().errors.clone() {
        ui.colored_label(Palette::of(ui).danger, e);
    }
    if busy || village_busy {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(if village_busy { tr!("Building the village…") } else { tr!("Assembling…") });
        });
    } else if let Some(note) = tools.village.rt.lock().note.clone().or(note) {
        hint(ui, &note);
    }
    let problems = tools.village.rt.lock().problems.clone();
    if !problems.is_empty() {
        egui::CollapsingHeader::new(tr!("{n} notes on the packs", n = problems.len())).id_salt("village pack notes").show(ui, |ui| {
            for p in problems.iter().take(40) {
                let text = format!("{} {}", p.file, p.message);
                if p.serious {
                    ui.colored_label(Palette::of(ui).danger, text);
                } else {
                    hint(ui, &text);
                }
            }
        });
    }

    // All those placed.
    let placed: Vec<(u32, String, [i32; 3], [i32; 3])> = doc.dimension().structures.iter().map(|s| (s.id, s.name.clone(), s.min, s.max)).collect();
    if !placed.is_empty() {
        section(ui, Icon::Castle, tr!("Placed"));
        well(ui, |ui| {
            for (id, name, min, max) in placed.iter().rev() {
                let on = tools.structure.selected == Some(*id);
                let text = format!("{}  ·  {} {}", crate::i18n::tr_name(name), (min[0] + max[0]) / 2, (min[2] + max[2]) / 2);
                if ui.add(egui::Button::selectable(on, text).frame_when_inactive(false).truncate()).clicked() {
                    tools.structure.selected = Some(*id);
                }
            }
        });
    }
}

/// Draw over the map: each structure placed (the selected one with its
/// corners to drag), what a drag would place or where it carries one, and
/// what a click would place under the pointer (`hover`, in blocks).
#[allow(clippy::too_many_arguments)]
pub fn draw(ctx: &egui::Context, painter: &egui::Painter, canvas: Rect, screen: &dyn Fn((f32, f32)) -> Pos2, zoom: f32, hover: Option<(i32, i32)>, doc: &Document, tools: &ToolState) {
    let pal = {
        let v = ctx.global_style().visuals.clone();
        Palette::themed(v.dark_mode, theme::is_high(&v))
    };
    let accent = pal.accent;
    let rect_of = |b: &BBox, dx: i32, dz: i32| -> Rect {
        Rect::from_two_pos(screen(((b.min[0] + dx) as f32, (b.min[2] + dz) as f32)), screen(((b.max[0] + 1 + dx) as f32, (b.max[2] + 1 + dz) as f32)))
    };
    let area_rect = |r: massif_core::BlockRect| Rect::from_two_pos(screen((r.x0 as f32, r.z0 as f32)), screen((r.x1 as f32, r.z1 as f32)));
    let selected = tools.structure.selected;
    let gesture = tools.structure.rt.gesture();
    // Those placed: their pieces' boxes, faint, their outline, named.
    for s in &doc.dimension().structures {
        let outline = area_rect(footprint(s));
        if !outline.intersects(canvas) {
            continue;
        }
        let on = Some(s.id) == selected;
        let color = if on { accent } else { Color32::from_white_alpha(170) };
        for p in &s.pieces {
            let b = BBox { min: p.min, max: p.max };
            painter.rect_stroke(rect_of(&b, 0, 0), 0.0, Stroke::new(1.0, color.gamma_multiply(0.35)), StrokeKind::Inside);
        }
        painter.rect_stroke(outline, 0.0, Stroke::new(1.5, Color32::from_black_alpha(110)), StrokeKind::Outside);
        painter.rect_stroke(outline, 0.0, Stroke::new(if on { 2.0 } else { 1.2 }, color), StrokeKind::Inside);
        if outline.width() > 60.0 {
            label(painter, outline.left_top() + vec2(4.0, 4.0), &format!("{} · y {}…{}", crate::i18n::tr_name(&s.name), s.min[1], s.max[1]));
        }
        // The selected village's corners, to drag.
        if on && recipe_of(s).and_then(|r| r.area()).is_some() {
            for c in corners(footprint(s)) {
                let p = screen(c);
                let r = Rect::from_center_size(p, vec2(9.0, 9.0));
                painter.rect_filled(r, theme::TILE_RADIUS / 4, accent);
                painter.rect_stroke(r, theme::TILE_RADIUS / 4, Stroke::new(1.0, Color32::from_black_alpha(160)), StrokeKind::Outside);
            }
        }
    }
    // A drag under way: the rectangle it covers.
    let drawn = match &gesture {
        Some(Gesture::Place { from, to }) if tools.structure.village => {
            let r = massif_core::BlockRect::new(from.0.min(to.0).floor() as i32, from.1.min(to.1).floor() as i32, from.0.max(to.0).ceil() as i32, from.1.max(to.1).ceil() as i32);
            (r.x1 - r.x0 >= 4 || r.z1 - r.z0 >= 4).then_some(r)
        }
        Some(Gesture::Move { id, from, to, .. }) => doc.dimension().structures.iter().find(|s| s.id == *id).map(|s| {
            let r = footprint(s);
            let (dx, dz) = ((to.0 - from.0).round() as i32, (to.1 - from.1).round() as i32);
            massif_core::BlockRect::new(r.x0 + dx, r.z0 + dz, r.x1 + dx, r.z1 + dz)
        }),
        Some(Gesture::Resize { recipe, corner, to, .. }) => recipe.area().map(|a| resized(a, *corner, *to)),
        _ => None,
    };
    if let Some(r) = drawn {
        let sr = area_rect(r);
        painter.rect_filled(sr, 0.0, accent.gamma_multiply(0.15));
        painter.rect_stroke(sr, 0.0, Stroke::new(1.5, accent), StrokeKind::Inside);
        label(painter, pos2(sr.left(), sr.bottom() + 4.0), &format!("{} × {}", r.x1 - r.x0, r.z1 - r.z0));
        return;
    }
    // What a click would place, where no structure is under the pointer.
    let Some((x, z)) = hover else { return };
    if hit(doc, selected, x, z).is_some() {
        return;
    }
    if tools.structure.village {
        let side = tools.village.size.side();
        let r = massif_core::BlockRect::new(x - side / 2, z - side / 2, x - side / 2 + side, z - side / 2 + side);
        let sr = area_rect(r);
        painter.rect_filled(sr, 0.0, accent.gamma_multiply(0.08));
        painter.rect_stroke(sr, 0.0, Stroke::new(1.0, accent.gamma_multiply(0.8)), StrokeKind::Inside);
        return;
    }
    let Some(boxes) = tools.structure.rt.layout(ctx, &tools.structure) else {
        return;
    };
    if boxes.is_empty() {
        return;
    }
    let fill = accent.gamma_multiply(0.15);
    let line = Stroke::new(1.0, accent);
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
    /// From small houses (−1) to large ones and manors (+1).
    pub size_bias: f32,
    /// The blocks of the roads.
    pub road: massif_structures::village::RoadType,
    /// How many props: none (0) to many (1).
    pub props: f32,
    /// How many houses; none: as many as the selection holds.
    pub houses: Option<u32>,
    pub seed: u64,
    /// How large a village a click places.
    pub size: VillageSize,
    /// How many of each type of building, where told.
    pub counts: Vec<(String, u32)>,
    #[serde(skip)]
    pub rt: VillageRuntime,
}

/// How large a village a click places: the side of its square, and the
/// houses it holds unless told otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum VillageSize {
    Small,
    #[default]
    Medium,
    Large,
}

impl VillageSize {
    pub const ALL: [VillageSize; 3] = [VillageSize::Small, VillageSize::Medium, VillageSize::Large];

    pub fn side(self) -> i32 {
        match self {
            VillageSize::Small => 112,
            VillageSize::Medium => 192,
            VillageSize::Large => 320,
        }
    }

    /// How close its houses stand: a small village's scattered, a large
    /// one's tight.
    pub fn density(self) -> f32 {
        match self {
            VillageSize::Small => 0.35,
            VillageSize::Medium => 0.6,
            VillageSize::Large => 0.85,
        }
    }

    /// The houses its square holds at its density.
    pub fn houses(self) -> u32 {
        massif_structures::village::houses_for(massif_core::BlockRect::new(0, 0, self.side(), self.side()), self.density())
    }

    /// The size whose density is nearest `d`.
    pub fn of_density(d: f32) -> VillageSize {
        *VillageSize::ALL.iter().min_by(|a, b| (a.density() - d).abs().total_cmp(&(b.density() - d).abs())).expect("three")
    }

    pub fn label(self) -> &'static str {
        match self {
            VillageSize::Small => tr!("Small"),
            VillageSize::Medium => tr!("Medium"),
            VillageSize::Large => tr!("Large"),
        }
    }

}

/// A village to build: what (its recipe), over what (a selection's mask,
/// when it fills one), in place of which one placed before.
#[derive(Debug, Clone)]
pub struct VillageJob {
    pub recipe: Recipe,
    pub mask: Option<Arc<massif_core::selection::Selection>>,
    pub replace: Option<u32>,
}

impl Default for VillageSettings {
    fn default() -> Self {
        Self { theme: (String::new(), "Plains".into()), packs: Vec::new(), walls: false, density: 0.7, size_bias: 0.0, road: massif_structures::village::RoadType::Dirt, props: 0.5, houses: None, seed: 1, size: VillageSize::Medium, counts: Vec::new(), rt: VillageRuntime::default() }
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
    /// What the packs' check found, file by file.
    problems: Vec<massif_structures::village::Problem>,
    pending: Option<VillageJob>,
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
            let mut problems = Vec::new();
            for p in packs {
                match massif_structures::village::read_pack_checked(p) {
                    Ok(r) => {
                        all.extend(r.themes);
                        problems.extend(r.problems);
                    }
                    Err(e) => errors.push(e),
                }
            }
            inner.themes = Some((packs.to_vec(), all));
            inner.errors = errors;
            inner.problems = problems;
        }
        inner.themes.as_ref().map(|t| t.1.clone()).unwrap_or_default()
    }

    /// The village asked for, once nothing else is being built.
    pub fn take_pending(&self) -> Option<VillageJob> {
        let mut inner = self.lock();
        if inner.busy {
            return None;
        }
        let job = inner.pending.take()?;
        inner.busy = true;
        Some(job)
    }

    pub fn is_busy(&self) -> bool {
        let inner = self.lock();
        inner.busy || inner.pending.is_some()
    }

    /// The village asked for, not taken yet.
    pub fn peek_pending(&self) -> Option<VillageJob> {
        let inner = self.lock();
        if inner.busy { None } else { inner.pending.clone() }
    }

    /// The village asked for cannot be built: say why.
    pub fn drop_pending(&self, note: Option<String>) {
        let mut inner = self.lock();
        inner.pending = None;
        inner.note = note;
    }

    /// Build `job` (in place of any asked for before and not begun).
    pub fn ask(&self, job: VillageJob) {
        self.lock().pending = Some(job);
    }

    /// The last note and the packs' errors (for tests).
    #[cfg(test)]
    pub fn notes(&self) -> Vec<String> {
        let inner = self.lock();
        inner.note.iter().cloned().chain(inner.errors.iter().cloned()).collect()
    }

    pub fn done(&self, note: Option<String>) {
        let mut inner = self.lock();
        inner.busy = false;
        inner.note = note;
    }

}

impl VillageSettings {
    /// What the village is to be like, from these settings.
    pub fn params(&self) -> massif_structures::village::Params {
        massif_structures::village::Params { walls: self.walls, density: self.density, size_bias: self.size_bias, road: self.road, props: self.props, houses: self.houses, counts: self.counts.clone(), ..Default::default() }
    }

    /// The recipe of a village of these settings over `rect`.
    pub fn recipe(&self, rect: massif_core::BlockRect) -> Recipe {
        let mut params = self.params();
        params.density = self.size.density();
        params.houses_by_hand = self.houses.is_some();
        params.houses = Some(self.houses.unwrap_or_else(|| massif_structures::village::houses_for(rect, params.density)));
        Recipe::Village { pack: self.theme.0.clone(), theme: self.theme.1.clone(), centre: [(rect.x0 + rect.x1) / 2, (rect.z0 + rect.z1) / 2], size: [rect.x1 - rect.x0, rect.z1 - rect.z0], params, seed: self.seed }
    }

    /// The theme a recipe names, among those offered.
    pub fn theme_named(&self, pack: &str, name: &str) -> Option<massif_structures::village::Theme> {
        self.rt.themes(&self.packs).into_iter().find(|t| t.pack == pack && t.name == name)
    }

}

/// A click with the old village brush in hand: the Structures tool's,
/// with a village.
pub fn village_click(doc: &mut Document, tools: &mut ToolState, x: i32, z: i32) -> Feedback {
    tools.structure.village = true;
    click(doc, tools, x, z, false)
}
