//! The panel's previews, made on a thread of their own: each node's
//! thumbnail and the viewer's picture, from the engine's tiles, lit on the
//! graphics card. The viewer comes first and coarse, then the selected
//! node's thumbnail, then the viewer sharp, then the other thumbnails; a
//! newer version of the graph stops the older one's work at its next node.

use super::shade::{self, Camera, Look, Overlay, Relief, Surface};
use massif_terrain::compile::{Compiled, compile_in};
use massif_terrain::engine::Engine;
use massif_terrain::kernel::{Env, Grid, NoEnv};
use massif_terrain::model::Graph;
use massif_terrain::registry::PortKind;
use massif_terrain::space::Space;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Instant;

/// Samples a side of a thumbnail's terrain.
pub const THUMB_SAMPLES: u32 = 128;
/// Its pixels a side.
pub const THUMB_PX: usize = 128;

/// What a node output is shown as.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Shown {
    pub node: String,
    /// Its output, by name (the first when `None`); for a Map Output, the
    /// input shown (`None`: the map as a whole: height and albedo).
    pub port: Option<String>,
}

/// How the viewer looks.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewerLook {
    pub three_d: bool,
    pub surface: Surface,
    pub sun: [f32; 2],
    pub yaw: f32,
    pub pitch: f32,
    pub distance: f32,
    /// Times the automatic exaggeration.
    pub relief: f32,
    pub contours: bool,
    /// The mask laid over the relief, 0 to 1.
    pub overlay: f32,
    /// Pixels.
    pub size: [usize; 2],
    pub back: [u8; 3],
    /// In 2D: the part of the terrain shown, as fractions (x0, z0, x1, z1).
    pub window: [f32; 4],
    /// In 2D: the centre of the view (fractions of the terrain), and how
    /// much of the terrain the shorter side shows.
    pub center: [f32; 2],
    pub span: f32,
}

impl Default for ViewerLook {
    fn default() -> Self {
        Self { three_d: false, surface: Surface::Colour, sun: [315.0, 40.0], yaw: 25.0, pitch: 34.0, distance: 1.0, relief: 1.0, contours: false, overlay: 0.85, size: [640, 400], back: [20, 20, 21], window: [0.0, 0.0, 1.0, 1.0], center: [0.5, 0.5], span: 1.0 }
    }
}

/// Colours of the categories, by id: the world's terrains and biomes.
#[derive(Debug, Clone, Default)]
pub struct Categories {
    pub terrains: Vec<[u8; 3]>,
    pub biomes: Vec<[u8; 3]>,
}

impl Categories {
    pub fn of(reg: &massif_core::registry::Registry) -> Self {
        Self { terrains: reg.terrains.iter().map(|t| t.color).collect(), biomes: reg.biomes.iter().map(|b| b.color).collect() }
    }
}

/// What the panel asks for.
#[derive(Clone)]
pub struct Request {
    pub revision: u64,
    pub graph: Graph,
    pub space: Space,
    pub sim_res: u32,
    pub env: Arc<dyn Env>,
    /// Thumbnails wanted, the selected first.
    pub thumbs: Vec<Shown>,
    /// Their pixels a side.
    pub thumb_px: usize,
    /// The graph as Live compiled it, when Live shows it: evaluated as it
    /// is, its tiles are Live's.
    pub compiled: Option<Arc<Compiled>>,
    pub thumb_surface: Surface,
    pub viewer: Option<(Shown, ViewerLook)>,
    pub categories: Arc<Categories>,
}

/// A picture made.
pub struct Picture {
    pub revision: u64,
    pub what: Shown,
    /// The viewer's (rather than a thumbnail).
    pub viewer: bool,
    /// Sharp (rather than the coarse first one).
    pub sharp: bool,
    pub size: [usize; 2],
    pub rgba: Vec<u8>,
    /// Lowest and highest value shown, and its unit's type.
    pub range: (f32, f32),
    pub kind: Option<PortKind>,
    pub ms: f64,
}

/// What a thread reports of its work.
#[derive(Debug, Clone, Default)]
pub struct Status {
    pub revision: u64,
    pub busy: bool,
    pub doing: String,
    pub error: Option<(String, String)>,
    /// Per node: why it cannot be computed.
    pub broken: HashMap<String, String>,
    pub diagnostics: Vec<String>,
    pub last_ms: f64,
    pub cache_mb: f64,
    pub gpu: Option<String>,
}

struct Shared {
    job: Mutex<Option<Request>>,
    wake: Condvar,
    done: Mutex<VecDeque<Picture>>,
    status: Mutex<Status>,
    generation: AtomicU64,
    repaint: Mutex<Option<egui::Context>>,
}

/// The thread that makes the previews.
pub struct Previews {
    shared: Arc<Shared>,
    engine: Arc<Engine>,
}

impl Previews {
    pub fn new(engine: Arc<Engine>) -> Self {
        let shared = Arc::new(Shared { job: Mutex::new(None), wake: Condvar::new(), done: Mutex::default(), status: Mutex::default(), generation: AtomicU64::new(0), repaint: Mutex::new(None) });
        let (s, e) = (shared.clone(), engine.clone());
        std::thread::Builder::new().name("terrain previews".into()).spawn(move || run(s, e)).expect("a thread");
        Self { shared, engine }
    }

    pub fn engine(&self) -> &Arc<Engine> {
        &self.engine
    }

    /// Asks for `req`, dropping what an older request still had to do.
    pub fn ask(&self, req: Request, ctx: &egui::Context) {
        *self.shared.repaint.lock().expect("lock") = Some(ctx.clone());
        self.shared.generation.store(req.revision, Ordering::SeqCst);
        *self.shared.job.lock().expect("lock") = Some(req);
        self.shared.wake.notify_one();
    }

    /// The pictures made since the last call.
    pub fn take(&self) -> Vec<Picture> {
        self.shared.done.lock().expect("lock").drain(..).collect()
    }

    pub fn status(&self) -> Status {
        self.shared.status.lock().expect("lock").clone()
    }

    /// Whether everything asked is made (for tests).
    pub fn idle(&self) -> bool {
        self.shared.job.lock().expect("lock").is_none() && !self.shared.status.lock().expect("lock").busy
    }
}

impl Drop for Previews {
    fn drop(&mut self) {
        self.shared.generation.store(u64::MAX, Ordering::SeqCst);
        *self.shared.job.lock().expect("lock") = None;
        self.shared.wake.notify_one();
    }
}

fn run(shared: Arc<Shared>, engine: Arc<Engine>) {
    loop {
        let req = {
            let mut job = shared.job.lock().expect("lock");
            while job.is_none() {
                if shared.generation.load(Ordering::SeqCst) == u64::MAX {
                    return;
                }
                job = shared.wake.wait(job).expect("lock");
            }
            let r = job.take().expect("a job");
            shared.status.lock().expect("lock").busy = true;
            r
        };
        if shared.generation.load(Ordering::SeqCst) == u64::MAX {
            return;
        }
        work(&shared, &engine, req);
        let mut s = shared.status.lock().expect("lock");
        s.busy = false;
        s.cache_mb = engine.cache_bytes() as f64 / 1048576.0;
        s.gpu = engine.gpu_name().map(str::to_owned);
        drop(s);
        if let Some(ctx) = shared.repaint.lock().expect("lock").as_ref() {
            ctx.request_repaint();
        }
    }
}

/// What a step of the work makes.
enum Step {
    Viewer { sharp: bool },
    Thumb(usize),
}

fn work(shared: &Shared, engine: &Engine, req: Request) {
    let revision = req.revision;
    let cancel = || shared.generation.load(Ordering::SeqCst) != revision;
    let (own, diags) = match &req.compiled {
        Some(c) if c.space == req.space => (None, Vec::new()),
        _ => {
            let (c, d) = compile_in(&req.graph, req.space, req.sim_res, req.env.clone());
            (Some(c), d)
        }
    };
    let c: &Compiled = match (&own, &req.compiled) {
        (Some(c), _) => c,
        (None, Some(c)) => c,
        (None, None) => unreachable!("compiled one way or the other"),
    };
    {
        let mut s = shared.status.lock().expect("lock");
        s.revision = revision;
        s.broken = c.nodes.iter().filter_map(|n| n.broken.clone().map(|b| (n.id.clone(), b))).collect();
        s.diagnostics = diags.iter().map(|d| d.to_string()).collect();
        s.error = None;
    }
    let mut steps = Vec::new();
    if req.viewer.is_some() {
        steps.push(Step::Viewer { sharp: false });
    }
    if !req.thumbs.is_empty() {
        steps.push(Step::Thumb(0));
    }
    if req.viewer.is_some() {
        steps.push(Step::Viewer { sharp: true });
    }
    steps.extend((1..req.thumbs.len()).map(Step::Thumb));
    let push = |p: Picture| {
        shared.done.lock().expect("lock").push_back(p);
        if let Some(ctx) = shared.repaint.lock().expect("lock").as_ref() {
            ctx.request_repaint();
        }
    };
    let mut seen = HashSet::new();
    for step in steps {
        if cancel() {
            return;
        }
        let t0 = Instant::now();
        let (what, viewer, sharp, made) = match step {
            Step::Viewer { sharp } => {
                let (what, look) = req.viewer.clone().expect("a viewer");
                let samples = if sharp { (look.size[0].max(look.size[1]) as u32).clamp(256, 1024) } else { 160 };
                shared.status.lock().expect("lock").doing = format!("{}{}", what.node, if sharp { "" } else { " (coarse)" });
                (what.clone(), true, sharp, picture(engine, c, &what, samples, Some(&look), &req, &cancel))
            }
            Step::Thumb(k) => {
                let what = req.thumbs[k].clone();
                if !seen.insert(what.clone()) {
                    continue;
                }
                shared.status.lock().expect("lock").doing = what.node.clone();
                (what.clone(), false, true, picture(engine, c, &what, req.thumb_px as u32, None, &req, &cancel))
            }
        };
        match made {
            Ok(Some((size, rgba, range, kind))) => {
                let ms = t0.elapsed().as_secs_f64() * 1000.0;
                shared.status.lock().expect("lock").last_ms = ms;
                push(Picture { revision, what, viewer, sharp, size, rgba, range, kind, ms });
            }
            Ok(None) => {}
            Err(e) => {
                if cancel() {
                    return;
                }
                shared.status.lock().expect("lock").error = Some((what.node.clone(), e));
            }
        }
    }
}

type Made = Option<([usize; 2], Vec<u8>, (f32, f32), Option<PortKind>)>;

/// The node output feeding `what`, and its type.
fn source(c: &Compiled, what: &Shown) -> Option<((usize, u8), PortKind)> {
    let n = c.node(&what.node)?;
    let spec = c.nodes[n].spec;
    if spec.is_output() {
        let name = what.port.as_deref().unwrap_or("height");
        let mo = c.map_output().filter(|m| m.node == n);
        // Another Map Output than the active one: its inputs as they are.
        let src = match mo {
            Some(m) => m.get(name),
            None => {
                let k = spec.input(name)?;
                c.nodes[n].inputs.get(k).copied().flatten().or_else(|| c.nodes[n].open.iter().find(|o| o.0 == name).map(|o| o.1))
            }
        }?;
        return Some((src, c.kind(src)));
    }
    let port = match &what.port {
        Some(p) => spec.output(p)? as u8,
        None => 0,
    };
    Some(((n, port), c.kind((n, port))))
}

/// The nearest height upstream of `src`, the relief a mask or colour is
/// laid on; the Map Output's height when there is none.
fn relief_of(c: &Compiled, src: (usize, u8)) -> Option<(usize, u8)> {
    let mut queue = VecDeque::from([src.0]);
    let mut seen = HashSet::new();
    while let Some(n) = queue.pop_front() {
        if !seen.insert(n) {
            continue;
        }
        for i in c.nodes[n].inputs.iter().flatten().chain(c.nodes[n].driven.iter().flatten()) {
            if c.kind(*i) == PortKind::Height {
                return Some(*i);
            }
            queue.push_back(i.0);
        }
    }
    c.output("height")
}

/// The relief lit, what colours it, and the mask over it with its colour.
type Shading = (Grid, Option<Grid>, Option<(Grid, [u8; 3])>);

fn picture(engine: &Engine, c: &Compiled, what: &Shown, samples: u32, look: Option<&ViewerLook>, req: &Request, cancel: &(dyn Fn() -> bool + Sync)) -> Result<Made, String> {
    let (thumb_surface, cats) = (req.thumb_surface, &*req.categories);
    let n = c.node(&what.node).ok_or("no such node")?;
    let whole_map = c.nodes[n].spec.is_output() && what.port.is_none();
    let Some((src, kind)) = (if whole_map { c.output("height").map(|s| (s, PortKind::Height)) } else { source(c, what) }) else {
        return Ok(None);
    };
    let lod = c.space.lod_for(samples);
    let window = look.map_or([0.0, 0.0, 1.0, 1.0], |l| if l.three_d { [0.0, 0.0, 1.0, 1.0] } else { l.window });
    let whole = c.space.whole(lod);
    // The samples the 2D viewer's window covers, at a level sharp for it.
    let (lod, rect) = if look.is_some_and(|l| !l.three_d) && window != [0.0, 0.0, 1.0, 1.0] {
        let span = (window[2] - window[0]).max(window[3] - window[1]).max(1e-3);
        let lod = c.space.lod_for((samples as f32 / span) as u32);
        let n = c.space.samples(lod) as f32;
        // Only the terrain: beyond its edges the picture shows the ground
        // of the card.
        let x0 = ((window[0] * n).floor() as i32 - 1).max(0);
        let z0 = ((window[1] * n).floor() as i32 - 1).max(0);
        let x1 = ((window[2] * n).ceil() as i32 + 1).min(n as i32);
        let z1 = ((window[3] * n).ceil() as i32 + 1).min(n as i32);
        if x1 - x0 < 2 || z1 - z0 < 2 {
            return Ok(None);
        }
        (lod, massif_terrain::space::Rect { x: x0, z: z0, w: (x1 - x0) as u32, h: (z1 - z0) as u32 })
    } else {
        (lod, whole)
    };
    let eval = |s: (usize, u8)| engine.region(c, s, lod, rect, cancel).map_err(|e| e.to_string());
    // Geometry is drawn over the relief it stands on; a volume shows as its
    // cut under the sea's level, laid on that relief.
    let (geo, volume) = (kind.is_geometry().then_some(src), kind.is_volume().then_some(src));
    let shown_kind = kind;
    let (src, kind) = match kind.is_raster() {
        true => (src, kind),
        false => match relief_of(c, src) {
            Some(r) => (r, PortKind::Height),
            None => return Err(t!("Nothing to show here yet").to_owned()),
        },
    };
    let sea = c.space.sea_level;
    let main = eval(src)?;
    let cut = match volume {
        Some(v) => Some(volume_cut(engine, c, v, (sea as i32) - 16, lod, rect, cancel).map_err(|e| e.to_string())?),
        None => None,
    };
    let range = main.range();
    // Heights that are not a relief (a mask kept as heights, all under the
    // sea, all flat) are shown as a field: stretched over a relief, dry.
    let field = kind == PortKind::Height && (range.1 - range.0 < 6.0 || range.1 < sea - 1.0);
    let (heights, colors, overlay): Shading = match kind {
        PortKind::Height if field => {
            let side = c.space.size as f32;
            let span = (range.1 - range.0).max(1e-6);
            let lifted = Grid { data: main.data.iter().map(|h| sea + 2.0 + (h - range.0) / span * side * 0.02).collect(), ..main.clone() };
            (lifted, None, None)
        }
        PortKind::Height => {
            let colors = if whole_map || what.port.as_deref() == Some("height") { c.output("albedo").map(eval).transpose()? } else { None };
            (main, colors, None)
        }
        PortKind::Mask => {
            let h = relief_of(c, src).map(eval).transpose()?.unwrap_or_else(|| shade::flat_like(&main, sea));
            let tint = super::sockets::color(PortKind::Mask).to_array();
            (h, None, Some((main, [tint[0], tint[1], tint[2]])))
        }
        PortKind::Color => {
            let h = relief_of(c, src).map(eval).transpose()?.unwrap_or_else(|| shade::flat_like(&main, sea));
            (h, Some(main), None)
        }
        PortKind::Vector => {
            let h = relief_of(c, src).map(eval).transpose()?.unwrap_or_else(|| shade::flat_like(&main, sea));
            (h, Some(vector_colors(&main)), None)
        }
        PortKind::Layer => {
            // The layer's strength, over the relief.
            let h = relief_of(c, src).map(eval).transpose()?.unwrap_or_else(|| shade::flat_like(&main, sea));
            let strength = Grid { planes: 1, data: main.plane(1).to_vec(), ..main.clone() };
            let tint = super::sockets::color(PortKind::Layer).to_array();
            (h, None, Some((strength, [tint[0], tint[1], tint[2]])))
        }
        PortKind::Points | PortKind::Splines | PortKind::Footprints | PortKind::Instances | PortKind::Density => unreachable!("shown above"),
        PortKind::Terrain | PortKind::Biome => {
            let h = relief_of(c, src).map(eval).transpose()?.unwrap_or_else(|| shade::flat_like(&main, sea));
            let table = if kind == PortKind::Terrain { &cats.terrains } else { &cats.biomes };
            (h, Some(category_colors(&main, table)), None)
        }
    };
    if cancel() {
        return Ok(None);
    }
    let surface = match (look, kind) {
        (_, PortKind::Color | PortKind::Terrain | PortKind::Biome | PortKind::Vector) => Surface::Colour,
        (Some(l), _) => l.surface,
        (None, _) => thumb_surface,
    };
    let relief = Relief { heights: &heights, colors: colors.as_ref(), sea };
    let overlay = match cut {
        Some(m) => {
            let tint = super::sockets::color(PortKind::Density).to_array();
            Some((m, [tint[0], tint[1], tint[2]]))
        }
        None => overlay,
    };
    let ov = overlay.as_ref().map(|(m, col)| Overlay { mask: m, color: *col, opacity: look.map_or(0.9, |l| l.overlay) });
    let (size, shade_look) = match look {
        None => ([samples as usize, samples as usize], Look { exaggeration: shade::auto_exaggeration(&heights, sea) * 1.6, camera: Camera::Top { view: view_of(&heights, rect, whole, [0.0, 0.0, 1.0, 1.0], c.space.samples(lod)) }, back: [0, 0, 0], water: !field, ..Look::default() }),
        Some(l) => {
            let camera = if l.three_d { Camera::Orbit { yaw: l.yaw, pitch: l.pitch, distance: l.distance } } else { Camera::Top { view: view_of(&heights, rect, whole, window, c.space.samples(lod)) } };
            let exaggeration = if l.three_d { shade::auto_exaggeration_3d(&heights, sea) } else { shade::auto_exaggeration(&heights, sea) } * l.relief;
            let contours = if l.contours { contour_step(range) } else { 0.0 };
            (l.size, Look { sun: l.sun, exaggeration, camera, contours, back: l.back, water: !field, ..Look::default() })
        }
    };
    let mut rgba = shade::shade(engine.gpu().map(|g| &**g), &relief, ov, surface, &shade_look, size);
    // Geometry, drawn over the picture seen from above.
    if let Some(g) = geo
        && look.is_none_or(|l| !l.three_d)
    {
        let shown = engine.geometry(c, g, cancel).map_err(|e| e.to_string())?;
        let win = look.map_or([0.0, 0.0, 1.0, 1.0], |l| l.window);
        draw_geometry(&mut rgba, size, &shown, c, win, super::sockets::color(shown_kind).to_array());
    }
    let kind = if shown_kind.is_raster() { kind } else { shown_kind };
    Ok(Some((size, rgba, range, Some(kind))))
}

/// A volume cut at height `y`, one sample of it under each sample of
/// `rect` at `lod` (1 inside, 0 outside): the bricks under them only, not
/// every block, so that a thumbnail of a large terrain stays quick.
fn volume_cut(engine: &Engine, c: &Compiled, src: (usize, u8), y: i32, lod: u8, rect: massif_terrain::space::Rect, cancel: &(dyn Fn() -> bool + Sync)) -> Result<Grid, massif_terrain::engine::EvalError> {
    use massif_terrain::kernel::BRICK;
    let step = massif_terrain::space::Space::spacing(lod);
    let at = |i: u32, k: u32| -> [i32; 2] { [c.space.x0 + ((rect.x as f64 + i as f64 + 0.5) * step) as i32, c.space.z0 + ((rect.z as f64 + k as f64 + 0.5) * step) as i32] };
    let by = y.div_euclid(BRICK) * BRICK;
    let mut origins: Vec<[i32; 3]> = Vec::new();
    let mut index: std::collections::HashMap<[i32; 3], usize> = std::collections::HashMap::new();
    let mut which = Vec::with_capacity(rect.area());
    for k in 0..rect.h {
        for i in 0..rect.w {
            let [x, z] = at(i, k);
            let o = [x.div_euclid(BRICK) * BRICK, by, z.div_euclid(BRICK) * BRICK];
            let n = *index.entry(o).or_insert_with(|| {
                origins.push(o);
                origins.len() - 1
            });
            which.push((n, x, z));
        }
    }
    let bricks = engine.bricks(c, src, &origins, cancel)?;
    let mut out = Grid::filled(0, rect, 0.0);
    for (s, (n, x, z)) in which.into_iter().enumerate() {
        out.data[s] = massif_terrain::underground::fill(bricks[n].at(x, y, z)) as f32 / 255.0;
    }
    Ok(out)
}

/// Points, lines, footprints or instances drawn over a picture of the
/// terrain seen from above (`window`: the part of the terrain it shows, as
/// fractions), in `color`.
fn draw_geometry(rgba: &mut [u8], size: [usize; 2], geo: &massif_terrain::kernel::Geo, c: &Compiled, window: [f32; 4], color: [u8; 4]) {
    use massif_terrain::kernel::Geo;
    let (w, h) = (size[0] as f64, size[1] as f64);
    let side = c.space.size as f64;
    let (wx, wz) = ((window[2] - window[0]).max(1e-6) as f64, (window[3] - window[1]).max(1e-6) as f64);
    // The picture keeps the terrain's square in its middle.
    let fit = (w / wx).min(h / wz);
    let (ox, oz) = ((w - wx * fit) / 2.0, (h - wz * fit) / 2.0);
    let px = |x: f64, z: f64| -> (f64, f64) { (ox + ((x - c.space.x0 as f64) / side - window[0] as f64) * fit, oz + ((z - c.space.z0 as f64) / side - window[1] as f64) * fit) };
    let mut plot = |x: f64, y: f64, a: f32| {
        if x < 0.0 || y < 0.0 || x >= w || y >= h {
            return;
        }
        let k = (y as usize * size[0] + x as usize) * 4;
        for ch in 0..3 {
            rgba[k + ch] = (rgba[k + ch] as f32 * (1.0 - a) + color[ch] as f32 * a) as u8;
        }
    };
    let line = |a: (f64, f64), b: (f64, f64), plot: &mut dyn FnMut(f64, f64, f32)| {
        let n = ((b.0 - a.0).abs().max((b.1 - a.1).abs()).ceil() as usize).max(1);
        for k in 0..=n {
            let t = k as f64 / n as f64;
            plot(a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t, 1.0);
        }
    };
    let rect = |a: (f64, f64), b: (f64, f64), plot: &mut dyn FnMut(f64, f64, f32)| {
        line(a, (b.0, a.1), plot);
        line((b.0, a.1), b, plot);
        line(b, (a.0, b.1), plot);
        line((a.0, b.1), a, plot);
    };
    match geo {
        Geo::Points(p) => {
            let r = (fit / side * 3.0).clamp(1.0, 3.0);
            for q in &p.items {
                let (x, y) = px(q.pos[0], q.pos[2]);
                let ri = r.ceil() as i32;
                for dz in -ri..=ri {
                    for dx in -ri..=ri {
                        let d = ((dx * dx + dz * dz) as f64).sqrt();
                        if d <= r {
                            plot(x + dx as f64, y + dz as f64, if d > r - 1.0 { 0.6 } else { 1.0 });
                        }
                    }
                }
            }
        }
        Geo::Splines(s) => {
            for l in &s.lines {
                let pts: Vec<(f64, f64)> = l.points.iter().map(|p| px(p[0], p[2])).collect();
                for pair in pts.windows(2) {
                    line(pair[0], pair[1], &mut plot);
                }
                if l.closed && pts.len() > 2 {
                    line(pts[pts.len() - 1], pts[0], &mut plot);
                }
            }
        }
        Geo::Footprints(f) => {
            for q in &f.items {
                rect(px(q.min[0] as f64, q.min[1] as f64), px(q.max[0] as f64, q.max[1] as f64), &mut plot);
            }
        }
        Geo::Instances(i) => {
            for q in &i.items {
                rect(px(q.bbox.0[0] as f64, q.bbox.0[2] as f64), px(q.bbox.1[0] as f64 + 1.0, q.bbox.1[2] as f64 + 1.0), &mut plot);
            }
        }
    }
}

/// The window, in fractions of the terrain, as samples of `grid` (whose
/// rect is `rect` among `n` samples a side).
fn view_of(grid: &Grid, rect: massif_terrain::space::Rect, _whole: massif_terrain::space::Rect, window: [f32; 4], n: u32) -> [f32; 4] {
    let _ = grid;
    let n = n as f32;
    [window[0] * n - rect.x as f32, window[1] * n - rect.z as f32, window[2] * n - rect.x as f32, window[3] * n - rect.z as f32]
}

/// Contour lines a sensible number of blocks apart for a range of heights.
fn contour_step(range: (f32, f32)) -> f32 {
    let span = (range.1 - range.0).max(1.0);
    [4.0, 8.0, 16.0, 25.0, 50.0, 100.0].into_iter().find(|s| span / s <= 24.0).unwrap_or(200.0)
}

/// A vector field as colours: its direction as the hue, its strength as
/// the saturation (linear light).
fn vector_colors(v: &Grid) -> Grid {
    let n = v.rect.area();
    let (x, z) = (v.plane(0), if v.planes > 1 { v.plane(1) } else { v.plane(0) });
    let max = x.iter().zip(z).map(|(a, b)| (a * a + b * b).sqrt()).fold(1e-6f32, f32::max);
    let mut out = vec![0.0; 3 * n];
    for i in 0..n {
        let a = z[i].atan2(x[i]);
        let s = ((x[i] * x[i] + z[i] * z[i]).sqrt() / max).clamp(0.0, 1.0);
        let h = (a / std::f32::consts::TAU).rem_euclid(1.0) * 6.0;
        let f = |k: f32| {
            let t = ((h + k) % 6.0 - 3.0).abs() - 1.0;
            t.clamp(0.0, 1.0)
        };
        let rgb = [f(0.0), f(4.0), f(2.0)];
        for q in 0..3 {
            let c = 0.5 + (rgb[q] - 0.5) * s;
            out[q * n + i] = (c * c).max(0.0);
        }
    }
    Grid { lod: v.lod, rect: v.rect, planes: 3, data: out }
}

/// Ids as their colours (linear light); an id with no colour gets one of
/// its own.
fn category_colors(ids: &Grid, table: &[[u8; 3]]) -> Grid {
    let n = ids.rect.area();
    let lin = |b: u8| {
        let v = b as f32 / 255.0;
        if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    let mut out = vec![0.0; 3 * n];
    for (i, v) in ids.plane(0).iter().enumerate() {
        let id = v.round().max(0.0) as usize;
        let c = table.get(id).copied().unwrap_or_else(|| {
            let h = (id as u32).wrapping_mul(2654435761);
            [(h >> 24) as u8 | 64, (h >> 16) as u8 | 64, (h >> 8) as u8 | 64]
        });
        for q in 0..3 {
            out[q * n + i] = lin(c[q]);
        }
    }
    Grid { lod: ids.lod, rect: ids.rect, planes: 3, data: out }
}

/// For callers with no map.
pub fn no_env() -> Arc<dyn Env> {
    Arc::new(NoEnv)
}
