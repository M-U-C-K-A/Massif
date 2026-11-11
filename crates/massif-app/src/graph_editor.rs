//! The terrain graph editor, in the manner of Gaea: a node graph, a 3D
//! viewport showing the selected node as a smooth, coloured and lit relief,
//! and the properties of the selected node. When the result is right, it is
//! converted to the map: heights, terrains, and the colours of the ground
//! turned into the closest real blocks of the game.
//!
//! - Right-click the graph, or press Tab or Space, to add a node (type to
//!   search). Drop a wire on empty space to add a node already wired.
//! - Drag from an output (right) to an input (left) to wire them; click a
//!   wired input to pick its wire up again.
//! - Drag a node by its title; Delete removes the selected node.
//! - Scroll or drag the background to move around, ⌘/Ctrl + scroll or pinch
//!   to zoom, F to frame everything.
//! - The viewport follows the selected node; pin it to keep one in view.
//!
//! Previews are evaluated on another thread with a cache, so changing a
//! setting only computes that node and what follows it.

use crate::blocks::BlockLibrary;
use crate::doc::Document;
use crate::terrain_view::{MAX_SLABS, RenderSettings, Slab, TerrainView, ViewData, occlusion};
use crate::theme::{self, Icon, Palette, RichTextExt};
use egui::{Color32, ColorImage, Key, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, TextureHandle, TextureOptions, Ui, Vec2, pos2, vec2};
use massif_core::graph::{
    AutoPalette, Bend, BlockPalette, CellStyle, ColorField, CombineOp, EvalCache, Evaluation, Extent, Family, Field, GradientPreset, Graph, GraphError,
    LayerOut, MountainStyle, NodeId, NodeKind, NoiseStyle, Outputs, PortType, Progress, ShapeKind, Template, TintMode, Value,
};
use massif_core::registry::{Registry, TerrainId};
use massif_core::{BlockRect, LevelKind, TILE_SIZE};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Instant;

/// Build resolutions offered, in samples per side.
pub const RESOLUTIONS: [usize; 7] = [512, 1024, 2048, 4096, 8192, 12288, 16384];
/// Preview resolutions offered.
pub const PREVIEWS: [usize; 6] = [256, 384, 512, 768, 1024, 2048];
/// Sizes of the images the Render panel saves.
pub const IMAGE_SIZES: [(&str, [u32; 2]); 5] = [
    ("HD", [1920, 1080]),
    ("QHD", [2560, 1440]),
    ("4K", [3840, 2160]),
    ("Square", [4096, 4096]),
    ("8K", [7680, 4320]),
];
const NODE_WIDTH: f32 = 168.0;
const HEADER: f32 = 26.0;
const ROW: f32 = 18.0;
const THUMB: f32 = 68.0;
/// Pixels of the thumbnails.
const THUMB_PX: usize = 96;

/// Colour of a family of nodes.
pub fn family_color(f: Family) -> Color32 {
    match f {
        Family::Landform => Color32::from_rgb(86, 196, 120),
        Family::Generator => Color32::from_rgb(52, 186, 176),
        Family::Modifier => Color32::from_rgb(86, 146, 255),
        Family::Erosion => Color32::from_rgb(245, 146, 64),
        Family::Combiner => Color32::from_rgb(168, 124, 255),
        Family::Selector => Color32::from_rgb(236, 192, 64),
        Family::Color => Color32::from_rgb(236, 96, 164),
        Family::Output => Color32::from_rgb(244, 84, 104),
    }
}

/// Colour of what flows along a wire.
fn port_color(t: PortType) -> Color32 {
    match t {
        PortType::Height => Color32::from_rgb(176, 188, 206),
        PortType::Mask => Color32::from_rgb(236, 192, 64),
        PortType::Color => Color32::from_rgb(236, 96, 164),
    }
}

/// The square of the world the graph covers: the whole map.
fn map_extent(doc: &Document, n: usize) -> Extent {
    let rect = match doc.dimension().bounds() {
        Some((lo, hi)) => BlockRect::new(lo.x * TILE_SIZE, lo.z * TILE_SIZE, (hi.x + 1) * TILE_SIZE, (hi.z + 1) * TILE_SIZE),
        None => BlockRect::new(-512, -512, 512, 512),
    };
    Extent::covering(rect, n)
}

// -------------------------------------------------------------- previews --

/// A small picture of a node's output: shaded relief for heights, grey for
/// masks, the colours themselves for colours.
fn thumbnail(value: &Value, kind: PortType, spacing: f32) -> ColorImage {
    let px = THUMB_PX;
    let mut pixels = Vec::with_capacity(px * px);
    match value {
        Value::Color(c) => {
            let k = c.n as f32 / px as f32;
            for j in 0..px {
                for i in 0..px {
                    let s = c.sample((i as f32 + 0.5) * k - 0.5, (j as f32 + 0.5) * k - 0.5);
                    let e = s.map(massif_core::graph::encode_srgb);
                    pixels.push(Color32::from_rgb(e[0], e[1], e[2]));
                }
            }
        }
        Value::Field(f) => {
            let k = f.n as f32 / px as f32;
            let (lo, hi) = f.range();
            let span = (hi - lo).max(1e-3);
            for j in 0..px {
                for i in 0..px {
                    let (u, v) = ((i as f32 + 0.5) * k - 0.5, (j as f32 + 0.5) * k - 0.5);
                    let h = f.sample(u, v);
                    if kind == PortType::Mask {
                        let g = (h.clamp(0.0, 1.0) * 255.0) as u8;
                        pixels.push(Color32::from_rgb(g, g, g));
                        continue;
                    }
                    let t = (h - lo) / span;
                    let dx = (f.sample(u + k, v) - f.sample(u - k, v)) / (2.0 * spacing * k);
                    let dz = (f.sample(u, v + k) - f.sample(u, v - k)) / (2.0 * spacing * k);
                    let shade = (1.0 + (dx + dz) * 1.4).clamp(0.35, 1.5);
                    let g = ((40.0 + 170.0 * t) * shade).clamp(0.0, 255.0);
                    pixels.push(Color32::from_rgb(g as u8, (g * 1.01).min(255.0) as u8, (g * 1.04).min(255.0) as u8));
                }
            }
        }
    }
    ColorImage::new([px, px], pixels)
}

/// What the viewport shows on the relief.
enum Paint {
    /// A neutral clay, to judge the forms.
    Clay,
    Mask(Arc<Field>),
    Color(Arc<ColorField>),
}

/// The closest heights before a node: the first height output met going
/// back through its inputs.
fn height_before(graph: &Graph, eval: &Evaluation, id: NodeId) -> Option<Arc<Field>> {
    let mut queue = std::collections::VecDeque::from([id]);
    let mut seen = std::collections::HashSet::new();
    while let Some(n) = queue.pop_front() {
        if !seen.insert(n) {
            continue;
        }
        let Some(node) = graph.node(n) else { continue };
        for k in 0..node.kind.inputs().len() {
            if let Some((src, out)) = graph.input_of(n, k as u8) {
                if graph.output_type(src, out) == PortType::Height
                    && let Some(f) = eval.field(src, out)
                {
                    return Some(f.clone());
                }
                queue.push_back(src);
            }
        }
    }
    None
}

/// What feeds the first output node of a kind.
fn source_of(graph: &Graph, pick: impl Fn(&NodeKind) -> bool) -> Option<(NodeId, u8)> {
    let node = graph.nodes.iter().find(|n| pick(&n.kind))?;
    graph.input_of(node.id, 0)
}

/// The relief and colours the viewport shows for `view` (a node's output),
/// or for the graph's outputs.
fn pick_view(graph: &Graph, eval: &Evaluation, view: Option<(NodeId, u8)>) -> Option<(Arc<Field>, Paint, (NodeId, u8))> {
    let final_height = source_of(graph, |k| matches!(k, NodeKind::HeightOutput));
    let final_color = source_of(graph, |k| matches!(k, NodeKind::ColorOutput { .. }));
    let colors_of = |src: Option<(NodeId, u8)>| src.and_then(|s| eval.values.get(&s)).map(|v| v.as_color());
    // Output nodes show what they receive.
    let view = view.map(|(id, out)| match graph.node(id).map(|n| &n.kind) {
        Some(k) if k.family() == Family::Output => graph.input_of(id, 0).unwrap_or((id, out)),
        _ => (id, out),
    });
    let (id, out) = view.or(final_height).or(final_color)?;
    let value = eval.values.get(&(id, out))?;
    let fallback = || final_height.and_then(|s| eval.field(s.0, s.1).cloned());
    match graph.output_type(id, out) {
        PortType::Height => {
            let heights = value.as_field();
            let paint = match colors_of(final_color) {
                Some(c) if Some((id, out)) == final_height => Paint::Color(c),
                _ => Paint::Clay,
            };
            Some((heights, paint, (id, out)))
        }
        PortType::Mask => {
            let heights = height_before(graph, eval, id).or_else(fallback)?;
            Some((heights, Paint::Mask(value.as_field()), (id, out)))
        }
        PortType::Color => {
            let heights = height_before(graph, eval, id).or_else(fallback)?;
            Some((heights, Paint::Color(value.as_color()), (id, out)))
        }
    }
}

/// Rock over the ground to show: a top, a bottom and maybe its colours.
type ShownLayer = (Arc<Field>, Arc<Field>, Option<Arc<ColorField>>);

/// Whether a node makes rock over the ground (a top and a bottom).
fn makes_layer(kind: &NodeKind) -> bool {
    matches!(kind, NodeKind::FloatingIslands { .. } | NodeKind::Overhang { .. })
}

/// The rock over the ground the viewport shows: the layer outputs' with the
/// graph's output, or the viewed node's own.
fn layers_shown(graph: &Graph, eval: &Evaluation, view: Option<(NodeId, u8)>) -> Vec<ShownLayer> {
    let field = |src: Option<(NodeId, u8)>| src.and_then(|(id, out)| eval.field(id, out).cloned());
    match view.and_then(|(id, _)| graph.node(id)) {
        Some(node) if makes_layer(&node.kind) => field(Some((node.id, 0))).zip(field(Some((node.id, 1)))).map(|(t, b)| (t, b, None)).into_iter().collect(),
        Some(_) => Vec::new(),
        None => {
            let mut outputs: Vec<_> = graph.nodes.iter().filter(|n| matches!(n.kind, NodeKind::LayerOutput { .. })).collect();
            outputs.sort_by_key(|n| match n.kind {
                NodeKind::LayerOutput { slot, .. } => slot,
                _ => 0,
            });
            outputs
                .into_iter()
                .filter_map(|n| {
                    let top = field(graph.input_of(n.id, 0))?;
                    let bottom = field(graph.input_of(n.id, 1))?;
                    let colors = graph.input_of(n.id, 2).and_then(|s| eval.values.get(&s)).map(|v| v.as_color());
                    Some((top, bottom, colors))
                })
                .collect()
        }
    }
}

/// Layers as the viewport takes them: two at most, the others laid into
/// the second where it has no rock.
fn slabs_of(layers: &[ShownLayer], n: usize) -> Vec<Slab> {
    let mut slabs: Vec<Slab> = Vec::new();
    for (k, (top, bottom, colors)) in layers.iter().enumerate() {
        if top.n != n || bottom.n != n {
            continue;
        }
        let shape: Vec<[f32; 2]> = (0..n * n)
            .map(|c| {
                let (t, b) = (top.data[c], bottom.data[c]);
                if t - b > 0.5 { [t, b] } else { [t, t] }
            })
            .collect();
        let paint: Vec<[u8; 4]> = (0..n * n)
            .map(|c| {
                let rgb = colors.as_ref().and_then(|col| col.data.get(c).copied()).unwrap_or([150, 143, 132]);
                [rgb[0], rgb[1], rgb[2], 255]
            })
            .collect();
        if k < MAX_SLABS {
            slabs.push(Slab {
                top_bottom: shape,
                colors: paint,
            });
        } else if let Some(last) = slabs.last_mut() {
            for c in 0..n * n {
                let [t, b] = last.top_bottom[c];
                if t - b <= 0.5 && shape[c][0] - shape[c][1] > 0.5 {
                    last.top_bottom[c] = shape[c];
                    last.colors[c] = paint[c];
                }
            }
        }
    }
    slabs
}

fn view_data(heights: &Field, paint: &Paint, extent: &Extent, exaggeration: f32, layers: &[ShownLayer]) -> ViewData {
    let n = heights.n;
    let (lo, hi) = heights.range();
    let span = (hi - lo).max(1e-3);
    let mix = |a: [f32; 3], b: [f32; 3], t: f32| [0, 1, 2].map(|k| (a[k] + (b[k] - a[k]) * t.clamp(0.0, 1.0)) as u8);
    let colors: Vec<[u8; 4]> = (0..n * n)
        .map(|c| {
            let rgb = match paint {
                Paint::Clay => mix([150.0, 143.0, 132.0], [204.0, 198.0, 188.0], (heights.data[c] - lo) / span),
                Paint::Mask(m) => mix([44.0, 46.0, 54.0], [244.0, 242.0, 236.0], m.data.get(c).copied().unwrap_or(0.0)),
                Paint::Color(col) => col.data.get(c).copied().unwrap_or([128; 3]),
            };
            [rgb[0], rgb[1], rgb[2], 255]
        })
        .collect();
    ViewData {
        n,
        heights: heights.data.clone(),
        colors,
        light: occlusion(&heights.data, n, extent.spacing(), exaggeration),
        size: extent.size as f32,
        exaggeration,
        slabs: slabs_of(layers, n),
    }
}

struct Request {
    revision: u64,
    graph: Graph,
    extent: Extent,
    view: Option<(NodeId, u8)>,
    exaggeration: f32,
    cancel: Arc<AtomicBool>,
}

struct Preview {
    revision: u64,
    thumbs: Vec<((NodeId, u8), ColorImage)>,
    failed: HashMap<NodeId, String>,
    view: Option<(Arc<ViewData>, (NodeId, u8))>,
    error: Option<String>,
    ms: f32,
}

/// Evaluates previews on its own thread, keeping a cache between them.
struct Worker {
    tx: Sender<Request>,
    rx: Receiver<Preview>,
    cancel: Option<Arc<AtomicBool>>,
}

impl Worker {
    fn new(ctx: egui::Context) -> Self {
        let (tx, requests) = channel::<Request>();
        let (answers, rx) = channel();
        std::thread::Builder::new()
            .name("massif graph previews".into())
            .spawn(move || {
                let mut cache = EvalCache::default();
                // What each thumbnail was drawn from.
                let mut drawn: HashMap<(NodeId, u8), usize> = HashMap::new();
                while let Ok(mut req) = requests.recv() {
                    while let Ok(newer) = requests.try_recv() {
                        req = newer;
                    }
                    let started = Instant::now();
                    let ids: Vec<NodeId> = req.graph.nodes.iter().map(|n| n.id).collect();
                    let eval = match req.graph.evaluate_cached(&req.extent, &ids, &mut cache, &req.cancel) {
                        Ok(e) => e,
                        Err(GraphError::Cancelled) => continue,
                        Err(e) => {
                            let _ = answers.send(Preview {
                                revision: req.revision,
                                thumbs: Vec::new(),
                                failed: HashMap::new(),
                                view: None,
                                error: Some(e.to_string()),
                                ms: 0.0,
                            });
                            ctx.request_repaint();
                            continue;
                        }
                    };
                    let spacing = req.extent.spacing();
                    let mut thumbs = Vec::new();
                    drawn.retain(|k, _| eval.values.contains_key(k));
                    for (key, value) in &eval.values {
                        let ptr = match value {
                            Value::Field(f) => Arc::as_ptr(f) as usize,
                            Value::Color(c) => Arc::as_ptr(c) as usize,
                        };
                        if drawn.get(key) != Some(&ptr) {
                            thumbs.push((*key, thumbnail(value, req.graph.output_type(key.0, key.1), spacing)));
                            drawn.insert(*key, ptr);
                        }
                    }
                    // A node that makes rock over the ground is shown over
                    // the graph's ground.
                    let viewed = |pick: &dyn Fn(&NodeKind) -> bool| req.view.filter(|(id, _)| req.graph.node(*id).is_some_and(|n| pick(&n.kind)));
                    let layer_node = viewed(&makes_layer);
                    // A layer output shows the whole of the graph's output.
                    let layer_output = viewed(&|k| matches!(k, NodeKind::LayerOutput { .. }));
                    let layers = layers_shown(&req.graph, &eval, if layer_output.is_some() { None } else { req.view });
                    let relief_of = if layer_node.is_some() || layer_output.is_some() { None } else { req.view };
                    let view = pick_view(&req.graph, &eval, relief_of).map(|(h, paint, shown)| {
                        let data = view_data(&h, &paint, &req.extent, req.exaggeration, &layers);
                        (Arc::new(data), layer_node.or(layer_output).unwrap_or(shown))
                    });
                    let failed = eval.failed.iter().map(|(k, e)| (*k, e.to_string())).collect();
                    let _ = answers.send(Preview {
                        revision: req.revision,
                        thumbs,
                        failed,
                        view,
                        error: None,
                        ms: started.elapsed().as_secs_f32() * 1000.0,
                    });
                    ctx.request_repaint();
                }
            })
            .expect("a thread for previews");
        Self { tx, rx, cancel: None }
    }

    fn request(&mut self, req: Request) {
        if let Some(c) = self.cancel.take() {
            c.store(true, Ordering::Relaxed);
        }
        self.cancel = Some(req.cancel.clone());
        let _ = self.tx.send(req);
    }
}

/// A build on another thread.
struct Building {
    rx: Receiver<Result<Built, GraphError>>,
    /// Stops it.
    cancel: Arc<AtomicBool>,
    /// How far it got.
    progress: Arc<Progress>,
    started: Instant,
    /// Samples per side.
    n: usize,
}

/// A finished build: what to write to the map.
struct Built {
    extent: Extent,
    outputs: Outputs,
}

/// The node library opened over the graph.
struct Library {
    /// Where it opened, in graph coordinates, and on screen.
    at: [f32; 2],
    screen: Pos2,
    query: String,
    /// A wire dropped on empty space: the new node gets it.
    from: Option<(NodeId, u8)>,
    focus: bool,
}

// ------------------------------------------------------------- templates --

/// How a template's landscape is best seen: its sun, sky, clouds and sea.
fn template_render(t: Template, sea: f32, exaggeration: f32) -> RenderSettings {
    let d = RenderSettings {
        exaggeration,
        ..RenderSettings::default()
    };
    let clouds = |cover: f32, base: f32, thickness: f32, size: f32| RenderSettings {
        clouds: true,
        cloud_cover: cover,
        cloud_base: sea + base,
        cloud_thickness: thickness,
        cloud_size: size,
        ..d
    };
    match t {
        Template::Alpine => clouds(0.38, 170.0, 80.0, 300.0),
        Template::Archipelago => RenderSettings {
            sea: true,
            sun_azimuth: 210.0,
            sun_elevation: 52.0,
            haze: 0.35,
            ..clouds(0.32, 110.0, 50.0, 200.0)
        },
        Template::KarstTowers => RenderSettings {
            sea: true,
            cloud_density: 1.3,
            sun_azimuth: 115.0,
            sun_elevation: 32.0,
            haze: 0.7,
            ..clouds(0.5, 55.0, 45.0, 150.0)
        },
        Template::FloatingMountains => RenderSettings {
            sea: true,
            cloud_density: 1.25,
            sun_azimuth: 120.0,
            sun_elevation: 30.0,
            haze: 0.6,
            ..clouds(0.5, 70.0, 45.0, 160.0)
        },
        Template::RiverValley => RenderSettings {
            sea: true,
            sun_azimuth: 140.0,
            sun_elevation: 26.0,
            haze: 0.4,
            ..clouds(0.36, 190.0, 70.0, 280.0)
        },
        Template::DesertCanyon => RenderSettings {
            sea: true,
            sun_azimuth: 250.0,
            sun_elevation: 9.0,
            haze: 0.4,
            ..d
        },
        Template::VolcanicIsland => RenderSettings {
            sea: true,
            sun_azimuth: 300.0,
            sun_elevation: 30.0,
            haze: 0.45,
            ..clouds(0.42, 150.0, 70.0, 240.0)
        },
        Template::DuneSea => RenderSettings {
            sun_azimuth: 100.0,
            sun_elevation: 12.0,
            haze: 0.5,
            ..d
        },
        Template::RollingHills => RenderSettings {
            sea: true,
            sun_azimuth: 200.0,
            sun_elevation: 45.0,
            ..clouds(0.45, 130.0, 70.0, 260.0)
        },
    }
}

/// A small picture of what a template makes: its colours shaded by its
/// relief, the sea over what lies under it.
fn template_picture(t: Template, reg: &Registry, sea: f32, extent: &Extent) -> Option<ColorImage> {
    let graph = t.graph(reg, sea, extent.size as f32);
    let out = graph.build(extent, &AtomicBool::new(false)).ok()?;
    let h = out.heights?;
    let colors = out.colors.map(|(c, _)| c);
    let n = h.n;
    let spacing = extent.spacing();
    let light = {
        let l = [-0.6f32, 0.75, -0.45];
        let k = (l[0] * l[0] + l[1] * l[1] + l[2] * l[2]).sqrt();
        l.map(|v| v / k)
    };
    let water = [0.03f32, 0.16, 0.28].map(|v| v * 0.8);
    // A surface lit from the north-west.
    let shade = |f: &Field, i: usize, j: usize| {
        let dx = (f.at(i + 1, j) - f.at(i.saturating_sub(1), j)) / (2.0 * spacing);
        let dz = (f.at(i, j + 1) - f.at(i, j.saturating_sub(1))) / (2.0 * spacing);
        let k = (dx * dx + 1.0 + dz * dz).sqrt();
        0.25 + 0.95 * ((-dx * light[0] + light[1] - dz * light[2]) / k).max(0.0)
    };
    let mut picture: Vec<[f32; 3]> = Vec::with_capacity(n * n);
    for j in 0..n {
        for i in 0..n {
            let v = h.at(i, j);
            let base = colors.as_ref().map_or([0.4, 0.4, 0.4], |c| c.linear(j * n + i));
            let mut c = base.map(|b| b * shade(&h, i, j));
            if v < sea {
                let deep = 0.55 + 0.4 * ((sea - v) / 24.0).min(1.0);
                c = [0, 1, 2].map(|k| c[k] + (water[k] - c[k]) * deep);
            }
            picture.push(c);
        }
    }
    // Rock over the ground, seen from above, and its shadow under it.
    for layer in &out.layers {
        let there = |i: usize, j: usize| layer.top.at(i, j) - layer.bottom.at(i, j) >= 1.0;
        for j in 0..n {
            for i in 0..n {
                if i >= 2 && j >= 2 && there(i - 2, j - 2) && !there(i, j) {
                    picture[j * n + i] = picture[j * n + i].map(|v| v * 0.55);
                }
            }
        }
        for j in 0..n {
            for i in 0..n {
                if there(i, j) {
                    let base = layer.colors.as_ref().map_or([0.3, 0.42, 0.2], |(c, _)| c.linear(j * n + i));
                    picture[j * n + i] = base.map(|b| b * shade(&layer.top, i, j) * 1.1);
                }
            }
        }
    }
    let rgb: Vec<u8> = picture.into_iter().flat_map(|c| c.map(massif_core::graph::encode_srgb)).collect();
    Some(ColorImage::from_rgb([n, n], &rgb))
}

/// The pictures of the templates, made on another thread for the map they
/// show.
#[derive(Default)]
struct TemplatePictures {
    /// The world (its generation), sea and size they were made for.
    made_for: Option<(u64, i32, u64)>,
    rx: Option<Receiver<(Template, ColorImage)>>,
    textures: HashMap<Template, TextureHandle>,
}

/// A template in the picker: its picture, name and what it is.
fn template_card(ui: &mut Ui, t: Template, picture: Option<&TextureHandle>, pal: &Palette) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(300.0, 78.0), Sense::click());
    let painter = ui.painter_at(rect);
    let fill = if response.hovered() { pal.accent_soft } else { pal.panel };
    painter.rect(rect, 9.0, fill, Stroke::new(1.0, pal.border), StrokeKind::Inside);
    let pic = Rect::from_min_size(rect.min + vec2(7.0, 7.0), vec2(64.0, 64.0));
    match picture {
        Some(tex) => {
            egui::Image::new(tex).corner_radius(6).paint_at(ui, pic);
        }
        None => {
            painter.rect_filled(pic, 6.0, pal.border);
        }
    }
    let text = Rect::from_min_max(pos2(pic.right() + 10.0, rect.top() + 9.0), rect.max - vec2(8.0, 6.0));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(text).layout(egui::Layout::top_down(egui::Align::Min)));
    child.spacing_mut().item_spacing.y = 2.0;
    child.add(egui::Label::new(RichText::new(t.label()).size(13.5).color(pal.text).medium()).selectable(false));
    child.add(egui::Label::new(RichText::new(t.description()).size(11.5).color(pal.weak)).selectable(false));
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

// ----------------------------------------------------------------- editor --

pub struct GraphEditor {
    /// The world the graph belongs to (its generation).
    generation: Option<u64>,
    pub selected: Option<NodeId>,
    /// The node the viewport shows (V); `None`: the graph's output.
    pinned: Option<NodeId>,
    /// Which output of the viewed node is shown.
    view_output: u8,
    pan: Vec2,
    zoom: f32,
    /// A wire being drawn from an output.
    wiring: Option<(NodeId, u8)>,
    textures: HashMap<(NodeId, u8), TextureHandle>,
    failed: HashMap<NodeId, String>,
    /// Revision of the graph, of the last preview asked for and received.
    revision: u64,
    asked: u64,
    received: u64,
    worker: Option<Worker>,
    /// What the viewport shows, as the worker last said.
    shown: Option<(NodeId, u8)>,
    pub resolution: usize,
    pub preview: usize,
    build: Option<Building>,
    error: Option<String>,
    preview_ms: f32,
    /// Frame the whole graph at the next opportunity.
    fit: bool,
    library: Option<Library>,
    pub view: TerrainView,
    /// The render settings are open over the viewport.
    render_open: bool,
    /// Size of the images saved.
    pub image_size: [u32; 2],
    /// What saving an image gave, to show: the file's name, or what failed.
    saved: Option<Result<String, String>>,
    /// The last canvas rectangle, for keyboard actions.
    canvas: Rect,
    /// The template picker is open, under this point.
    templates_open: bool,
    templates_at: Pos2,
    templates_up: bool,
    pictures: TemplatePictures,
    /// The graph a template replaced, to go back to.
    replaced: Option<Graph>,
}

impl Default for GraphEditor {
    fn default() -> Self {
        Self {
            generation: None,
            selected: None,
            pinned: None,
            view_output: 0,
            pan: vec2(20.0, 20.0),
            zoom: 1.0,
            wiring: None,
            textures: HashMap::new(),
            failed: HashMap::new(),
            revision: 1,
            asked: 0,
            received: 0,
            worker: None,
            shown: None,
            resolution: 1024,
            preview: 384,
            build: None,
            error: None,
            preview_ms: 0.0,
            fit: true,
            library: None,
            view: TerrainView::default(),
            render_open: false,
            image_size: [3840, 2160],
            saved: None,
            canvas: Rect::NOTHING,
            templates_open: false,
            templates_at: Pos2::ZERO,
            templates_up: false,
            pictures: TemplatePictures::default(),
            replaced: None,
        }
    }
}

/// The graph of the world, or the starter one.
fn graph_of(doc: &Document) -> Graph {
    match &doc.world.meta.graph {
        Some(g) => g.clone(),
        None => {
            let reg = &doc.world.registry;
            let get = |name: &str| reg.terrain_by_name(name).unwrap_or(TerrainId::ROCK).0;
            Graph::starter(get("Grass"), TerrainId::ROCK.0, get("Snow"), get("Gravel"))
        }
    }
}

impl GraphEditor {
    /// Is the preview up to date with the graph?
    #[cfg(test)]
    pub fn is_current(&self) -> bool {
        self.received == self.revision && self.asked == self.revision && self.view.has_data()
    }

    /// The world's graph was replaced from outside the editor.
    #[cfg(test)]
    pub fn graph_replaced(&mut self) {
        self.selected = None;
        self.pinned = None;
        self.revision += 1;
        self.fit = true;
    }

    /// Select a node and view it, as V does (or go back to the output).
    #[cfg(test)]
    pub fn view(&mut self, id: Option<NodeId>) {
        self.selected = id;
        match id {
            Some(id) if self.pinned != Some(id) => self.toggle_view(id),
            Some(_) => {}
            None => self.view_key(),
        }
    }

    fn store(&mut self, doc: &mut Document, graph: Graph) {
        doc.world.meta.graph = Some(graph);
        doc.mark_changed();
        self.revision += 1;
    }

    /// View a node in the viewport, or go back to the graph's output if it
    /// is the one viewed.
    fn toggle_view(&mut self, id: NodeId) {
        if self.pinned == Some(id) {
            self.pinned = None;
        } else {
            self.pinned = Some(id);
            self.view_output = 0;
        }
        self.revision += 1;
    }

    /// V: view the selected node, or go back to the output.
    fn view_key(&mut self) {
        match self.selected {
            Some(id) => self.toggle_view(id),
            None if self.pinned.is_some() => {
                self.pinned = None;
                self.revision += 1;
            }
            None => {}
        }
    }

    /// The node the viewport shows (V), and which of its outputs.
    fn viewed(&self, graph: &Graph) -> Option<(NodeId, u8)> {
        let id = self.pinned.filter(|id| graph.node(*id).is_some())?;
        let outs = graph.node(id)?.kind.outputs().len() as u8;
        Some((id, if outs == 0 { 0 } else { self.view_output.min(outs - 1) }))
    }

    /// Keep up with the world and the previews: called by every part of
    /// the editor each frame, the first call does the work.
    fn sync(&mut self, ctx: &egui::Context, doc: &Document) {
        if self.generation != Some(doc.generation) {
            self.generation = Some(doc.generation);
            self.textures.clear();
            self.selected = None;
            self.pinned = None;
            self.revision += 1;
            self.fit = true;
            self.view.camera = Default::default();
            // Convert at a block per sample, or as close as offered.
            let side = map_extent(doc, 2).size;
            self.resolution = RESOLUTIONS.iter().copied().find(|&r| r as f64 >= side).unwrap_or(RESOLUTIONS[RESOLUTIONS.len() - 1]);
        }
        let worker = self.worker.get_or_insert_with(|| Worker::new(ctx.clone()));
        while let Ok(p) = worker.rx.try_recv() {
            self.received = self.received.max(p.revision);
            self.error = p.error;
            self.failed = p.failed;
            self.preview_ms = p.ms;
            for (key, image) in p.thumbs {
                match self.textures.get_mut(&key) {
                    Some(t) => t.set(image, TextureOptions::LINEAR),
                    None => {
                        let t = ctx.load_texture(format!("graph {key:?}"), image, TextureOptions::LINEAR);
                        self.textures.insert(key, t);
                    }
                }
            }
            if let Some((data, shown)) = p.view {
                self.view.set(data);
                self.shown = Some(shown);
            }
        }
        if self.asked != self.revision {
            let graph = graph_of(doc);
            self.textures.retain(|k, _| graph.node(k.0).is_some());
            let req = Request {
                revision: self.revision,
                view: self.viewed(&graph),
                graph,
                extent: map_extent(doc, self.preview),
                exaggeration: self.view.render.exaggeration,
                cancel: Arc::new(AtomicBool::new(false)),
            };
            if let Some(w) = &mut self.worker {
                w.request(req);
            }
            self.asked = self.revision;
        }
        self.view.sea_level = doc.world.meta.sea_level as f32;
    }

    // ------------------------------------------------------------ viewport --

    /// The 3D viewport of the graph.
    pub fn viewport_ui(&mut self, ui: &mut Ui, frame: &eframe::Frame, doc: &Document) {
        self.sync(ui.ctx(), doc);
        let pal = Palette::of(ui);
        let rect = ui.available_rect_before_wrap();
        if self.view.ui(ui, frame) {
            // The light depends on the exaggeration.
            self.revision += 1;
        }
        let graph = graph_of(doc);
        // What is shown, top left.
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink(10.0)).layout(egui::Layout::top_down(egui::Align::Min)));
        if ui.rect_contains_pointer(rect) && !ui.ctx().egui_wants_keyboard_input() && ui.input(|i| i.key_pressed(Key::V)) {
            self.view_key();
        }
        let label = match self.shown.filter(|_| self.pinned.is_some()).and_then(|(id, out)| graph.node(id).map(|n| (n, out))) {
            Some((n, out)) => {
                let outs = n.kind.outputs();
                match outs.get(out as usize) {
                    Some((name, _)) if outs.len() > 1 => format!("{} · {}", n.kind.label(), name),
                    _ => n.kind.label().to_owned(),
                }
            }
            None => "The graph's output".to_owned(),
        };
        egui::Frame::new()
            .fill(Color32::from_black_alpha(120))
            .corner_radius(6)
            .inner_margin(egui::Margin::symmetric(8, 4))
            .show(&mut child, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(theme::glyph(Icon::Eye).to_string()).color(Color32::WHITE));
                    ui.label(RichText::new(label).color(Color32::WHITE).medium());
                    let hint = if self.pinned.is_some() { "V: back to the output" } else { "Select a node and press V to view it" };
                    ui.label(RichText::new(hint).size(11.0).color(Color32::from_white_alpha(150)));
                    if self.received != self.revision || self.build.is_some() {
                        ui.spinner();
                    }
                });
            });
        // Controls, bottom left.
        let bar = Rect::from_min_max(pos2(rect.left() + 10.0, rect.bottom() - 40.0), pos2(rect.right() - 10.0, rect.bottom() - 8.0));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(bar).layout(egui::Layout::left_to_right(egui::Align::Center)));
        egui::Frame::new()
            .fill(pal.panel.gamma_multiply(0.92))
            .stroke(Stroke::new(1.0, pal.border))
            .corner_radius(8)
            .inner_margin(egui::Margin::symmetric(8, 3))
            .show(&mut child, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().interact_size.y = 18.0;
                    ui.toggle_value(&mut self.render_open, theme::with_icon(Icon::Sun, "Render"))
                        .on_hover_text("Sun, clouds, haze, sea and exposure of the view");
                    ui.toggle_value(&mut self.view.render.clouds, theme::with_icon(Icon::Cloud, "Clouds"));
                    ui.toggle_value(&mut self.view.render.sea, theme::with_icon(Icon::Waves, "Sea"))
                        .on_hover_text(format!("Water at the world's sea level ({})", doc.world.meta.sea_level));
                    if ui.small_button(theme::glyph(Icon::Focus).to_string()).on_hover_text("Frame the relief").clicked() {
                        self.view.camera = Default::default();
                    }
                    let [w, h] = self.image_size;
                    if ui
                        .small_button(theme::glyph(Icon::Camera).to_string())
                        .on_hover_text(format!("Save the view as an image, at full quality ({w} × {h})"))
                        .clicked()
                    {
                        self.save_image(doc);
                    }
                    match &self.saved {
                        Some(Ok(name)) => {
                            ui.label(RichText::new(format!("Saved {name}")).size(11.5).color(pal.weak));
                        }
                        Some(Err(e)) => {
                            ui.label(RichText::new(e).size(11.5).color(pal.danger));
                        }
                        None => {}
                    }
                });
            });
        if self.render_open {
            let exaggeration = self.view.render.exaggeration;
            egui::Area::new(ui.scope_id().with("render settings"))
                .order(egui::Order::Foreground)
                .fixed_pos(pos2(rect.left() + 10.0, rect.top() + 46.0))
                .show(ui.ctx(), |ui| {
                    egui::Frame::popup(ui.style()).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
                        ui.set_width(300.0);
                        ui.spacing_mut().slider_width = 150.0;
                        egui::ScrollArea::vertical().max_height((rect.height() - 70.0).max(160.0)).show(ui, |ui| {
                            self.view.settings_ui(ui);
                        });
                    });
                });
            if (self.view.render.exaggeration - exaggeration).abs() > 1e-4 {
                self.revision += 1;
            }
        }
        if !self.view.has_data() {
            let text = match &self.error {
                Some(e) => e.clone(),
                None => "Building the preview…".to_owned(),
            };
            ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, text, egui::FontId::proportional(13.0), pal.weak);
        }
    }

    /// Render the view at full quality and save it where the user says.
    fn save_image(&mut self, doc: &Document) {
        let name = format!("{}.png", doc.title());
        let Some(path) = rfd::FileDialog::new().set_file_name(name).add_filter("PNG image", &["png"]).save_file() else {
            return;
        };
        let Some((w, h, rgba)) = self.view.render_image(self.image_size) else {
            self.saved = Some(Err("Nothing to render yet".to_owned()));
            return;
        };
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        self.saved = Some(match image::save_buffer(&path, &rgba, w, h, image::ColorType::Rgba8) {
            Ok(()) => Ok(name),
            Err(e) => Err(format!("The image could not be saved: {e}")),
        });
    }

    /// The Render panel: the sun, the sky, the clouds and the sea of the
    /// viewport, and saving it as an image.
    pub fn render_ui(&mut self, ui: &mut Ui, doc: &Document) {
        self.sync(ui.ctx(), doc);
        let pal = Palette::of(ui);
        ui.spacing_mut().slider_width = (ui.available_width() - 150.0).clamp(80.0, 240.0);
        let exaggeration = self.view.render.exaggeration;
        self.view.settings_ui(ui);
        if (self.view.render.exaggeration - exaggeration).abs() > 1e-4 {
            // The light depends on the exaggeration.
            self.revision += 1;
        }
        ui.add_space(6.0);
        ui.label(RichText::new(theme::with_icon(Icon::Camera, "Image")).size(11.5).color(pal.weak));
        ui.horizontal(|ui| {
            ui.add_sized([86.0, 18.0], egui::Label::new(RichText::new("Size").size(12.0)));
            let [w, h] = self.image_size;
            egui::ComboBox::from_id_salt("render image size")
                .selected_text(format!("{w} × {h}"))
                .width(130.0)
                .show_ui(ui, |ui| {
                    for (name, size) in IMAGE_SIZES {
                        ui.selectable_value(&mut self.image_size, size, format!("{name} · {} × {}", size[0], size[1]));
                    }
                });
        });
        ui.add_space(4.0);
        let detail = self.preview;
        ui.label(
            RichText::new(format!(
                "Rendered from the preview, {detail} × {detail} samples: a finer preview (graph toolbar) gives a sharper image."
            ))
            .size(11.0)
            .color(pal.weak),
        );
        ui.add_space(4.0);
        if ui
            .add_enabled(self.view.has_data(), theme::primary_button(ui, theme::with_icon(Icon::Camera, "Save image…")))
            .on_hover_text("Render the view at full quality and save it as a PNG")
            .clicked()
        {
            self.save_image(doc);
        }
        match &self.saved {
            Some(Ok(name)) => {
                ui.label(RichText::new(format!("Saved {name}")).size(11.5).color(pal.weak));
            }
            Some(Err(e)) => {
                ui.colored_label(pal.danger, e);
            }
            None => {}
        }
    }

    // --------------------------------------------------------------- graph --

    /// The node graph, with its toolbar. Returns a message for the status
    /// bar.
    pub fn canvas_ui(&mut self, ui: &mut Ui, doc: &mut Document, blocks: &BlockLibrary) -> Option<String> {
        self.sync(ui.ctx(), doc);
        let mut feedback = None;
        let pal = Palette::of(ui);
        let mut graph = graph_of(doc);
        let before = graph.clone();

        // A build finished: write it to the map.
        if let Some(b) = &self.build
            && let Ok(result) = b.rx.try_recv()
        {
            let took = b.started.elapsed().as_secs_f32();
            self.build = None;
            match result {
                Ok(built) => {
                    let palette = match &built.outputs.colors {
                        Some((colors, set)) => block_palette(doc, blocks, *set, colors),
                        None => Vec::new(),
                    };
                    let layer_palettes: Vec<_> = built
                        .outputs
                        .layers
                        .iter()
                        .map(|l| match &l.colors {
                            Some((colors, set)) => block_palette(doc, blocks, *set, colors),
                            None => Vec::new(),
                        })
                        .collect();
                    doc.history.begin("Terrain graph", doc.dim);
                    let levels = layer_levels(doc, &built.outputs.layers);
                    let seed = doc.world.meta.seed as u64;
                    massif_core::graph::apply(&mut doc.history.edit(&mut doc.world, 0), &built.extent, &built.outputs, seed, &palette);
                    let grass = doc.world.registry.terrain_by_name("Grass").unwrap_or(TerrainId::ROCK);
                    for ((layer, index), layer_palette) in built.outputs.layers.iter().zip(&levels).zip(&layer_palettes) {
                        let edit = &mut doc.history.edit(&mut doc.world, *index);
                        massif_core::graph::apply_layer(edit, &built.extent, layer, seed, layer_palette, grass);
                    }
                    doc.commit();
                    doc.clamp_active_level();
                    let blocks_note = if palette.is_empty() { String::new() } else { format!(", in {} kinds of blocks", palette.len()) };
                    let layers_note = match built.outputs.layers.len() {
                        0 => String::new(),
                        1 => ", with one level of rock over the void".to_owned(),
                        k => format!(", with {k} levels of rock over the void"),
                    };
                    feedback = Some(format!(
                        "Converted the graph to voxels at {} × {} in {took:.1} s{blocks_note}{layers_note}",
                        built.extent.n, built.extent.n
                    ));
                }
                Err(GraphError::Cancelled) => feedback = Some("Conversion cancelled".to_owned()),
                Err(e) => feedback = Some(format!("The graph cannot be built: {e}")),
            }
        }

        // ---- toolbar ---------------------------------------------------------
        let mut toggled = false;
        ui.horizontal(|ui| {
            ui.add_space(6.0);
            if let Some(b) = &self.build {
                ui.spinner();
                let (done, total) = (b.progress.done.load(Ordering::Relaxed), b.progress.total.load(Ordering::Relaxed));
                let node = graph.node(b.progress.node.load(Ordering::Relaxed)).map(|n| n.kind.label()).unwrap_or("");
                let fraction = if total == 0 { 0.0 } else { done as f32 / total as f32 };
                ui.add(
                    egui::ProgressBar::new(fraction)
                        .desired_width(120.0)
                        .desired_height(14.0)
                        .text(RichText::new(format!("{done} / {total}")).size(10.5)),
                );
                ui.label(format!("{} × {}: {node}… {:.0} s", b.n, b.n, b.started.elapsed().as_secs_f32()));
                if ui.button("Cancel").clicked() {
                    b.cancel.store(true, Ordering::Relaxed);
                }
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
            } else if ui
                .add(theme::primary_button(ui, format!("{}  Convert to voxels", theme::glyph(Icon::Boxes))))
                .on_hover_text("Build the graph over the whole map at the chosen resolution, and write it to the ground: heights, terrains, and the colours as the closest real blocks (undoable)")
                .clicked()
            {
                let (tx, rx) = channel();
                let cancel = Arc::new(AtomicBool::new(false));
                let progress = Arc::new(Progress::default());
                let (flag, told) = (cancel.clone(), progress.clone());
                let extent = map_extent(doc, self.resolution);
                let g = graph.clone();
                let ctx = ui.ctx().clone();
                std::thread::spawn(move || {
                    let result = g.build_with_progress(&extent, &flag, &told).map(|outputs| Built { extent, outputs });
                    let _ = tx.send(result);
                    ctx.request_repaint();
                });
                self.build = Some(Building {
                    rx,
                    cancel,
                    progress,
                    started: Instant::now(),
                    n: self.resolution,
                });
            }
            ui.label(RichText::new("at").color(pal.weak));
            egui::ComboBox::from_id_salt("graph resolution")
                .selected_text(format!("{0} × {0}", self.resolution))
                .width(112.0)
                .show_ui(ui, |ui| {
                    for r in RESOLUTIONS {
                        let r_ = ui.selectable_value(&mut self.resolution, r, format!("{r} × {r}"));
                        if r >= 8192 {
                            r_.on_hover_text("A large build: it takes a while and several GB of memory");
                        }
                    }
                });
            let blocks_per = map_extent(doc, self.resolution).spacing();
            ui.label(RichText::new(format!("{blocks_per:.1} blocks per sample")).size(11.5).color(pal.weak));
            ui.separator();
            ui.label(RichText::new("Preview").color(pal.weak));
            let before_preview = self.preview;
            egui::ComboBox::from_id_salt("graph preview")
                .selected_text(format!("{0}", self.preview))
                .width(64.0)
                .show_ui(ui, |ui| {
                    for r in PREVIEWS {
                        ui.selectable_value(&mut self.preview, r, format!("{r} × {r}"));
                    }
                });
            if self.preview != before_preview {
                self.revision += 1;
            }
            if self.received != self.revision {
                ui.spinner();
            } else if self.preview_ms > 0.0 {
                ui.label(RichText::new(format!("{:.0} ms", self.preview_ms)).size(11.5).color(pal.weak));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let button = ui
                    .add(egui::Button::new(theme::with_icon(Icon::LayoutTemplate, "Templates")).selected(self.templates_open))
                    .on_hover_text("Start from a ready-made landscape: an archipelago, karst towers, a river valley, a canyon…");
                if button.clicked() {
                    self.templates_open = !self.templates_open;
                    toggled = true;
                }
                // Under the button, or over it when the window has no room
                // below.
                let room = ui.ctx().content_rect().bottom() - button.rect.bottom();
                self.templates_up = room < 560.0;
                self.templates_at = if self.templates_up {
                    button.rect.right_top() - vec2(0.0, 6.0)
                } else {
                    button.rect.right_bottom() + vec2(0.0, 6.0)
                };
                if ui.small_button(theme::with_icon(Icon::Maximize, "Fit")).on_hover_text("Show the whole graph (F)").clicked() {
                    self.fit = true;
                }
                if ui
                    .small_button(theme::with_icon(Icon::Plus, "Add"))
                    .on_hover_text("Add a node (Tab)")
                    .clicked()
                {
                    let c = self.canvas.center();
                    self.open_library(c, None);
                }
                if let Some(e) = &self.error {
                    ui.colored_label(pal.danger, e);
                }
            });
        });
        ui.separator();

        if self.templates_open {
            self.templates_ui(ui, doc, &mut graph, toggled, &mut feedback);
        }

        let canvas = ui.available_rect_before_wrap();
        self.canvas = canvas;
        self.canvas(ui, canvas, &mut graph, &doc.world.registry);
        ui.allocate_rect(canvas, Sense::hover());
        if graph != before {
            self.store(doc, graph);
        }
        feedback
    }

    /// The template picker, over the graph.
    fn templates_ui(&mut self, ui: &mut Ui, doc: &Document, graph: &mut Graph, toggled: bool, feedback: &mut Option<String>) {
        self.make_pictures(ui.ctx(), doc);
        let pal = Palette::of(ui);
        let mut chosen = None;
        let mut back = false;
        let area = egui::Area::new(ui.scope_id().with("templates"))
            .order(egui::Order::Foreground)
            .pivot(if self.templates_up { egui::Align2::RIGHT_BOTTOM } else { egui::Align2::RIGHT_TOP })
            .fixed_pos(self.templates_at)
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).inner_margin(egui::Margin::same(12)).show(ui, |ui| {
                    ui.label(RichText::new("Start from a landscape").size(14.0).medium());
                    ui.label(
                        RichText::new("Replaces the graph, and sets the sun, clouds and sea of the view to suit it.")
                            .size(11.5)
                            .color(pal.weak),
                    );
                    ui.add_space(6.0);
                    egui::Grid::new("template cards").spacing([8.0, 8.0]).show(ui, |ui| {
                        for (k, t) in Template::ALL.into_iter().enumerate() {
                            if template_card(ui, t, self.pictures.textures.get(&t), &pal).clicked() {
                                chosen = Some(t);
                            }
                            if k % 2 == 1 {
                                ui.end_row();
                            }
                        }
                    });
                    if self.replaced.is_some() {
                        ui.add_space(4.0);
                        if ui
                            .button(theme::with_icon(Icon::Undo2, "Back to the previous graph"))
                            .on_hover_text("The graph the last template replaced")
                            .clicked()
                        {
                            back = true;
                        }
                    }
                });
            });
        let escape = ui.input(|i| i.key_pressed(Key::Escape));
        if (area.response.clicked_elsewhere() && !toggled) || escape {
            self.templates_open = false;
        }
        if let Some(t) = chosen {
            self.start_from(doc, t, graph);
            *feedback = Some(format!("Started from the {} template", t.label()));
        }
        if back && let Some(previous) = self.replaced.take() {
            self.replaced = Some(std::mem::replace(graph, previous));
            self.selected = None;
            self.pinned = None;
            self.fit = true;
            self.templates_open = false;
        }
    }

    /// Replace `graph` by a template's, and set the view to suit it.
    fn start_from(&mut self, doc: &Document, t: Template, graph: &mut Graph) {
        let sea = doc.world.meta.sea_level as f32;
        let side = map_extent(doc, 2).size as f32;
        let new = t.graph(&doc.world.registry, sea, side);
        self.replaced = Some(std::mem::replace(graph, new));
        self.view.render = template_render(t, sea, self.view.render.exaggeration);
        self.view.camera = Default::default();
        self.selected = None;
        self.pinned = None;
        self.fit = true;
        self.templates_open = false;
    }

    /// Start drawing the templates' pictures for this map, and take those
    /// that are ready.
    fn make_pictures(&mut self, ctx: &egui::Context, doc: &Document) {
        let sea = doc.world.meta.sea_level;
        let extent = map_extent(doc, 112);
        let key = (doc.generation, sea, extent.size.to_bits());
        let p = &mut self.pictures;
        if p.made_for != Some(key) {
            p.made_for = Some(key);
            p.textures.clear();
            let (tx, rx) = channel();
            let reg = doc.world.registry.clone();
            let ctx = ctx.clone();
            std::thread::Builder::new()
                .name("massif template pictures".into())
                .spawn(move || {
                    for t in Template::ALL {
                        if let Some(image) = template_picture(t, &reg, sea as f32, &extent) {
                            if tx.send((t, image)).is_err() {
                                return;
                            }
                            ctx.request_repaint();
                        }
                    }
                })
                .expect("a thread for template pictures");
            p.rx = Some(rx);
        }
        if let Some(rx) = &p.rx {
            while let Ok((t, image)) = rx.try_recv() {
                let texture = ctx.load_texture(format!("template {t:?}"), image, TextureOptions::LINEAR);
                p.textures.insert(t, texture);
            }
        }
    }

    /// Start from a template, as its card does.
    #[cfg(test)]
    pub fn use_template(&mut self, doc: &mut Document, t: Template) {
        let mut graph = graph_of(doc);
        self.start_from(doc, t, &mut graph);
        self.store(doc, graph);
    }

    /// Whether a conversion is under way.
    #[cfg(test)]
    pub fn converting(&self) -> bool {
        self.build.is_some()
    }

    /// Open the template picker.
    #[cfg(test)]
    pub fn open_templates(&mut self) {
        self.templates_open = true;
    }

    /// How many template pictures are ready.
    #[cfg(test)]
    pub fn pictures_ready(&self) -> usize {
        self.pictures.textures.len()
    }

    fn to_screen(&self, p: [f32; 2]) -> Pos2 {
        self.canvas.min + self.pan + vec2(p[0], p[1]) * self.zoom
    }

    fn to_graph(&self, p: Pos2) -> [f32; 2] {
        let v = (p - self.canvas.min - self.pan) / self.zoom;
        [v.x, v.y]
    }

    fn node_height(kind: &NodeKind) -> f32 {
        let rows = kind.inputs().len().max(kind.outputs().len()) as f32;
        HEADER + rows * ROW + if kind.outputs().is_empty() { 8.0 } else { THUMB + 12.0 }
    }

    fn node_rect(&self, kind: &NodeKind, pos: [f32; 2]) -> Rect {
        Rect::from_min_size(self.to_screen(pos), vec2(NODE_WIDTH, Self::node_height(kind)) * self.zoom)
    }

    fn input_pos(&self, rect: Rect, k: usize) -> Pos2 {
        pos2(rect.left(), rect.top() + (HEADER + (k as f32 + 0.5) * ROW) * self.zoom)
    }

    fn output_pos(&self, rect: Rect, k: usize) -> Pos2 {
        pos2(rect.right(), rect.top() + (HEADER + (k as f32 + 0.5) * ROW) * self.zoom)
    }

    /// Zoom and move so that every node is in view.
    fn frame(&mut self, graph: &Graph) {
        if graph.nodes.is_empty() {
            return;
        }
        let (mut lo, mut hi) = (vec2(f32::MAX, f32::MAX), vec2(f32::MIN, f32::MIN));
        for n in &graph.nodes {
            let size = vec2(NODE_WIDTH, Self::node_height(&n.kind));
            lo = lo.min(vec2(n.pos[0], n.pos[1]));
            hi = hi.max(vec2(n.pos[0], n.pos[1]) + size);
        }
        let span = hi - lo + vec2(60.0, 60.0);
        let canvas = self.canvas;
        self.zoom = (canvas.width() / span.x).min(canvas.height() / span.y).clamp(0.25, 1.1);
        let centre = (lo + hi) * 0.5;
        self.pan = canvas.size() * 0.5 - centre * self.zoom;
    }

    fn open_library(&mut self, screen: Pos2, from: Option<(NodeId, u8)>) {
        self.library = Some(Library {
            at: self.to_graph(screen),
            screen,
            query: String::new(),
            from,
            focus: true,
        });
    }

    fn canvas(&mut self, ui: &mut Ui, canvas: Rect, graph: &mut Graph, reg: &Registry) {
        let pal = Palette::of(ui);
        if self.fit && canvas.width() > 50.0 {
            self.frame(graph);
            self.fit = false;
        }
        let background = ui.interact(canvas, ui.scope_id().with("graph canvas"), Sense::click_and_drag());
        let painter = ui.painter_at(canvas);
        painter.rect_filled(canvas, 0, pal.canvas);
        // Dot grid.
        let step = 24.0 * self.zoom;
        if step > 6.0 {
            let start = canvas.min + vec2(self.pan.x.rem_euclid(step), self.pan.y.rem_euclid(step));
            let mut y = start.y;
            while y < canvas.max.y {
                let mut x = start.x;
                while x < canvas.max.x {
                    painter.circle_filled(pos2(x, y), 0.8, pal.border);
                    x += step;
                }
                y += step;
            }
        }
        // Moving and zooming.
        if background.dragged_by(egui::PointerButton::Middle) || background.dragged_by(egui::PointerButton::Primary) {
            self.pan += background.drag_delta();
        }
        let hovered = ui.rect_contains_pointer(canvas);
        if hovered && self.library.is_none() {
            let (scroll, zoom, cmd) = ui.input(|i| (i.smooth_scroll_delta, i.zoom_delta(), i.modifiers.command || i.modifiers.ctrl));
            let pointer = ui.input(|i| i.pointer.hover_pos()).unwrap_or(canvas.center());
            let factor = if zoom != 1.0 {
                zoom
            } else if cmd && scroll.y != 0.0 {
                (scroll.y * 0.004).exp()
            } else {
                1.0
            };
            if factor != 1.0 {
                let new_zoom = (self.zoom * factor).clamp(0.25, 2.5);
                let anchor = pointer - canvas.min - self.pan;
                self.pan -= anchor * (new_zoom / self.zoom - 1.0);
                self.zoom = new_zoom;
            } else if scroll != Vec2::ZERO {
                self.pan += scroll;
            }
        }
        // Keys over the graph.
        let typing = ui.ctx().egui_wants_keyboard_input();
        if hovered && !typing && self.library.is_none() {
            let (tab, space, f, delete, v) = ui.input(|i| {
                (
                    i.key_pressed(Key::Tab),
                    i.key_pressed(Key::Space),
                    i.key_pressed(Key::F),
                    i.key_pressed(Key::Delete) || i.key_pressed(Key::Backspace),
                    i.key_pressed(Key::V),
                )
            });
            if v {
                self.view_key();
            }
            if tab || space {
                let at = ui.input(|i| i.pointer.hover_pos()).unwrap_or(canvas.center());
                self.open_library(at, None);
            }
            if f {
                self.fit = true;
            }
            if delete && let Some(id) = self.selected.take() {
                graph.remove(id);
                if self.pinned == Some(id) {
                    self.pinned = None;
                }
                self.revision += 1;
            }
        }
        if background.secondary_clicked()
            && let Some(p) = background.interact_pointer_pos()
        {
            self.open_library(p, None);
        }
        if background.clicked() {
            self.selected = None;
            self.library = None;
            self.revision += 1;
        }

        // Wires, coloured by what flows along them.
        let rects: HashMap<NodeId, Rect> = graph.nodes.iter().map(|n| (n.id, self.node_rect(&n.kind, n.pos))).collect();
        let zoom = self.zoom;
        let wire = |painter: &egui::Painter, a: Pos2, b: Pos2, color: Color32, width: f32| {
            let d = ((b.x - a.x).abs() * 0.5).max(30.0 * zoom);
            painter.add(egui::epaint::CubicBezierShape::from_points_stroke(
                [a, a + vec2(d, 0.0), b - vec2(d, 0.0), b],
                false,
                Color32::TRANSPARENT,
                Stroke::new(width * zoom.max(0.6), color),
            ));
        };
        for l in &graph.links {
            let (Some(a), Some(b)) = (rects.get(&l.from), rects.get(&l.to)) else { continue };
            let t = graph.output_type(l.from, l.output);
            let near = self.selected == Some(l.from) || self.selected == Some(l.to);
            let color = if near { port_color(t) } else { port_color(t).gamma_multiply(0.6) };
            wire(&painter, self.output_pos(*a, l.output as usize), self.input_pos(*b, l.input as usize), color, if near { 2.4 } else { 1.8 });
        }

        // Nodes.
        let mut drop_on: Option<(NodeId, u8)> = None;
        let pointer = ui.input(|i| i.pointer.interact_pos());
        let released = ui.input(|i| i.pointer.any_released());
        let mut remove = None;
        let mut pick_up = None;
        let ids: Vec<NodeId> = graph.nodes.iter().map(|n| n.id).collect();
        let shown = self.shown.map(|s| s.0);
        for id in ids {
            let node = graph.node(id).expect("listed").clone();
            let rect = rects[&id];
            let family = family_color(node.kind.family());
            let selected = self.selected == Some(id);
            let failed = self.failed.contains_key(&id);
            let border = if failed {
                pal.danger
            } else if selected {
                pal.accent
            } else {
                pal.border
            };
            painter.rect(rect, 8, pal.panel, Stroke::new(if selected { 1.6 } else { 1.0 }, border), StrokeKind::Inside);
            // A band of the family's colour across the top.
            let band = Rect::from_min_size(rect.min, vec2(rect.width(), 3.0 * self.zoom));
            painter.rect_filled(band, egui::CornerRadius { nw: 8, ne: 8, sw: 0, se: 0 }, family);
            let header = Rect::from_min_size(rect.min, vec2(rect.width(), HEADER * self.zoom));
            let title = match &node.kind {
                NodeKind::TerrainOutput { terrain } => format!("Terrain · {}", reg.terrain(TerrainId(*terrain)).name),
                NodeKind::Combine { op, .. } => format!("Combine · {}", op.label()),
                NodeKind::Gradient { preset, .. } => format!("Gradient · {}", preset.label()),
                NodeKind::Mountain { style, .. } => format!("Mountain · {}", style.label()),
                _ => node.kind.label().to_owned(),
            };
            painter.text(
                header.left_center() + vec2(10.0 * self.zoom, 1.5 * self.zoom),
                egui::Align2::LEFT_CENTER,
                title,
                egui::FontId::proportional(12.0 * self.zoom),
                pal.text,
            );
            if shown == Some(id) {
                painter.text(
                    header.right_center() + vec2(-9.0 * self.zoom, 1.5 * self.zoom),
                    egui::Align2::RIGHT_CENTER,
                    theme::glyph(Icon::Eye).to_string(),
                    egui::FontId::proportional(11.0 * self.zoom),
                    pal.accent,
                );
            }
            // Anywhere on the node (but its ports) selects it and drags it;
            // a double-click views it, as V does.
            let handle = ui.interact(rect, ui.scope_id().with(("graph node", id)), Sense::click_and_drag());
            if handle.dragged()
                && let Some(n) = graph.node_mut(id)
            {
                let d = handle.drag_delta() / self.zoom;
                n.pos = [n.pos[0] + d.x, n.pos[1] + d.y];
            }
            if handle.clicked() || handle.drag_started() {
                self.selected = Some(id);
                self.library = None;
            }
            if handle.double_clicked() {
                self.toggle_view(id);
            }
            if failed {
                handle.clone().on_hover_text(self.failed[&id].clone());
            } else {
                handle.clone().on_hover_text(node.kind.description());
            }
            handle.context_menu(|ui| {
                let view = if self.pinned == Some(id) { "Back to the output (V)" } else { "View this node (V)" };
                if ui.button(theme::with_icon(Icon::Eye, view)).clicked() {
                    self.toggle_view(id);
                    ui.close();
                }
                if ui.button(theme::with_icon(Icon::Trash2, "Delete node")).clicked() {
                    remove = Some(id);
                    ui.close();
                }
            });
            // Ports, coloured by type.
            let r = 4.5 * self.zoom;
            let font = egui::FontId::proportional(10.5 * self.zoom);
            for (k, (name, t)) in node.kind.inputs().iter().enumerate() {
                let p = self.input_pos(rect, k);
                let wired = graph.input_of(id, k as u8).is_some();
                let c = port_color(*t);
                painter.circle(p, r, if wired { c } else { pal.panel }, Stroke::new(1.5, c));
                painter.text(p + vec2(9.0 * self.zoom, 0.0), egui::Align2::LEFT_CENTER, *name, font.clone(), pal.weak);
                let port = ui.interact(Rect::from_center_size(p, vec2(16.0, 16.0) * self.zoom.max(0.7)), ui.scope_id().with(("in", id, k)), Sense::click_and_drag());
                if (port.clicked() || port.drag_started()) && wired {
                    pick_up = Some((id, k as u8));
                }
                if self.wiring.is_some() && pointer.is_some_and(|q| q.distance(p) < 14.0 * self.zoom.max(0.7)) {
                    painter.circle_stroke(p, r + 3.0, Stroke::new(2.0, pal.accent));
                    if released {
                        drop_on = Some((id, k as u8));
                    }
                }
            }
            for (k, (name, _)) in node.kind.outputs().iter().enumerate() {
                let p = self.output_pos(rect, k);
                let c = port_color(graph.output_type(id, k as u8));
                painter.circle(p, r, c, Stroke::new(1.5, c));
                painter.text(p - vec2(9.0 * self.zoom, 0.0), egui::Align2::RIGHT_CENTER, *name, font.clone(), pal.weak);
                let port = ui.interact(Rect::from_center_size(p, vec2(16.0, 16.0) * self.zoom.max(0.7)), ui.scope_id().with(("out", id, k)), Sense::drag());
                if port.drag_started() {
                    self.wiring = Some((id, k as u8));
                }
            }
            // Thumbnail of the output shown (the first one, or the one picked).
            if !node.kind.outputs().is_empty() {
                let rows = node.kind.inputs().len().max(node.kind.outputs().len()) as f32;
                let top = rect.top() + (HEADER + rows * ROW + 6.0) * self.zoom;
                let out = if selected { self.view_output.min(node.kind.outputs().len() as u8 - 1) } else { 0 };
                let thumb = Rect::from_min_size(pos2(rect.left() + 8.0 * self.zoom, top), vec2(rect.width() - 16.0 * self.zoom, THUMB * self.zoom));
                match self.textures.get(&(id, out)) {
                    Some(t) => {
                        // Square pixels: crop the picture to the strip.
                        let aspect = thumb.height() / thumb.width();
                        let uv = Rect::from_min_max(pos2(0.0, 0.5 - aspect / 2.0), pos2(1.0, 0.5 + aspect / 2.0));
                        painter.image(t.id(), thumb, uv, Color32::WHITE);
                        painter.rect_stroke(thumb, 4, Stroke::new(1.0, pal.border), StrokeKind::Inside);
                    }
                    None => {
                        painter.rect_filled(thumb, 4, pal.header);
                    }
                }
            }
        }
        if let Some((to, input)) = pick_up
            && let Some((from, output)) = graph.input_of(to, input)
        {
            graph.disconnect(to, input);
            self.wiring = Some((from, output));
        }
        if let Some(id) = remove {
            graph.remove(id);
            if self.selected == Some(id) {
                self.selected = None;
            }
            if self.pinned == Some(id) {
                self.pinned = None;
            }
        }
        // The wire being drawn.
        if let Some((from, output)) = self.wiring {
            if let (Some(a), Some(p)) = (rects.get(&from), pointer) {
                wire(&painter, self.output_pos(*a, output as usize), p, port_color(graph.output_type(from, output)), 2.4);
            }
            if released {
                match drop_on {
                    Some((to, input)) => {
                        if !graph.connect(from, output, to, input) {
                            ui.ctx().debug_text("That would make a loop");
                        }
                    }
                    // Dropped on empty space: add a node there, wired.
                    None => {
                        if let Some(p) = pointer.filter(|p| canvas.contains(*p) && !rects.values().any(|r| r.contains(*p))) {
                            self.open_library(p, Some((from, output)));
                        }
                    }
                }
                self.wiring = None;
            }
        }
        if graph.nodes.is_empty() {
            painter.text(canvas.center(), egui::Align2::CENTER_CENTER, "Right-click or press Tab to add a node", egui::FontId::proportional(14.0), pal.weak);
        }
        self.library_ui(ui, graph);
    }

    /// The library of nodes: type to search, Enter adds the first match.
    fn library_ui(&mut self, ui: &mut Ui, graph: &mut Graph) {
        let Some(lib) = &mut self.library else { return };
        let pal = Palette::of(ui);
        let mut close = ui.input(|i| i.key_pressed(Key::Escape));
        let mut add: Option<NodeKind> = None;
        let screen = lib.screen;
        let wanted = lib.from.map(|(id, out)| graph.output_type(id, out));
        let area = egui::Area::new(ui.scope_id().with("graph library"))
            .order(egui::Order::Foreground)
            .fixed_pos(screen)
            .constrain(true)
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).inner_margin(egui::Margin::same(8)).show(ui, |ui| {
                    ui.set_width(280.0);
                    let edit = ui.add(
                        egui::TextEdit::singleline(&mut lib.query)
                            .hint_text(format!("{}  Search nodes", theme::glyph(Icon::Search)))
                            .desired_width(f32::INFINITY),
                    );
                    if lib.focus {
                        edit.request_focus();
                        lib.focus = false;
                    }
                    let q = lib.query.to_lowercase();
                    let matches: Vec<NodeKind> = NodeKind::catalogue()
                        .into_iter()
                        .filter(|k| {
                            q.is_empty() || k.label().to_lowercase().contains(&q) || k.description().to_lowercase().contains(&q) || k.family().label().to_lowercase().contains(&q)
                        })
                        // With a wire in hand, only nodes that take it.
                        .filter(|k| match wanted {
                            None => true,
                            Some(PortType::Color) => k.inputs().iter().any(|i| i.1 == PortType::Color),
                            Some(_) => k.inputs().iter().any(|i| i.1 != PortType::Color),
                        })
                        .collect();
                    if edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                        add = matches.first().cloned();
                    }
                    ui.add_space(4.0);
                    egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                        for family in Family::ALL {
                            let in_family: Vec<&NodeKind> = matches.iter().filter(|k| k.family() == family).collect();
                            if in_family.is_empty() {
                                continue;
                            }
                            ui.horizontal(|ui| {
                                let (dot, _) = ui.allocate_exact_size(vec2(8.0, 8.0), Sense::hover());
                                ui.painter().circle_filled(dot.center(), 3.5, family_color(family));
                                ui.label(RichText::new(family.label()).size(11.0).color(pal.weak).medium());
                            });
                            for k in in_family {
                                let r = ui.add(egui::Button::new(k.label()).frame(false).min_size(vec2(260.0, 20.0)));
                                if r.clicked() {
                                    add = Some(k.clone());
                                }
                                r.on_hover_text(k.description());
                            }
                            ui.add_space(2.0);
                        }
                        if matches.is_empty() {
                            ui.label(RichText::new("No node by that name").color(pal.weak));
                        }
                    });
                });
            });
        // A click elsewhere closes it.
        if ui.input(|i| i.pointer.any_pressed()) && !area.response.contains_pointer() && add.is_none() {
            let p = ui.input(|i| i.pointer.interact_pos());
            if p.is_some_and(|p| !area.response.rect.contains(p)) {
                close = true;
            }
        }
        if let Some(kind) = add {
            let lib = self.library.take().expect("open");
            let id = graph.add(kind, lib.at);
            if let Some((from, output)) = lib.from {
                // Wire the first input of a matching type.
                let t = graph.output_type(from, output);
                let inputs = graph.node(id).map(|n| n.kind.inputs()).unwrap_or(&[]);
                let k = inputs
                    .iter()
                    .position(|i| (i.1 == PortType::Color) == (t == PortType::Color))
                    .unwrap_or(0);
                if !inputs.is_empty() {
                    graph.connect(from, output, id, k as u8);
                }
            }
            self.selected = Some(id);
            self.view_output = 0;
            self.revision += 1;
        } else if close {
            self.library = None;
        }
    }

    // ---------------------------------------------------------- properties --

    /// The settings of the selected node.
    pub fn properties_ui(&mut self, ui: &mut Ui, doc: &mut Document) {
        self.sync(ui.ctx(), doc);
        let pal = Palette::of(ui);
        let mut graph = graph_of(doc);
        let before = graph.clone();
        ui.spacing_mut().slider_width = (ui.available_width() - 120.0).clamp(80.0, 240.0);
        let Some(id) = self.selected.filter(|id| graph.node(*id).is_some()) else {
            ui.label(RichText::new("Terrain graph").bold().size(14.0));
            ui.add_space(2.0);
            ui.label(
                RichText::new(
                    "Build relief from nodes, as in Gaea: landforms and primitives make it, adjustments and simulations shape it, \
                     masks pick parts of it, colour nodes paint it. The viewport shows the graph's output; select a node and \
                     press V to see the graph up to it. When it looks right, converting the graph writes its heights, terrains \
                     and real blocks to the map.",
                )
                .size(11.5)
                .color(pal.weak),
            );
            ui.add_space(8.0);
            ui.label(RichText::new(format!("{} nodes, {} links", graph.nodes.len(), graph.links.len())).size(11.5).color(pal.weak));
            ui.label(RichText::new("Right-click the graph or press Tab to add a node.").size(11.5).color(pal.weak));
            return;
        };
        let node = graph.node_mut(id).expect("checked");
        let family = node.kind.family();
        ui.horizontal(|ui| {
            let (dot, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
            ui.painter().circle_filled(dot.center(), 4.0, family_color(family));
            ui.label(RichText::new(node.kind.label()).bold().size(15.0));
            ui.label(RichText::new(family.label()).size(11.0).color(pal.weak));
        });
        ui.label(RichText::new(node.kind.description()).size(11.5).color(pal.weak));
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let viewed = self.pinned == Some(id);
            if ui
                .selectable_label(viewed, theme::with_icon(Icon::Eye, if viewed { "Viewing (V)" } else { "View (V)" }))
                .on_hover_text("Show this node in the viewport (V, or double-click it); V again goes back to the graph's output")
                .clicked()
            {
                self.toggle_view(id);
            }
            let outs = node.kind.outputs();
            if outs.len() > 1 {
                ui.separator();
                for (k, (name, _)) in outs.iter().enumerate() {
                    if ui.selectable_label(viewed && self.view_output == k as u8, *name).clicked() {
                        self.view_output = k as u8;
                        self.pinned = Some(id);
                        self.revision += 1;
                    }
                }
            }
        });
        if let Some(e) = self.failed.get(&id) {
            ui.colored_label(pal.danger, e);
        }
        ui.add_space(6.0);
        let mut delete = false;
        egui::ScrollArea::vertical().id_salt("node properties").show(ui, |ui| {
            properties(ui, &mut node.kind, &doc.world.registry);
            ui.add_space(10.0);
            delete = ui.button(theme::with_icon(Icon::Trash2, "Delete node")).clicked();
        });
        if delete {
            graph.remove(id);
            self.selected = None;
            if self.pinned == Some(id) {
                self.pinned = None;
            }
        }
        if graph != before {
            self.store(doc, graph);
        }
    }
}

/// Turn a build's colours into terrains of real blocks: the blocks of the
/// palette (read from the game, or Massif's terrains without it), only
/// those the colours come close to, made terrains of the world.
/// The Solid levels the layers of a conversion go to, in their order: those
/// earlier conversions made, new ones for the others. Levels of layers the
/// graph no longer has are removed. Called at the start of the history step.
fn layer_levels(doc: &mut Document, layers: &[LayerOut]) -> Vec<usize> {
    let find = |doc: &Document, slot: u32| {
        doc.dimension()
            .levels
            .iter()
            .position(|l| l.kind == LevelKind::Solid && LayerOut::is_level_of(&l.name, slot))
    };
    let graph_level = |name: &str| (1..=64).any(|slot| LayerOut::is_level_of(name, slot));
    let stale: Vec<usize> = doc
        .dimension()
        .levels
        .iter()
        .enumerate()
        .filter(|(_, l)| l.kind == LevelKind::Solid && graph_level(&l.name) && !layers.iter().any(|o| LayerOut::is_level_of(&l.name, o.slot)))
        .map(|(k, _)| k)
        .collect();
    let missing: Vec<&LayerOut> = layers.iter().filter(|l| find(doc, l.slot).is_none()).collect();
    if !stale.is_empty() || !missing.is_empty() {
        let mut edit = doc.history.edit(&mut doc.world, 0);
        let dim = edit.structure();
        for k in stale.into_iter().rev() {
            dim.remove_level(k);
        }
        for l in missing {
            dim.add_level(l.name.clone(), LevelKind::Solid);
        }
    }
    layers.iter().map(|l| find(doc, l.slot).expect("the level was just made")).collect()
}

fn block_palette(doc: &mut Document, blocks: &BlockLibrary, set: BlockPalette, colors: &ColorField) -> Vec<(TerrainId, [u8; 3])> {
    match &blocks.catalogue {
        Some(cat) => {
            let wanted = set.blocks();
            let candidates: Vec<&massif_assets::Block> = if wanted.is_empty() {
                cat.blocks.iter().filter(|b| b.full_cube && b.top.is_some()).collect()
            } else {
                wanted.iter().filter_map(|id| cat.block(id)).filter(|b| b.full_cube).collect()
            };
            let rgb: Vec<[u8; 3]> = candidates.iter().map(|b| b.color).collect();
            let used = massif_core::graph::colors_in_use(colors, &rgb);
            candidates
                .into_iter()
                .zip(used)
                .filter(|(_, u)| *u)
                .map(|(b, _)| (massif_assets::block_terrain(&mut doc.world.registry, b), b.color))
                .collect()
        }
        None => doc
            .world
            .registry
            .terrains
            .iter()
            .enumerate()
            .map(|(k, t)| (TerrainId(k as u16), t.color))
            .collect(),
    }
}

// ------------------------------------------------------------- settings --

/// A row of settings: a label, then the control.
fn row(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui) -> egui::Response) -> bool {
    ui.horizontal(|ui| {
        ui.spacing_mut().interact_size.y = 18.0;
        let l = ui.add_sized([96.0, 18.0], egui::Label::new(RichText::new(label).size(12.0)).truncate());
        add(ui).labelled_by(l.id).changed()
    })
    .inner
}

/// A distance in blocks: a plain, linear slider (type a value to go past
/// its end).
fn blocks(ui: &mut Ui, label: &str, v: &mut f32, max: f32) -> bool {
    row(ui, label, |ui| {
        ui.add(
            egui::Slider::new(v, 1.0..=max.min(6000.0))
                .clamping(egui::SliderClamping::Never)
                .suffix(" blocks")
                .max_decimals(0),
        )
    })
}

fn height(ui: &mut Ui, label: &str, v: &mut f32) -> bool {
    row(ui, label, |ui| {
        ui.add(
            egui::Slider::new(v, -64.0..=320.0)
                .clamping(egui::SliderClamping::Never)
                .max_decimals(0),
        )
    })
}

fn amount(ui: &mut Ui, label: &str, v: &mut f32, range: std::ops::RangeInclusive<f32>) -> bool {
    row(ui, label, |ui| ui.add(egui::Slider::new(v, range).max_decimals(2)))
}

fn fraction(ui: &mut Ui, label: &str, v: &mut f32) -> bool {
    row(ui, label, |ui| {
        ui.add(
            egui::Slider::new(v, 0.0..=1.0)
                .custom_formatter(|v, _| format!("{:.0}%", v * 100.0))
                .custom_parser(|s| s.trim_end_matches('%').trim().parse::<f64>().ok().map(|v| v / 100.0)),
        )
    })
}

fn angle(ui: &mut Ui, label: &str, v: &mut f32) -> bool {
    row(ui, label, |ui| ui.add(egui::Slider::new(v, 0.0..=360.0).suffix("°").max_decimals(0)))
}

fn count(ui: &mut Ui, label: &str, v: &mut u32, range: std::ops::RangeInclusive<u32>) -> bool {
    row(ui, label, |ui| ui.add(egui::Slider::new(v, range)))
}

fn seed(ui: &mut Ui, v: &mut u32) -> bool {
    ui.horizontal(|ui| {
        ui.add_sized([96.0, 18.0], egui::Label::new(RichText::new("Seed").size(12.0)));
        let mut changed = ui.add(egui::DragValue::new(v)).changed();
        if ui.small_button(theme::glyph(Icon::Dices).to_string()).on_hover_text("Another seed").clicked() {
            *v = v.wrapping_mul(1_103_515_245).wrapping_add(12_345) % 100_000;
            changed = true;
        }
        changed
    })
    .inner
}

fn choice<T: PartialEq + Copy>(ui: &mut Ui, label: &str, v: &mut T, options: &[(T, &str)]) -> bool {
    row(ui, label, |ui| {
        let current = options.iter().find(|o| o.0 == *v).map_or("", |o| o.1);
        let mut changed = false;
        let mut r = egui::ComboBox::from_id_salt(label)
            .selected_text(current)
            .width(150.0)
            .show_ui(ui, |ui| {
                for (o, name) in options {
                    if ui.selectable_value(v, *o, *name).changed() {
                        changed = true;
                    }
                }
            })
            .response;
        if changed {
            r.mark_changed();
        }
        r
    })
}

fn color(ui: &mut Ui, label: &str, v: &mut [u8; 3]) -> bool {
    row(ui, label, |ui| ui.color_edit_button_srgb(v))
}

fn flag(ui: &mut Ui, label: &str, v: &mut bool) -> bool {
    row(ui, label, |ui| ui.checkbox(v, ""))
}

/// Swatches of the gradients, to pick one.
fn gradient_picker(ui: &mut Ui, preset: &mut GradientPreset) -> bool {
    let mut changed = false;
    let pal = Palette::of(ui);
    let width = ((ui.available_width() - 8.0) / 2.0).max(60.0);
    egui::Grid::new("gradient presets").num_columns(2).spacing(vec2(6.0, 4.0)).show(ui, |ui| {
        for (k, p) in GradientPreset::ALL.iter().enumerate() {
            let (rect, r) = ui.allocate_exact_size(vec2(width, 20.0), Sense::click());
            let painter = ui.painter();
            let steps = 24;
            for s in 0..steps {
                let t = s as f32 / (steps - 1) as f32;
                let c = p.at(t).map(massif_core::graph::encode_srgb);
                let x0 = rect.left() + rect.width() * s as f32 / steps as f32;
                let x1 = rect.left() + rect.width() * (s + 1) as f32 / steps as f32;
                painter.rect_filled(Rect::from_min_max(pos2(x0, rect.top()), pos2(x1 + 0.5, rect.bottom())), 0, Color32::from_rgb(c[0], c[1], c[2]));
            }
            let selected = *preset == *p;
            painter.rect_stroke(rect, 3, Stroke::new(if selected { 2.0 } else { 1.0 }, if selected { pal.accent } else { pal.border }), StrokeKind::Inside);
            painter.text(rect.left_center() + vec2(6.0, 0.0), egui::Align2::LEFT_CENTER, p.label(), egui::FontId::proportional(10.5), Color32::WHITE);
            if r.clicked() {
                *preset = *p;
                changed = true;
            }
            if k % 2 == 1 {
                ui.end_row();
            }
        }
    });
    changed
}

/// The settings of a node. Every control reports changes through the graph
/// comparison done by the caller.
fn properties(ui: &mut Ui, kind: &mut NodeKind, reg: &Registry) {
    let pal = Palette::of(ui);
    let section = |ui: &mut Ui, name: &str| {
        ui.add_space(6.0);
        ui.label(RichText::new(name).size(11.0).color(pal.weak).medium());
    };
    match kind {
        NodeKind::Noise {
            style,
            scale,
            octaves,
            roughness,
            height: h,
            base,
            seed: s,
        } => {
            choice(ui, "Style", style, &[(NoiseStyle::Fbm, "Rolling"), (NoiseStyle::Ridged, "Ridged"), (NoiseStyle::Billow, "Billow")]);
            blocks(ui, "Feature size", scale, 20000.0);
            count(ui, "Detail", octaves, 1..=12);
            amount(ui, "Roughness", roughness, 0.05..=0.95);
            height(ui, "Height", h);
            height(ui, "Base", base);
            seed(ui, s);
        }
        NodeKind::Cells {
            style,
            scale,
            jitter,
            height: h,
            base,
            seed: s,
        } => {
            choice(ui, "Style", style, &[(CellStyle::Cones, "Cones"), (CellStyle::Ridges, "Ridges")]);
            blocks(ui, "Cell size", scale, 20000.0);
            fraction(ui, "Jitter", jitter);
            height(ui, "Height", h);
            height(ui, "Base", base);
            seed(ui, s);
        }
        NodeKind::Radial {
            inner,
            outer,
            height: h,
            base,
        } => {
            fraction(ui, "Fall starts", inner);
            fraction(ui, "Fall ends", outer);
            amount(ui, "Height", h, -400.0..=400.0);
            amount(ui, "Base", base, -64.0..=320.0);
        }
        NodeKind::Constant { value } => {
            row(ui, "Value", |ui| ui.add(egui::DragValue::new(value).speed(0.5)));
        }
        NodeKind::LinearGradient { angle: a, height: h, base } => {
            angle(ui, "Rises toward", a);
            amount(ui, "Height", h, -400.0..=400.0);
            amount(ui, "Base", base, -64.0..=320.0);
        }
        NodeKind::Shape {
            shape,
            x,
            z,
            radius,
            falloff,
            height: h,
            base,
        } => {
            choice(ui, "Shape", shape, &ShapeKind::ALL.map(|s| (s, s.label())));
            fraction(ui, "Across (x)", x);
            fraction(ui, "Down (z)", z);
            blocks(ui, "Radius", radius, 10000.0);
            fraction(ui, "Soft edge", falloff);
            amount(ui, "Height", h, -400.0..=400.0);
            amount(ui, "Base", base, -64.0..=320.0);
        }
        NodeKind::Mountain {
            style,
            x,
            z,
            radius,
            height: h,
            base,
            detail,
            seed: s,
        } => {
            choice(ui, "Style", style, &MountainStyle::ALL.map(|s| (s, s.label())));
            blocks(ui, "Radius", radius, 10000.0);
            height(ui, "Height", h);
            height(ui, "Base", base);
            fraction(ui, "Detail", detail);
            section(ui, "Where");
            fraction(ui, "Across (x)", x);
            fraction(ui, "Down (z)", z);
            seed(ui, s);
        }
        NodeKind::MountainRange {
            angle: a,
            width,
            scale,
            height: h,
            base,
            seed: s,
        } => {
            angle(ui, "Runs toward", a);
            blocks(ui, "Width", width, 10000.0);
            blocks(ui, "Peak size", scale, 5000.0);
            height(ui, "Height", h);
            height(ui, "Base", base);
            seed(ui, s);
        }
        NodeKind::Volcano {
            x,
            z,
            radius,
            height: h,
            base,
            crater,
            depth,
            seed: s,
        } => {
            blocks(ui, "Radius", radius, 10000.0);
            height(ui, "Height", h);
            height(ui, "Base", base);
            fraction(ui, "Crater width", crater);
            amount(ui, "Crater depth", depth, 0.0..=200.0);
            section(ui, "Where");
            fraction(ui, "Across (x)", x);
            fraction(ui, "Down (z)", z);
            seed(ui, s);
        }
        NodeKind::Crater {
            x,
            z,
            radius,
            depth,
            rim,
            peak,
            base,
            seed: s,
        } => {
            blocks(ui, "Radius", radius, 5000.0);
            amount(ui, "Depth", depth, 0.0..=200.0);
            amount(ui, "Rim", rim, 0.0..=100.0);
            amount(ui, "Central peak", peak, 0.0..=100.0);
            height(ui, "Plain", base);
            section(ui, "Where");
            fraction(ui, "Across (x)", x);
            fraction(ui, "Down (z)", z);
            seed(ui, s);
        }
        NodeKind::CraterField {
            count: c,
            min_radius,
            max_radius,
            depth,
            base,
            seed: s,
        } => {
            count(ui, "Craters", c, 1..=2000);
            blocks(ui, "Smallest", min_radius, 2000.0);
            blocks(ui, "Largest", max_radius, 5000.0);
            fraction(ui, "Depth", depth);
            height(ui, "Plain", base);
            seed(ui, s);
        }
        NodeKind::Dunes {
            angle: a,
            wavelength,
            height: h,
            base,
            sharpness,
            seed: s,
        } => {
            angle(ui, "Wind toward", a);
            blocks(ui, "Spacing", wavelength, 2000.0);
            amount(ui, "Height", h, 0.0..=120.0);
            height(ui, "Base", base);
            fraction(ui, "Steep lee side", sharpness);
            seed(ui, s);
        }
        NodeKind::Canyon {
            angle: a,
            width,
            depth,
            meander,
            steps,
            base,
            seed: s,
        } => {
            angle(ui, "Runs toward", a);
            blocks(ui, "Width", width, 5000.0);
            amount(ui, "Depth", depth, 0.0..=300.0);
            blocks(ui, "Winding", meander, 3000.0);
            count(ui, "Steps", steps, 0..=20);
            height(ui, "Plateau", base);
            seed(ui, s);
        }
        NodeKind::Mesas {
            scale,
            coverage,
            height: h,
            base,
            steps,
            seed: s,
        } => {
            blocks(ui, "Size", scale, 5000.0);
            fraction(ui, "Coverage", coverage);
            amount(ui, "Height", h, 0.0..=300.0);
            height(ui, "Base", base);
            count(ui, "Steps", steps, 1..=12);
            seed(ui, s);
        }
        NodeKind::Island {
            size,
            height: h,
            base,
            coast,
            seed: s,
        } => {
            fraction(ui, "Size", size);
            amount(ui, "Height", h, 0.0..=380.0);
            height(ui, "Sea floor", base);
            fraction(ui, "Ragged coast", coast);
            seed(ui, s);
        }
        NodeKind::Rocks {
            scale,
            height: h,
            sharpness,
            seed: s,
        } => {
            blocks(ui, "Size", scale, 2000.0);
            amount(ui, "Height", h, 0.0..=100.0);
            fraction(ui, "Sharpness", sharpness);
            seed(ui, s);
        }
        NodeKind::Levels {
            in_min,
            in_max,
            out_min,
            out_max,
            gamma,
        } => {
            section(ui, "From");
            height(ui, "Low", in_min);
            height(ui, "High", in_max);
            section(ui, "To");
            height(ui, "Low", out_min);
            height(ui, "High", out_max);
            amount(ui, "Gamma", gamma, 0.1..=5.0);
        }
        NodeKind::Terrace { steps, strength } => {
            count(ui, "Steps", steps, 1..=64);
            fraction(ui, "Strength", strength);
        }
        NodeKind::Clamp { min, max } => {
            height(ui, "Lowest", min);
            height(ui, "Highest", max);
        }
        NodeKind::Blur { radius } => {
            blocks(ui, "Radius", radius, 500.0);
        }
        NodeKind::Warp { amount: a, scale, seed: s } => {
            blocks(ui, "Distance", a, 2000.0);
            blocks(ui, "Size", scale, 10000.0);
            seed(ui, s);
        }
        NodeKind::Normalize { min, max } => {
            height(ui, "Lowest", min);
            height(ui, "Highest", max);
        }
        NodeKind::Curve { gamma, contrast } => {
            amount(ui, "Gamma", gamma, 0.1..=6.0);
            amount(ui, "Contrast", contrast, -1.0..=1.0);
        }
        NodeKind::Transform {
            scale,
            rotate,
            offset_x,
            offset_z,
        } => {
            amount(ui, "Scale", scale, 0.1..=8.0);
            row(ui, "Turn", |ui| ui.add(egui::Slider::new(rotate, -180.0..=180.0).suffix("°").max_decimals(0)));
            amount(ui, "Move across", offset_x, -1.0..=1.0);
            amount(ui, "Move down", offset_z, -1.0..=1.0);
        }
        NodeKind::Swirl { angle: a, radius } => {
            row(ui, "Turn", |ui| ui.add(egui::Slider::new(a, -720.0..=720.0).suffix("°").max_decimals(0)));
            fraction(ui, "Reach", radius);
        }
        NodeKind::Fold { level, strength } => {
            height(ui, "Level", level);
            fraction(ui, "Strength", strength);
        }
        NodeKind::Stratify {
            layers,
            strength,
            tilt,
            seed: s,
        } => {
            count(ui, "Layers", layers, 1..=80);
            fraction(ui, "Strength", strength);
            row(ui, "Tilt", |ui| ui.add(egui::Slider::new(tilt, -45.0..=45.0).suffix("°").max_decimals(0)));
            seed(ui, s);
        }
        NodeKind::Sharpen { radius, amount: a } => {
            blocks(ui, "Radius", radius, 500.0);
            amount(ui, "Amount", a, 0.0..=4.0);
        }
        NodeKind::InvertHeight | NodeKind::Blend | NodeKind::Invert | NodeKind::MixColors | NodeKind::HeightOutput => {
            ui.label(RichText::new("No settings.").size(11.5).color(pal.weak));
        }
        NodeKind::Erosion {
            rain,
            erosion,
            deposition,
            capacity,
            evaporation,
            radius,
            seed: s,
        } => {
            amount(ui, "Rain", rain, 0.05..=8.0);
            fraction(ui, "Erosion", erosion);
            fraction(ui, "Deposition", deposition);
            amount(ui, "Capacity", capacity, 0.1..=20.0);
            amount(ui, "Evaporation", evaporation, 0.0..=0.5);
            blocks(ui, "Brush", radius, 100.0);
            seed(ui, s);
        }
        NodeKind::Thermal {
            talus,
            iterations,
            strength,
        } => {
            row(ui, "Steepest", |ui| ui.add(egui::Slider::new(talus, 5.0..=85.0).suffix("°").max_decimals(0)));
            count(ui, "Passes", iterations, 1..=500);
            fraction(ui, "Strength", strength);
        }
        NodeKind::Snow {
            line,
            slope,
            depth,
            softness,
        } => {
            height(ui, "Snow line", line);
            row(ui, "Steepest", |ui| ui.add(egui::Slider::new(slope, 5.0..=85.0).suffix("°").max_decimals(0)));
            amount(ui, "Depth", depth, 0.0..=20.0);
            amount(ui, "Fade", softness, 0.5..=100.0);
        }
        NodeKind::Rivers { threshold, depth, width } => {
            fraction(ui, "Fewer", threshold);
            amount(ui, "Depth", depth, 0.0..=40.0);
            blocks(ui, "Width", width, 200.0);
        }
        NodeKind::Lakes { amount: a } => {
            fraction(ui, "Fill", a);
        }
        NodeKind::Sea { level, shore } => {
            height(ui, "Level", level);
            amount(ui, "Shore", shore, 0.5..=40.0);
        }
        NodeKind::Combine { op, weight } => {
            choice(ui, "Operation", op, &CombineOp::ALL.map(|o| (o, o.label())));
            amount(ui, "Weight of B", weight, -4.0..=4.0);
        }
        NodeKind::SelectHeight { min, max, falloff } => {
            height(ui, "From", min);
            height(ui, "To", max);
            amount(ui, "Fade", falloff, 0.1..=100.0);
        }
        NodeKind::SelectSlope { min, max, falloff } => {
            row(ui, "From", |ui| ui.add(egui::Slider::new(min, 0.0..=90.0).suffix("°").max_decimals(0)));
            row(ui, "To", |ui| ui.add(egui::Slider::new(max, 0.0..=90.0).suffix("°").max_decimals(0)));
            amount(ui, "Fade", falloff, 0.1..=30.0);
        }
        NodeKind::Curvature { bend, radius, strength } => {
            choice(ui, "Pick", bend, &[(Bend::Convex, "Ridges"), (Bend::Concave, "Hollows")]);
            blocks(ui, "Size", radius, 1000.0);
            amount(ui, "Strength", strength, 0.1..=5.0);
        }
        NodeKind::Facing { angle: a, spread } => {
            angle(ui, "Facing", a);
            row(ui, "Spread", |ui| ui.add(egui::Slider::new(spread, 5.0..=180.0).suffix("°").max_decimals(0)));
        }
        NodeKind::Occlusion { radius } => {
            blocks(ui, "Radius", radius, 1000.0);
        }
        NodeKind::Flow { spread } => {
            fraction(ui, "Small rills", spread);
        }
        NodeKind::Gradient {
            preset,
            auto_range,
            min,
            max,
            reverse,
            variation,
            seed: s,
        } => {
            gradient_picker(ui, preset);
            ui.add_space(4.0);
            flag(ui, "Fit the range", auto_range);
            if !*auto_range {
                amount(ui, "From", min, -64.0..=320.0);
                amount(ui, "To", max, -64.0..=320.0);
            }
            flag(ui, "Reverse", reverse);
            fraction(ui, "Variation", variation);
            seed(ui, s);
        }
        NodeKind::SolidColor { color: c } => {
            color(ui, "Colour", c);
        }
        NodeKind::AdjustColor {
            hue,
            saturation,
            brightness,
            contrast,
        } => {
            row(ui, "Hue", |ui| ui.add(egui::Slider::new(hue, -180.0..=180.0).suffix("°").max_decimals(0)));
            amount(ui, "Saturation", saturation, 0.0..=3.0);
            amount(ui, "Brightness", brightness, 0.0..=3.0);
            amount(ui, "Contrast", contrast, 0.0..=3.0);
        }
        NodeKind::Tint { color: c, mode, strength } => {
            color(ui, "Colour", c);
            choice(ui, "Mode", mode, &TintMode::ALL.map(|m| (m, m.label())));
            fraction(ui, "Strength", strength);
        }
        NodeKind::AutoColor {
            palette,
            snow_line,
            rock_slope,
            seed: s,
        } => {
            choice(ui, "Land", palette, &AutoPalette::ALL.map(|p| (p, p.label())));
            height(ui, "Snow line", snow_line);
            row(ui, "Rock from", |ui| ui.add(egui::Slider::new(rock_slope, 5.0..=80.0).suffix("°").max_decimals(0)));
            seed(ui, s);
        }
        NodeKind::TerrainOutput { terrain } => {
            row(ui, "Terrain", |ui| {
                let mut changed = false;
                let mut r = egui::ComboBox::from_id_salt("output terrain")
                    .selected_text(reg.terrain(TerrainId(*terrain)).name.clone())
                    .width(170.0)
                    .height(320.0)
                    .show_ui(ui, |ui| {
                        for (k, t) in reg.terrains.iter().enumerate() {
                            if ui.selectable_value(terrain, k as u16, &t.name).changed() {
                                changed = true;
                            }
                        }
                    })
                    .response;
                if changed {
                    r.mark_changed();
                }
                r
            });
        }
        NodeKind::FloatingIslands {
            altitude,
            spread,
            size,
            coverage,
            depth,
            seed: s,
        } => {
            height(ui, "Altitude", altitude);
            blocks(ui, "Altitude spread", spread, 150.0);
            blocks(ui, "Largest radius", size, 600.0);
            fraction(ui, "Coverage", coverage);
            amount(ui, "Underside", depth, 0.2..=3.0);
            seed(ui, s);
            ui.label(
                RichText::new(
                    "Wire Top and Bottom to a Layer output: the islands become a level of the map, floating over the void.",
                )
                .size(11.0)
                .color(pal.weak),
            );
        }
        NodeKind::Overhang {
            reach,
            thickness,
            drop,
            variation,
            seed: s,
        } => {
            blocks(ui, "Reach", reach, 64.0);
            blocks(ui, "Thickness", thickness, 48.0);
            blocks(ui, "Cliff drop", drop, 160.0);
            fraction(ui, "Variation", variation);
            seed(ui, s);
            ui.label(
                RichText::new("Wire Top and Bottom to a Layer output: the rims jut out over the void at the foot of the cliffs.")
                    .size(11.0)
                    .color(pal.weak),
            );
        }
        NodeKind::LayerOutput { slot, palette } => {
            count(ui, "Layer", slot, 1..=8);
            choice(ui, "Blocks from", palette, &BlockPalette::ALL.map(|p| (p, p.label())));
            ui.label(
                RichText::new(
                    "When converted, the rock between Top and Bottom becomes a Solid level of the map, its columns spans over \
                     the void (one level per layer number). Without a colour, its top is grass.",
                )
                .size(11.0)
                .color(pal.weak),
            );
        }
        NodeKind::ColorOutput { blocks: b, palette } => {
            flag(ui, "Real blocks", b);
            if *b {
                choice(ui, "From", palette, &BlockPalette::ALL.map(|p| (p, p.label())));
            }
            ui.label(
                RichText::new(
                    "When converted, every column takes the real block of the game whose colour is closest to the graph's, \
                     two neighbouring blocks mixing where the colour lies between them.",
                )
                .size(11.0)
                .color(pal.weak),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use massif_core::graph::Graph;

    #[test]
    fn v_views_the_selected_node_and_goes_back_to_the_output() {
        let mut ed = GraphEditor::default();
        let r0 = ed.revision;
        // Selecting does not change what is viewed.
        ed.selected = Some(3);
        assert_eq!(ed.pinned, None);
        ed.view_key();
        assert_eq!(ed.pinned, Some(3));
        assert!(ed.revision > r0, "a new preview is asked for");
        ed.view_key();
        assert_eq!(ed.pinned, None);
        // Viewing another node, then V with nothing selected: the output.
        ed.toggle_view(5);
        ed.selected = None;
        ed.view_key();
        assert_eq!(ed.pinned, None);
    }

    #[test]
    fn the_viewport_shows_the_selected_node_on_the_closest_relief() {
        let mut g = Graph::default();
        let hills = g.add(
            NodeKind::Noise {
                style: NoiseStyle::Fbm,
                scale: 200.0,
                octaves: 4,
                roughness: 0.5,
                height: 100.0,
                base: 50.0,
                seed: 1,
            },
            [0.0; 2],
        );
        let slope = g.add(
            NodeKind::SelectSlope {
                min: 10.0,
                max: 90.0,
                falloff: 2.0,
            },
            [0.0; 2],
        );
        let paint = g.add(
            NodeKind::AutoColor {
                palette: AutoPalette::Temperate,
                snow_line: 140.0,
                rock_slope: 30.0,
                seed: 1,
            },
            [0.0; 2],
        );
        let out = g.add(NodeKind::HeightOutput, [0.0; 2]);
        let color_out = g.add(
            NodeKind::ColorOutput {
                blocks: true,
                palette: BlockPalette::Natural,
            },
            [0.0; 2],
        );
        g.connect(hills, 0, slope, 0);
        g.connect(hills, 0, paint, 0);
        g.connect(hills, 0, out, 0);
        g.connect(paint, 0, color_out, 0);
        let extent = Extent {
            x0: 0.0,
            z0: 0.0,
            size: 512.0,
            n: 32,
        };
        let ids: Vec<NodeId> = g.nodes.iter().map(|n| n.id).collect();
        let eval = g.evaluate(&extent, &ids, true, &AtomicBool::new(false)).unwrap();
        // Nothing selected: the output, in its colours.
        let (h, paint_of, shown) = pick_view(&g, &eval, None).unwrap();
        assert_eq!(shown, (hills, 0));
        assert!(matches!(paint_of, Paint::Color(_)));
        assert_eq!(h.n, 32);
        // A mask: shown on the relief before it.
        let (_, p, shown) = pick_view(&g, &eval, Some((slope, 0))).unwrap();
        assert!(matches!(p, Paint::Mask(_)) && shown == (slope, 0));
        // A colour node, and the colour output (which shows what it gets).
        let (_, p, _) = pick_view(&g, &eval, Some((paint, 0))).unwrap();
        assert!(matches!(p, Paint::Color(_)));
        let (_, _, shown) = pick_view(&g, &eval, Some((color_out, 0))).unwrap();
        assert_eq!(shown, (paint, 0));
        // The data for the viewport, with its light.
        let (h, p, _) = pick_view(&g, &eval, Some((slope, 0))).unwrap();
        let data = view_data(&h, &p, &extent, 2.0, &[]);
        assert_eq!((data.colors.len(), data.light.len()), (32 * 32, 32 * 32));
        // Thumbnails of each kind.
        for (key, value) in &eval.values {
            let img = thumbnail(value, g.output_type(key.0, key.1), extent.spacing());
            assert_eq!(img.size, [THUMB_PX, THUMB_PX]);
        }
    }
}
