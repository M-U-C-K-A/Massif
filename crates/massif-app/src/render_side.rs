//! The Render side: the open map path-traced on the graphics card
//! (`massif_render`), a sample a pixel a frame, added up while the view
//! stays still. Its panels are editors of the layout as the other sides'
//! are: the viewport, the sun and sky, the camera, the output and the
//! timeline of an animation, each in whatever area (or floating window)
//! the user puts it.
//!
//! The map's chunks are built in the background by the exporter, nearest
//! the view first, and put on the graphics card as they come, so the
//! image fills in from the middle out; an edit made on the World side
//! builds its tiles again when the Render side is shown next.

use crate::doc::Document;
use crate::theme::{self, Icon, Palette};
use crate::widgets::{Field, row};
use egui::{Align2, Color32, FontId, PointerButton, Rect, RichText, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use massif_core::animation::{Animation, Pose};
use massif_core::coords::TilePos;
use massif_render::map_source::{Extent, MapBuilder, target_for, tiles_by_distance};
use massif_render::{Camera, Light, Materials, Renderer};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

/// Time a frame spends putting built chunks on the graphics card.
const UPLOAD_BUDGET: Duration = Duration::from_millis(6);
/// Seconds after the view last moved that it is drawn sharp again.
const STILL_AFTER: f64 = 0.2;
/// Samples a pixel the image stops at.
const MOST_SAMPLES: u32 = 2048;

/// The sizes a still is saved at: the view's, or one of the usual ones.
const STILL_SIZES: [(&str, Option<[u32; 2]>); 4] = [(tn!("The view's"), None), ("HD", Some([1920, 1080])), ("4K", Some([3840, 2160])), ("8K", Some([7680, 4320]))];
/// Pixels a frame adds a sample to while a still is drawn: about a view's
/// worth, a short job for the graphics card whatever the still's size.
const STILL_BAND: u32 = 2_000_000;

/// A still being drawn: where it goes, at what size and to how many
/// samples a pixel, from the camera and in the light it was asked in, and
/// the next row of its next band.
struct Still {
    path: PathBuf,
    size: [u32; 2],
    samples: u32,
    camera: Camera,
    light: Light,
    /// Denoised before it is saved.
    denoise: bool,
    row: u32,
    started: Instant,
}

/// What a frame of an animation gives its writer: the samples to denoise
/// (Open Image Denoise, on the writer's thread), or the image.
enum FrameData {
    Samples(massif_render::gpu::Samples),
    Image(u32, u32, Vec<u8>),
}

/// An animation being drawn along the Timeline's camera path, a still a
/// frame, each written (MP4 or PNG images) on a thread of its own.
struct Movie {
    anim: Animation,
    /// The frames, the first drawn (a PNG sequence goes on where it was
    /// left), the one being drawn.
    frames: u32,
    first: u32,
    frame: u32,
    size: [u32; 2],
    samples: u32,
    denoise: bool,
    light: Light,
    time: Time,
    tx: Option<std::sync::mpsc::SyncSender<(u32, FrameData)>>,
    written: Arc<std::sync::atomic::AtomicU32>,
    result: Receiver<Result<String, String>>,
    started: Instant,
}

impl Movie {
    /// The still of frame `frame`: the camera and the hour the path has
    /// then.
    fn still(&self, frame: u32) -> Option<Still> {
        let t = frame as f32 / self.anim.fps.max(1) as f32;
        let pose = self.anim.sample(t)?;
        let mut light = self.light;
        light.set_time(pose.hour, self.time.day, self.time.latitude, self.time.north);
        let mut time = Time { hour: pose.hour, ..self.time };
        apply_tracks(&self.anim, t, &mut light, &mut time);
        light.time = t;
        let camera = Orbit::between(pose.eye, pose.target).camera(pose.fov.clamp(10.0, 100.0));
        Some(Still { path: PathBuf::new(), size: self.size, samples: self.samples, camera, light, denoise: self.denoise, row: 0, started: Instant::now() })
    }

    /// Frames drawn and written so far, of those to draw this time.
    fn progress(&self) -> (u32, u32) {
        (self.written.load(std::sync::atomic::Ordering::Relaxed), self.frames - self.first)
    }
}

/// How an animation is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MovieFormat {
    Mp4,
    Png,
}

/// The time and place the sun is put by.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Time {
    /// Local solar time, hours.
    pub(crate) hour: f32,
    /// Day of the year.
    day: f32,
    /// Degrees, north positive.
    latitude: f32,
    /// How far the map's north is turned from the world's, degrees.
    north: f32,
}

impl Default for Time {
    fn default() -> Self {
        Self { hour: 15.0, day: 172.0, latitude: 45.0, north: 0.0 }
    }
}

/// The ready-made looks of the sky, by name.
const PRESETS: [&str; 7] = [tn!("Clear noon"), tn!("Fair weather"), tn!("Golden hour"), tn!("Blue hour"), tn!("Foggy morning"), tn!("Night"), tn!("Valley mist")];

/// Put a ready-made look of the sky on `light`, its sun by `time` (the
/// clouds from the sea level `sea`, the fog and the mist lying on the floor
/// of the map's valleys `valley`, whatever the map's height).
///
/// A look sets everything of the sky it owns (the hour, the sun's and the
/// sky's strength, the air, the haze, the fog, the clouds, the exposure),
/// and leaves the rest (the lens, the bounces, the grass, the water) as it
/// was.
fn preset(k: usize, light: &mut Light, time: &mut Time, sea: f32, valley: f32) {
    let d = Light::default();
    let base = Light {
        sun_strength: d.sun_strength,
        sun_tint: d.sun_tint,
        sky_strength: d.sky_strength,
        air: d.air,
        haze: d.haze,
        haze_g: d.haze_g,
        fog: 0.0,
        fog_height: valley + 8.0,
        fog_falloff: d.fog_falloff,
        fog_g: d.fog_g,
        moon: d.moon,
        cloud_cover: 0.0,
        cloud_height: sea + 1200.0,
        cloud_thickness: d.cloud_thickness,
        cloud_density: d.cloud_density,
        cloud_size: d.cloud_size,
        exposure: d.exposure,
        ..*light
    };
    let (hour, l) = match k {
        0 => (12.5, base),
        1 => (14.5, Light { cloud_cover: 0.42, ..base }),
        2 => (18.8, Light { haze: 4.0, cloud_cover: 0.2, exposure: 2.5, ..base }),
        // Twilight and moonlight are a thousand and some hundred thousand
        // times dimmer than day: exposed for them, as a camera would be.
        3 => (20.1, Light { haze: 2.0, exposure: 60.0, ..base }),
        // Fog one sees 300 blocks through, lying in the low ground.
        4 => (7.5, Light { fog: 3.912 / 300.0, fog_height: valley + 6.0, fog_falloff: 10.0, haze: 1.5, exposure: 0.8, ..base }),
        // A thin bright mist in the low ground at sunrise, the air clear
        // above it.
        6 => (6.6, Light { fog: 3.912 / 120.0, fog_height: valley + 2.0, fog_falloff: 6.0, fog_g: 0.75, haze: 1.2, exposure: 1.6, ..base }),
        5 => (23.0, Light { exposure: 40.0, moon: 1.0 / 150.0, ..base }),
        _ => (12.5, base),
    };
    *light = l;
    time.hour = hour;
    light.set_time(time.hour, time.day, time.latitude, time.north);
}

/// The camera of the Render side: turning about a point.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Orbit {
    target: [f64; 3],
    yaw: f32,
    pitch: f32,
    distance: f64,
}

impl Orbit {
    fn camera(&self, fov: f32) -> Camera {
        Camera::orbit(self.target, self.yaw, self.pitch, self.distance, fov.to_radians())
    }

    /// Where the eye stands.
    fn eye(&self) -> [f64; 3] {
        self.camera(50.0).eye
    }

    /// The camera at `eye` looking along `yaw` and `pitch`, turning about
    /// the point `distance` ahead.
    fn from_eye(eye: [f64; 3], yaw: f32, pitch: f32, distance: f64) -> Orbit {
        let f = Camera { eye, yaw, pitch, fov_y: 1.0 }.forward();
        let target = [eye[0] + f.x as f64 * distance, eye[1] + f.y as f64 * distance, eye[2] + f.z as f64 * distance];
        Orbit { target, yaw, pitch, distance }
    }

    /// Turned by `dyaw` and `dpitch` about `pivot`, the view's direction
    /// turning as much: Blender's orbit about the point under the pointer
    /// (its "Orbit Around Selection" with "Auto Depth"), the pivot staying
    /// where it is on the screen.
    fn orbit_about(&mut self, pivot: [f64; 3], dyaw: f32, dpitch: f32) {
        let eye = self.eye();
        let pitch = (self.pitch + dpitch).clamp(-1.55, 1.55);
        let dpitch = pitch - self.pitch;
        let yaw = self.yaw + dyaw;
        let v = glam::DVec3::new(eye[0] - pivot[0], eye[1] - pivot[1], eye[2] - pivot[2]);
        // The yaw turns about the vertical (a turn by -dyaw about +y adds
        // dyaw to the yaw), the pitch about the turned view's right.
        let v = glam::DQuat::from_rotation_y(-dyaw as f64) * v;
        let b = Camera { eye, yaw, pitch: self.pitch, fov_y: 1.0 }.basis();
        let right = glam::DVec3::new(b.right.x as f64, b.right.y as f64, b.right.z as f64);
        let v = glam::DQuat::from_axis_angle(right, dpitch as f64) * v;
        *self = Orbit::from_eye([pivot[0] + v.x, pivot[1] + v.y, pivot[2] + v.z], yaw, pitch, self.distance);
    }

    /// Closer to `point` (or farther) by `factor` of the way, the view's
    /// direction kept: Blender's "Zoom to Mouse Position", `point` the
    /// surface under the pointer, so that it stays under it.
    fn zoom_toward(&mut self, point: [f64; 3], factor: f64) {
        let eye = self.eye();
        let eye = [point[0] + (eye[0] - point[0]) * factor, point[1] + (eye[1] - point[1]) * factor, point[2] + (eye[2] - point[2]) * factor];
        *self = Orbit::from_eye(eye, self.yaw, self.pitch, (self.distance * factor).max(0.5));
    }

    /// Slid across the view by `dx`, `dy` pixels (y down), `per_pixel`
    /// blocks a pixel at the depth of the point held: that point follows
    /// the pointer.
    fn pan(&mut self, dx: f64, dy: f64, per_pixel: f64) {
        let b = self.camera(50.0).basis();
        let (r, u) = (b.right, b.up);
        let d = [(-r.x as f64 * dx + u.x as f64 * dy) * per_pixel, (-r.y as f64 * dx + u.y as f64 * dy) * per_pixel, (-r.z as f64 * dx + u.z as f64 * dy) * per_pixel];
        for (t, d) in self.target.iter_mut().zip(d) {
            *t += d;
        }
    }

    /// Turning about `target` from `eye`.
    fn between(eye: [f64; 3], target: [f64; 3]) -> Orbit {
        let d = [target[0] - eye[0], target[1] - eye[1], target[2] - eye[2]];
        let distance = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt().max(1e-3);
        let (yaw, pitch) = (d[0].atan2(-d[2]) as f32, (d[1] / distance).clamp(-1.0, 1.0).asin() as f32);
        Orbit { target, yaw, pitch, distance }
    }
}

/// What the renderer holds of the open map.
struct Scene {
    renderer: Renderer,
    materials: Arc<Materials>,
    builder: Option<MapBuilder>,
    /// Tiles on the graphics card, and all of the map's.
    built: HashSet<TilePos>,
    tiles: usize,
    started: Instant,
    /// How long the whole map took to build, once built.
    took: Option<Duration>,
    /// The map is larger than the graphics card's buffers.
    full: bool,
    /// The whole map's chunks; the scene holds them all, or, loading
    /// around the view, those within the reach of a point (the middle, the
    /// reach): see [`render_reach`].
    map: Extent,
    around: Option<([f64; 2], f64)>,
    generation: u64,
    catalogue: Option<usize>,
    /// Built with blades of grass over the grass blocks.
    blades: bool,
    dim: usize,
}

/// Where the Timeline panel is: its playhead, whether it plays, the
/// keyframe picked, a keyframe being dragged.
#[derive(Default)]
struct Playback {
    /// Seconds.
    playhead: f32,
    playing: bool,
    selected: Option<usize>,
    /// The keyframe dragged, and where the pointer took it from.
    dragging: Option<usize>,
    /// The playhead the view was last put at: moved since, the view follows.
    applied: Option<f32>,
    /// Home pressed: the view frames the whole map.
    frame_all: bool,
}

pub struct RenderSide {
    scene: Option<Scene>,
    orbit: Option<Orbit>,
    light: Light,
    time: Time,
    /// The vertical field of view, degrees.
    fov: f32,
    preset: usize,
    texture: Option<egui::TextureId>,
    /// Tiles edited since they were built.
    dirty: HashSet<TilePos>,
    /// When the builder last started again on edits: during a stroke, at
    /// most every [`crate::diorama::RESTART_EVERY`].
    restarted: Option<Instant>,
    moving_until: f64,
    last: Option<(Orbit, Light, [u32; 2], f32)>,
    /// When the image last started adding samples up, and its samples
    /// then: how many it adds a second.
    since: Option<(Instant, u32)>,
    /// The size a still is saved at (in [`STILL_SIZES`]) and its samples
    /// a pixel; the one being drawn; the one being written; what became of
    /// the last.
    still_size: usize,
    still_samples: u32,
    /// The view shown denoised, and the stills saved so.
    denoise: bool,
    still_denoise: bool,
    /// Open Image Denoise on the view: the image it is working on (the
    /// renderer's round of samples), the round last seen, the samples last
    /// denoised, and whether its image is the one shown.
    intel: Option<(u64, Receiver<Denoised>)>,
    round: u64,
    intel_at: u32,
    intel_shown: bool,
    /// The grass drawn as blades of grass (detailed), not as the game's.
    blades: bool,
    still: Option<Still>,
    /// The animation being drawn, how it is written and with how many
    /// samples a frame, and what became of the last.
    movie: Option<Movie>,
    movie_format: MovieFormat,
    movie_samples: u32,
    movie_saved: Option<Result<String, String>>,
    writing: Option<Receiver<Result<String, String>>>,
    saved: Option<Result<String, String>>,
    /// The size the image's texture was shown at.
    texture_size: [u32; 2],
    /// The size of the view, in pixels, when it was last drawn.
    view_px: [u32; 2],
    /// The area the view was drawn in, and in which pass: only one area
    /// draws it a pass, the others say where it is.
    drawn: Option<(u64, u64)>,
    playback: Playback,
    /// How fast W A S D walk, times the distance ahead (the wheel, while
    /// walking); the keys held, by their places; the point a drag turns
    /// about (or slides) and how far it is; `.` pressed: in close there.
    walk_factor: f64,
    /// The minimap: drawn at the document's revision (its texture and the
    /// blocks it covers), being drawn, when it was last asked.
    minimap: Option<(u64, egui::TextureHandle, [f64; 4])>,
    minimap_job: Option<(u64, Receiver<Option<crate::render_minimap::Picture>>)>,
    minimap_asked: Option<Instant>,
    /// Where the minimap was last drawn.
    minimap_shown: Option<Rect>,
    /// The pass a setting's diamond was last under the pointer in (I keys
    /// the setting then, not the camera); the samples the view adds up to.
    key_hover: u64,
    most_samples: u32,
    /// The render camera, as Blender's camera object, apart from the view
    /// while it is not locked to it: what stills, keyframes and the video
    /// are taken from.
    lock_camera: bool,
    camera: Option<Orbit>,
    held: HashSet<egui::Key>,
    grab: Option<([f64; 3], f64)>,
    focus_here: bool,
}

impl Default for RenderSide {
    fn default() -> Self {
        let mut light = Light::default();
        let time = Time::default();
        light.set_time(time.hour, time.day, time.latitude, time.north);
        Self {
            scene: None,
            orbit: None,
            light,
            time,
            fov: 50.0,
            preset: usize::MAX,
            texture: None,
            dirty: HashSet::new(),
            restarted: None,
            moving_until: 0.0,
            last: None,
            since: None,
            still_size: 2,
            still_samples: 256,
            denoise: true,
            still_denoise: true,
            intel: None,
            round: 0,
            intel_at: 0,
            intel_shown: false,
            blades: false,
            still: None,
            movie: None,
            movie_format: if crate::video::mp4_supported() { MovieFormat::Mp4 } else { MovieFormat::Png },
            movie_samples: 64,
            movie_saved: None,
            writing: None,
            saved: None,
            texture_size: [0; 2],
            view_px: [0; 2],
            drawn: None,
            playback: Playback::default(),
            walk_factor: 1.0,
            minimap: None,
            minimap_job: None,
            minimap_asked: None,
            minimap_shown: None,
            key_hover: 0,
            most_samples: MOST_SAMPLES,
            lock_camera: true,
            camera: None,
            held: HashSet::new(),
            grab: None,
            focus_here: false,
        }
    }
}

/// An image denoised on the processor: its light (linear) and exposure.
type Denoised = Result<(Vec<f32>, massif_render::color::Tone), String>;

/// The samples a pixel at which the view is denoised by Open Image
/// Denoise (and at the last, where it stops).
const INTEL_AT: [u32; 6] = [1, 4, 16, 64, 256, 1024];

/// A section's title over its well, as Blender's panels: a click folds the
/// well away (and back), kept per section.
fn title(ui: &mut Ui, text: &str) {
    let pal = Palette::of(ui);
    let id = ui.scope_id().with(("render fold", text));
    let open = ui.data(|d| d.get_temp::<bool>(id)).unwrap_or(true);
    let arrow = if open { "▾" } else { "▸" };
    let response = ui.add(egui::Label::new(RichText::new(format!("{arrow} {text}")).size(12.0).color(pal.weak)).sense(Sense::click()));
    if response.clicked() {
        ui.data_mut(|d| d.insert_temp(id, !open));
    }
    ui.data_mut(|d| d.insert_temp(egui::Id::unique("render fold next"), !open));
}

/// The well under a title, unless the title folded it away.
fn well<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> Option<R> {
    let folded = ui.data_mut(|d| d.remove_temp::<bool>(egui::Id::unique("render fold next"))).unwrap_or(false);
    (!folded).then(|| crate::widgets::well(ui, add))
}

/// A keyframe asked of a setting.
enum KeyEdit {
    Set(&'static str, f32),
    Unset(&'static str),
}

/// The keyframe diamonds of the settings an animation can change, as
/// Blender's: hollow when the setting is not keyed, outlined in the accent
/// when it is animated, filled when it has a key at the playhead. A click,
/// or I with the pointer over the setting, keys it there; a click on a key
/// or Alt+I takes it away.
struct Keyer {
    anim: Animation,
    at: f32,
    edits: Vec<KeyEdit>,
    hovered: bool,
}

impl Keyer {
    fn new(doc: &Document, at: f32) -> Self {
        Keyer { anim: doc.world.meta.animation.clone().unwrap_or_default(), at, edits: Vec::new(), hovered: false }
    }

    fn diamond(&mut self, ui: &mut Ui, name: &'static str, value: f32, field: &egui::Response) {
        let pal = Palette::of(ui);
        let here = self.anim.has_value_key(name, self.at);
        let animated = self.anim.tracks.contains_key(name);
        let (rect, response) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::click());
        let c = rect.center();
        let r = 4.5;
        let shape = vec![c + vec2(0.0, -r), c + vec2(r, 0.0), c + vec2(0.0, r), c + vec2(-r, 0.0)];
        let (fill, line) = if here { (pal.accent, pal.accent) } else if animated { (Color32::TRANSPARENT, pal.accent) } else { (Color32::TRANSPARENT, pal.weak) };
        ui.painter().add(egui::Shape::convex_polygon(shape, fill, Stroke::new(1.2, line)));
        let response = response.on_hover_text(tr!("Keyframe this setting at the playhead (I over the setting; Alt+I removes the key)"));
        if response.clicked() {
            self.edits.push(if here { KeyEdit::Unset(name) } else { KeyEdit::Set(name, value) });
        }
        if (response.hovered() || field.hovered()) && !ui.ctx().egui_wants_keyboard_input() {
            self.hovered = true;
            ui.input_mut(|i| {
                if i.consume_key(egui::Modifiers::ALT, egui::Key::I) {
                    self.edits.push(KeyEdit::Unset(name));
                } else if i.consume_key(egui::Modifiers::NONE, egui::Key::I) {
                    self.edits.push(KeyEdit::Set(name, value));
                }
            });
        }
    }

    /// The keys asked, into the map's animation.
    fn apply(self, doc: &mut Document) {
        if self.edits.is_empty() {
            return;
        }
        let anim = doc.world.meta.animation.get_or_insert_default();
        for e in self.edits {
            match e {
                KeyEdit::Set(name, v) => anim.key_value(name, self.at, v),
                KeyEdit::Unset(name) => {
                    anim.unkey_value(name, self.at);
                }
            }
        }
        doc.mark_changed();
    }
}

/// The setting an animation track of `name` changes.
fn animated<'a>(light: &'a mut Light, time: &'a mut Time, name: &str) -> Option<&'a mut f32> {
    Some(match name {
        "hour" => &mut time.hour,
        "sun" => &mut light.sun_strength,
        "sky" => &mut light.sky_strength,
        "air" => &mut light.air,
        "haze" => &mut light.haze,
        "fog" => &mut light.fog,
        "fog_height" => &mut light.fog_height,
        "cloud_cover" => &mut light.cloud_cover,
        "cloud_height" => &mut light.cloud_height,
        "cloud_density" => &mut light.cloud_density,
        "cloud_size" => &mut light.cloud_size,
        "wind" => &mut light.wind[0],
        "exposure" => &mut light.exposure,
        "aperture" => &mut light.aperture,
        "focus" => &mut light.focus,
        _ => return None,
    })
}

/// Put the keyed settings of `anim` at `t` on `light` and `time`.
fn apply_tracks(anim: &Animation, t: f32, light: &mut Light, time: &mut Time) {
    let hour = time.hour;
    for name in anim.tracks.keys() {
        if let Some(v) = anim.value(name, t)
            && let Some(p) = animated(light, time, name)
        {
            *p = v;
        }
    }
    if time.hour != hour {
        light.set_time(time.hour, time.day, time.latitude, time.north);
    }
}

impl RenderSide {
    /// Whether the map's chunks are being built.
    #[cfg(test)]
    pub fn building(&self) -> bool {
        self.scene.as_ref().is_none_or(|s| s.builder.is_some())
    }

    /// Tiles on the graphics card, and those the scene holds once built.
    #[cfg(test)]
    pub fn tiles_built(&self) -> (usize, usize) {
        self.scene.as_ref().map_or((0, 0), |s| (s.built.len(), s.tiles))
    }

    /// Samples a pixel added up in the image.
    #[cfg(test)]
    pub fn samples(&self) -> u32 {
        self.scene.as_ref().map_or(0, |s| s.renderer.samples)
    }

    /// Forget the scene, the camera kept: it is built afresh (tests).
    #[cfg(test)]
    pub fn rebuild(&mut self) {
        self.scene = None;
    }

    /// The image as the renderer holds it (tests).
    #[cfg(test)]
    pub fn image(&self) -> Option<(u32, u32, Vec<u8>)> {
        self.scene.as_ref()?.renderer.read_image()
    }

    /// Draw the map's animation now into `target`, without asking where
    /// (tests): an MP4, or PNG images when `png`.
    #[cfg(test)]
    pub fn render_movie(&mut self, anim: Animation, target: PathBuf, png: bool, size: [u32; 2], samples: u32) {
        self.movie_format = if png { MovieFormat::Png } else { MovieFormat::Mp4 };
        self.movie_samples = samples;
        self.launch_movie("frames", anim, target, size);
    }

    /// Whether an animation is being drawn or written, and what became of
    /// the last (tests).
    #[cfg(test)]
    pub fn movie_state(&self) -> (bool, Option<&Result<String, String>>) {
        (self.movie.is_some(), self.movie_saved.as_ref())
    }

    /// Draw a still now, without asking where: for tests.
    #[cfg(test)]
    pub fn save_still(&mut self, path: PathBuf, size: [u32; 2], samples: u32) {
        let fov = self.fov;
        if let Some(orbit) = self.shot() {
            self.still = Some(Still { path, size, samples, camera: orbit.camera(fov), light: self.light, denoise: self.still_denoise, row: 0, started: Instant::now() });
        }
    }

    /// Whether a still is being drawn or written.
    #[cfg(test)]
    pub fn saving(&self) -> bool {
        self.still.is_some() || self.writing.is_some()
    }

    /// What became of the last still: its file's name, or why it failed.
    #[cfg(test)]
    pub fn saved(&self) -> Option<&Result<String, String>> {
        self.saved.as_ref()
    }

    /// Put the playhead at `time` (tests, as a scrub would).
    #[cfg(test)]
    pub fn scrub(&mut self, time: f32) {
        self.playback.playhead = time;
    }

    /// Put look `k` on as the panel does, or (None) note the settings
    /// changed by hand since `was`; the look now (tests).
    #[cfg(test)]
    pub fn sky_edit(&mut self, k: Option<usize>, was: (Light, Time), doc: &mut Document) -> usize {
        self.after_sky(k, was.0, was.1, doc);
        self.preset
    }

    /// The light and the time now (tests).
    #[cfg(test)]
    pub fn sky(&self) -> (Light, Time) {
        (self.light, self.time)
    }

    /// The light, to change in tests.
    #[cfg(test)]
    pub fn light_mut(&mut self) -> &mut Light {
        &mut self.light
    }

    /// Put the camera at `eye` looking at `target` (tests).
    #[cfg(test)]
    pub fn set_view(&mut self, eye: [f64; 3], target: [f64; 3]) {
        self.orbit = Some(Orbit::between(eye, target));
    }

    /// Whether the minimap has been drawn, and where it is (tests).
    #[cfg(test)]
    pub fn has_minimap(&self) -> bool {
        self.minimap.is_some()
    }

    #[cfg(test)]
    pub fn minimap_rect(&self) -> Option<Rect> {
        self.minimap_shown
    }

    /// The Timeline's playhead, seconds (tests).
    #[cfg(test)]
    pub fn playhead(&self) -> f32 {
        self.playback.playhead
    }

    /// Where the camera stands and looks, its angle and the hour (tests).
    #[cfg(test)]
    pub fn pose_now(&self) -> Option<Pose> {
        self.pose()
    }

    /// Tiles the map changed in: built again when the side is shown.
    pub fn invalidate(&mut self, tiles: &[TilePos]) {
        if self.scene.is_some() {
            self.dirty.extend(tiles.iter().copied());
        }
    }

    /// Where stills, keyframes and the video are taken from: the view, or
    /// the camera apart from it while it is not locked to the view.
    fn shot(&self) -> Option<Orbit> {
        if self.lock_camera { self.orbit } else { self.camera.or(self.orbit) }
    }

    /// The camera and the sun now, as a keyframe holds them.
    fn pose(&self) -> Option<Pose> {
        let orbit = self.shot()?;
        Some(Pose { eye: orbit.camera(self.fov).eye, target: orbit.target, fov: self.fov, hour: self.time.hour })
    }

    /// Put the camera and the sun as `pose` has them.
    fn put(&mut self, pose: &Pose) {
        self.orbit = Some(Orbit::between(pose.eye, pose.target));
        self.fov = pose.fov.clamp(10.0, 100.0);
        if self.time.hour != pose.hour {
            self.time.hour = pose.hour;
            self.light.set_time(self.time.hour, self.time.day, self.time.latitude, self.time.north);
        }
    }

    /// The Render Viewport: the map path-traced in `ui`'s whole room. Area
    /// `area` draws it, unless another did in this pass.
    pub fn viewport_ui(&mut self, ui: &mut Ui, area: u64, frame: &eframe::Frame, doc: &mut Document, catalogue: Option<Arc<massif_assets::Catalogue>>) {
        let pal = Palette::of(ui);
        let view = ui.available_rect_before_wrap();
        ui.painter().rect_filled(view, 0.0, pal.canvas);
        let pass = ui.ctx().cumulative_pass_nr();
        if self.drawn.is_some_and(|(p, a)| p == pass && a != area) {
            ui.painter().text(view.center(), Align2::CENTER_CENTER, tr!("The render is shown in another area"), FontId::proportional(13.0), pal.weak);
            return;
        }
        self.drawn = Some((pass, area));
        let Some(render_state) = frame.wgpu_render_state() else {
            ui.painter().text(view.center(), Align2::CENTER_CENTER, tr!("Rendering needs the wgpu renderer"), FontId::proportional(13.0), pal.weak);
            return;
        };
        let (device, queue) = (&render_state.device, &render_state.queue);
        let cat_key = catalogue.as_ref().map(|c| Arc::as_ptr(c) as usize);
        // A new map (or the game's textures read since): a new scene.
        if self.scene.as_ref().is_some_and(|s| s.generation != doc.generation || s.catalogue != cat_key || s.dim != doc.dim) {
            self.scene = None;
            self.orbit = None;
            self.playback = Playback::default();
        }
        // Loading around the view: the camera gone far from the part of the
        // map the scene holds (or the setting changed), a scene around it
        // now, not while an image is drawn or saved.
        let reach = render_reach();
        if let (Some(s), Some(orbit)) = (&self.scene, &self.orbit)
            && self.still.is_none()
            && self.writing.is_none()
            && match (s.around, reach) {
                (Some((c, r)), Some(reach)) => r != reach || (c[0] - orbit.target[0]).hypot(c[1] - orbit.target[2]) > reach / 2.0,
                (None, None) => false,
                _ => true,
            }
        {
            self.scene = None;
        }
        // The grass drawn the other way: the scene built again, the camera
        // where it is.
        if self.scene.as_ref().is_some_and(|s| s.blades != self.blades) && self.still.is_none() && self.writing.is_none() {
            self.scene = None;
        }
        // Tiles edited out of the map the scene was made for (the map grew):
        // a scene of the whole map as it is now.
        if let Some(s) = &self.scene
            && self.dirty.iter().any(|p| {
                let (x, z) = (p.x * massif_render::map_source::TILE_CHUNKS, p.z * massif_render::map_source::TILE_CHUNKS);
                x < s.map.x0 || z < s.map.z0 || x >= s.map.x0 + s.map.width as i32 || z >= s.map.z0 + s.map.depth as i32
            })
            && self.still.is_none()
            && self.writing.is_none()
        {
            self.scene = None;
        }
        if self.scene.is_none() {
            let target = target_for(catalogue.as_ref().map(|c| c.version.as_str()));
            let Some(map) = Extent::of_map(&doc.world, doc.dim, &target) else {
                ui.painter().text(view.center(), Align2::CENTER_CENTER, tr!("Nothing to render: the map is empty"), FontId::proportional(13.0), pal.weak);
                return;
            };
            // A map with a camera path opens where it starts.
            if let Some(pose) = doc.world.meta.animation.as_ref().and_then(|a| a.sample(0.0)) {
                self.put(&pose);
                self.playback.applied = Some(0.0);
            }
            let orbit = *self.orbit.get_or_insert_with(|| framing(&map, reach, doc.world.meta.sea_level as f64));
            let around = reach.map(|r| ([orbit.target[0], orbit.target[2]], r));
            // Around a point off the map: the map's nearest part.
            let extent = match around {
                Some(([x, z], r)) => map.around(x, z, r).or_else(|| {
                    let c = map.center();
                    map.around(c[0], c[2], r)
                }),
                None => Some(map),
            }
            .unwrap_or(map);
            let materials = Arc::new(Materials::new(catalogue.clone(), device.limits().max_texture_array_layers as usize));
            materials.set_blades(self.blades);
            let tiles = wanted(doc, &extent, orbit.target);
            let count = tiles.len();
            let builder = MapBuilder::start(doc.world.copy_unfrozen(), doc.dim, target, materials.clone(), tiles);
            self.scene = Some(Scene {
                renderer: Renderer::new(device, queue, extent),
                materials,
                builder: Some(builder),
                built: HashSet::new(),
                tiles: count,
                started: Instant::now(),
                took: None,
                full: false,
                map,
                around,
                generation: doc.generation,
                catalogue: cat_key,
                blades: self.blades,
                dim: doc.dim,
            });
            self.dirty.clear();
            self.last = None;
            // A new renderer, a new image: shown from it even at the same size.
            self.texture_size = [0; 2];
            // The fog and the clouds where they belong on this map.
            let sea = doc.world.meta.sea_level as f32;
            self.light.fog_height = sea + 8.0;
            self.light.cloud_height = sea + 1200.0;
        }
        let scene = self.scene.as_mut().expect("made above");
        scene.renderer.sea_level = doc.world.meta.sea_level as f64;
        let orbit = self.orbit.get_or_insert_with(|| framing(&scene.map, reach, doc.world.meta.sea_level as f64));

        // Edits since: their tiles first, then those not built yet, from a
        // snapshot of the map as it is now.
        let due = !doc.history.is_recording() || self.restarted.is_none_or(|t| t.elapsed() >= crate::diorama::RESTART_EVERY);
        if !self.dirty.is_empty() && due {
            profiling::scope!("Render: edits");
            self.restarted = Some(Instant::now());
            let mut first: Vec<TilePos> = self.dirty.drain().collect();
            let near = |p: &TilePos| {
                let (x, z) = ((p.x as f64 + 0.5) * 128.0 - orbit.target[0], (p.z as f64 + 0.5) * 128.0 - orbit.target[2]);
                x * x + z * z
            };
            first.sort_by(|a, b| near(a).total_cmp(&near(b)));
            let again: HashSet<TilePos> = first.iter().copied().collect();
            let rest: Vec<TilePos> = wanted(doc, &scene.renderer.extent(), orbit.target).into_iter().filter(|p| !scene.built.contains(p) && !again.contains(p)).collect();
            for p in &first {
                scene.built.remove(p);
            }
            first.extend(rest);
            let target = target_for(catalogue.as_ref().map(|c| c.version.as_str()));
            scene.builder = Some(MapBuilder::start(doc.world.copy_unfrozen(), doc.dim, target, scene.materials.clone(), first));
            scene.took = None;
            scene.started = Instant::now();
        }

        // Built chunks onto the graphics card, a few milliseconds' worth.
        let started = Instant::now();
        let mut changed = false;
        if let Some(builder) = &mut scene.builder {
            while started.elapsed() < UPLOAD_BUDGET {
                let Some(built) = builder.take() else { break };
                scene.built.insert(built.tile);
                if !scene.renderer.put(built.chunks) {
                    scene.full = true;
                }
                changed = true;
            }
            if builder.finished() {
                if let Some(e) = builder.error() {
                    log::warn!("render scene: {e}");
                }
                scene.builder = None;
                scene.took = Some(scene.started.elapsed());
            }
            ui.ctx().request_repaint();
        }
        scene.renderer.sync_materials(&scene.materials);

        // The camera, as Blender's: drag (or the middle button, or Alt with
        // the left) turns about the point under the pointer where the drag
        // began; Shift with them slides, the point held following the
        // pointer; the wheel and a pinch come closer to the point under
        // the pointer; the right button held looks around from where the
        // camera stands, and W A S D, Q E walk (by their places on the
        // keyboard: Z Q S D, A E on an AZERTY one), Shift faster, Alt
        // slower, the wheel while walking setting the speed. Every speed
        // goes by the distance to what is under the pointer or ahead, so
        // that two blocks over the grass and two kilometres over the map
        // are both in hand. Double-click: in close on the point there (on
        // the sky: the whole map).
        let sense = if self.still.is_some() { Sense::hover() } else { Sense::click_and_drag() };
        let response = ui.interact(view, egui::Id::unique(("massif render view", area)), sense);
        let now = ui.input(|i| i.time);
        let map = scene.map;
        let span = map.width.max(map.depth) as f64 * 16.0;
        let renderer = &scene.renderer;
        let fov = self.fov;
        // The surface under a point of the view: the ray there, the point
        // and how far.
        let under = |orbit: &Orbit, at: egui::Pos2| -> ([f64; 3], [f64; 3], Option<f64>) {
            let camera = orbit.camera(fov);
            let b = camera.basis();
            let tan = (camera.fov_y * 0.5).tan();
            let ndc = [((at.x - view.left()) / view.width()) * 2.0 - 1.0, ((at.y - view.top()) / view.height()) * 2.0 - 1.0];
            let d = (b.forward + b.right * (ndc[0] * tan * view.width() / view.height()) - b.up * (ndc[1] * tan)).normalize();
            let dir = [d.x as f64, d.y as f64, d.z as f64];
            (camera.eye, dir, renderer.pick(camera.eye, dir, 60000.0))
        };
        let point_at = |eye: [f64; 3], dir: [f64; 3], t: f64| [eye[0] + dir[0] * t, eye[1] + dir[1] * t, eye[2] + dir[2] * t];
        // Track the keys by their places (the walk keys are where W A S D
        // are on any keyboard), and forget them when the window loses the
        // keyboard.
        ui.input(|i| {
            for e in &i.events {
                if let egui::Event::Key { key, physical_key, pressed, repeat: false, .. } = e {
                    let k = physical_key.unwrap_or(*key);
                    if *pressed {
                        self.held.insert(k);
                    } else {
                        self.held.remove(&k);
                    }
                }
            }
            if !i.focused {
                self.held.clear();
            }
        });
        let (mods, pointer, scroll, pinch, dt) = ui.input(|i| (i.modifiers, i.pointer.hover_pos(), i.smooth_scroll_delta.y + i.smooth_scroll_delta.x, i.zoom_delta(), i.stable_dt.min(0.1) as f64));
        let orbiting = response.dragged_by(PointerButton::Primary) || response.dragged_by(PointerButton::Middle);
        if response.drag_started_by(PointerButton::Primary) || response.drag_started_by(PointerButton::Middle) {
            // The pivot (or the point held, to slide): what is under the
            // pointer as the drag starts, else the point looked at.
            self.grab = response.interact_pointer_pos().map(|at| {
                let (eye, dir, t) = under(orbit, at);
                match t {
                    Some(t) => (point_at(eye, dir, t), t),
                    None => (orbit.target, orbit.distance),
                }
            });
        }
        if orbiting {
            let d = response.drag_delta();
            let (pivot, depth) = self.grab.unwrap_or((orbit.target, orbit.distance));
            if mods.shift {
                // The point held follows the pointer.
                orbit.pan(d.x as f64, d.y as f64, depth * (2.0 * (fov.to_radians() * 0.5).tan() as f64) / view.height().max(1.0) as f64);
            } else {
                orbit.orbit_about(pivot, d.x * 0.006, -d.y * 0.006);
            }
        }
        if response.drag_stopped() {
            self.grab = None;
        }
        let looking = response.dragged_by(PointerButton::Secondary);
        if looking {
            let d = response.drag_delta();
            let eye = orbit.eye();
            orbit.orbit_about(eye, d.x * 0.004, -d.y * 0.004);
        }
        if (response.hovered() || looking) && self.still.is_none() && !ui.ctx().egui_wants_keyboard_input() {
            use egui::Key;
            let k = |key: Key| self.held.contains(&key) as i32 as f64;
            let keys = if mods.command || mods.ctrl { [0.0; 3] } else { [k(Key::W) - k(Key::S), k(Key::D) - k(Key::A), k(Key::E) - k(Key::Q)] };
            let walking = keys != [0.0; 3];
            if walking || looking {
                if scroll != 0.0 {
                    self.walk_factor = (self.walk_factor * (scroll as f64 * 0.004).exp()).clamp(0.02, 50.0);
                }
            } else if (scroll != 0.0 || pinch != 1.0)
                && let Some(at) = pointer
            {
                // Toward the point under the pointer, a share of the way a
                // step whatever the distance; on the sky, the point looked
                // at.
                let factor = ((-scroll as f64 * 0.003).exp() / pinch as f64).clamp(0.2, 5.0);
                let (eye, dir, t) = under(orbit, at);
                let point = t.map_or(orbit.target, |t| point_at(eye, dir, t));
                if t.is_some() || orbit.distance * factor < span * 3.0 + 2000.0 {
                    orbit.zoom_toward(point, factor);
                }
            }
            if walking {
                // Where the eyes look, at a speed by the distance ahead (or
                // to the ground under the eyes when ahead is sky).
                let b = orbit.camera(fov).basis();
                let eye = orbit.eye();
                let ahead = renderer.pick(eye, [b.forward.x as f64, b.forward.y as f64, b.forward.z as f64], 6000.0);
                let depth = ahead.unwrap_or_else(|| renderer.surface(eye[0], eye[2]).map_or(200.0, |g| (eye[1] - g).abs().max(2.0)));
                let speed = walk_speed(depth, self.walk_factor, mods.shift, mods.alt) * dt;
                let (f, r) = (b.forward, b.right);
                let step = [
                    (f.x as f64 * keys[0] + r.x as f64 * keys[1]) * speed,
                    (f.y as f64 * keys[0] + r.y as f64 * keys[1] + keys[2]) * speed,
                    (f.z as f64 * keys[0] + r.z as f64 * keys[1]) * speed,
                ];
                for (t, d) in orbit.target.iter_mut().zip(step) {
                    *t += d;
                }
                ui.ctx().request_repaint();
            }
            // The numeric pad's views, as Blender's: 1 from the front, 3
            // from the right, 7 from above (Ctrl: the other side), about
            // the point looked at; . in close on the point under the
            // pointer.
            ui.input_mut(|i| {
                use egui::Modifiers;
                let view_at = |i: &mut egui::InputState, key: Key| {
                    if i.consume_key(Modifiers::CTRL, key) {
                        Some(true)
                    } else if i.consume_key(Modifiers::NONE, key) {
                        Some(false)
                    } else {
                        None
                    }
                };
                if let Some(back) = view_at(i, Key::Num1) {
                    *orbit = Orbit { yaw: if back { std::f32::consts::PI } else { 0.0 }, pitch: 0.0, ..*orbit };
                }
                if let Some(left) = view_at(i, Key::Num3) {
                    *orbit = Orbit { yaw: if left { std::f32::consts::FRAC_PI_2 } else { -std::f32::consts::FRAC_PI_2 }, pitch: 0.0, ..*orbit };
                }
                if let Some(below) = view_at(i, Key::Num7) {
                    *orbit = Orbit { pitch: if below { 1.55 } else { -1.55 }, ..*orbit };
                }
                if i.consume_key(Modifiers::NONE, Key::Period) {
                    self.focus_here = true;
                }
                if !self.lock_camera {
                    if i.consume_key(Modifiers::CTRL | Modifiers::ALT, Key::Num0) {
                        self.camera = Some(*orbit);
                    } else if i.consume_key(Modifiers::NONE, Key::Num0)
                        && let Some(c) = self.camera
                    {
                        *orbit = c;
                    }
                }
            });
        }
        let focus = std::mem::take(&mut self.focus_here);
        if (response.double_clicked() || focus)
            && let Some(at) = pointer
        {
            // In close on the point there, half the way.
            match under(orbit, at) {
                (eye, dir, Some(t)) => {
                    let point = point_at(eye, dir, t);
                    *orbit = Orbit::between(eye, point);
                    orbit.zoom_toward(point, 0.5);
                }
                _ => *orbit = framing(&map, reach, doc.world.meta.sea_level as f64),
            }
        }
        if std::mem::take(&mut self.playback.frame_all) {
            *orbit = framing(&map, reach, doc.world.meta.sea_level as f64);
        }
        // Shift-click: in focus there.
        if response.clicked()
            && mods.shift
            && let Some(at) = response.interact_pointer_pos()
            && let (_, dir, Some(t)) = under(orbit, at)
        {
            let f = orbit.camera(fov).forward();
            self.light.focus = (t * (dir[0] * f.x as f64 + dir[1] * f.y as f64 + dir[2] * f.z as f64)) as f32;
            if self.light.aperture == 0.0 {
                self.light.aperture = 0.4;
            }
        }

        // The image: as large as the view, in pixels; a still's own size
        // while one is drawn.
        // A pixel of the image a point of the view, as Blender's Pixel Size
        // set to Automatic: on a Retina screen, a quarter of the rays (the
        // still saved keeps its own size).
        let ppp = ui.ctx().pixels_per_point();
        let scale = if ppp > 1.0 { 1.0 } else { ppp };
        let px = [(view.width() * scale).round().max(1.0) as u32, (view.height() * scale).round().max(1.0) as u32];
        // (A still of the view's size has its pixels.)
        self.view_px = [(view.width() * ppp).round().max(1.0) as u32, (view.height() * ppp).round().max(1.0) as u32];
        let wanted = self.still.as_ref().map_or(px, |s| s.size);
        scene.renderer.resize(wanted[0], wanted[1]);
        if scene.renderer.size() != Some(self.texture_size) || self.texture.is_none() {
            let image = scene.renderer.view().expect("made by resize");
            let mut egui_renderer = render_state.renderer.write();
            match self.texture {
                Some(id) => egui_renderer.update_egui_texture_from_wgpu_texture(device, image, wgpu::FilterMode::Linear, id),
                None => self.texture = Some(egui_renderer.register_native_texture(device, image, wgpu::FilterMode::Linear)),
            }
            self.texture_size = scene.renderer.size().unwrap_or(wanted);
        }
        let most = if theme::reduce_motion(ui.ctx()) { 64.min(self.most_samples) } else { self.most_samples };
        if let Some(still) = &mut self.still {
            // A band of rows a frame, every row a sample before the next.
            let [w, h] = still.size;
            let rows = ((STILL_BAND / w.max(1)) / 8 * 8).clamp(8, h.max(8));
            if still.row == 0 && scene.renderer.samples == 0 {
                scene.renderer.restart();
            }
            // Drawn band by band, each sample written as it comes; denoised
            // once done, if asked.
            scene.renderer.denoise = false;
            if let Some(cmd) = scene.renderer.render_rows(&still.camera, &still.light, still.row, (still.row + rows).min(h)) {
                queue.submit([cmd]);
            }
            still.row += rows;
            if still.row >= h {
                still.row = 0;
                scene.renderer.sample_done();
            }
            ui.ctx().request_repaint();
            if scene.renderer.samples >= still.samples {
                // Done: read it back, and write it away from the frames.
                let still = self.still.take().expect("drawn above");
                // Denoised as Blender's renders are (Open Image Denoise, on
                // the writing thread), or by the graphics card's filter
                // without it.
                let intel = still.denoise && massif_render::oidn::available();
                let samples = if intel { scene.renderer.read_samples() } else { None };
                if still.denoise
                    && samples.is_none()
                    && let Some(cmd) = scene.renderer.denoise(1.0)
                {
                    queue.submit([cmd]);
                }
                let image = if samples.is_some() { None } else { scene.renderer.read_image() };
                if let Some(movie) = &mut self.movie {
                    // A frame of an animation: to its writer, then the next.
                    let data = match (samples, image) {
                        (Some(s), _) => Some(FrameData::Samples(s)),
                        (None, Some((w, h, rgba))) => Some(FrameData::Image(w, h, rgba)),
                        _ => None,
                    };
                    if let (Some(data), Some(tx)) = (data, &movie.tx) {
                        let _ = tx.send((movie.frame, data));
                    }
                    movie.frame += 1;
                    if movie.frame < movie.frames {
                        self.still = movie.still(movie.frame);
                    } else {
                        // All drawn: the writer closes the file.
                        movie.tx = None;
                    }
                    scene.renderer.restart();
                    ui.ctx().request_repaint();
                    return;
                }
                let (tx, rx) = std::sync::mpsc::channel();
                let took = still.started.elapsed().as_secs_f64();
                std::thread::spawn(move || {
                    let name = still.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    let image = match samples {
                        Some(mut s) => match massif_render::oidn::denoise(&mut s.color, Some(&s.albedo), Some(&s.normal), s.width as usize, s.height as usize) {
                            Ok(()) => Some((s.width, s.height, s.tone.map(&s.color))),
                            Err(e) => {
                                log::warn!("denoising the image: {e}");
                                Some((s.width, s.height, s.tone.map(&s.color)))
                            }
                        },
                        None => image,
                    };
                    let result = match image {
                        Some((w, h, rgba)) => {
                            let mut png = Vec::new();
                            image::write_buffer_with_format(&mut std::io::Cursor::new(&mut png), &rgba, w, h, image::ColorType::Rgba8, image::ImageFormat::Png)
                                .map_err(|e| e.to_string())
                                .and_then(|()| massif_core::atomic::write(&still.path, &png, true).map_err(|e| massif_core::atomic::describe(&e, &still.path)))
                                .map(|()| format!("{name} · {w} × {h} · {took:.0} s"))
                        }
                        None => Err(tr!("the graphics card gave no image").to_owned()),
                    };
                    let _ = tx.send(result.map_err(|e| tr!("The image could not be saved: {e}", e)));
                });
                self.writing = Some(rx);
                scene.renderer.restart();
            }
        } else {
            let shown = (*orbit, self.light, px, self.fov);
            if self.last.is_some_and(|l| l.0 != shown.0 || l.3 != shown.3) {
                self.moving_until = now + STILL_AFTER;
            }
            if self.last.is_some_and(|l| l.1 != shown.1) || changed {
                scene.renderer.restart();
            }
            self.last = Some(shown);
            let moving = now < self.moving_until;
            // With less motion (Settings ▸ Accessibility), the image settles
            // sooner and stops there.
            // Denoised as the samples come (the samples kept as they are);
            // turned on or off, the image made from them again.
            // The graphics card's filter at once; Open Image Denoise (as
            // Blender's) as the samples settle, its image shown in place
            // of the filter's once it comes.
            let toggled = scene.renderer.denoise != self.denoise;
            scene.renderer.denoise = self.denoise;
            if toggled && !self.denoise {
                scene.renderer.hold = false;
                self.intel_shown = false;
            }
            let mut drawn = false;
            // A sample at a time on the graphics card (never frames behind
            // the interface), none while the window is in the background.
            let focused = ui.input(|i| i.focused);
            if (moving || scene.renderer.samples < most) && focused {
                if !scene.renderer.busy()
                    && let Some(cmd) = scene.renderer.render(&orbit.camera(self.fov), &self.light, moving)
                {
                    scene.renderer.submit(cmd);
                    drawn = true;
                }
                ui.ctx().request_repaint();
            }
            let n = scene.renderer.samples;
            if scene.renderer.round != self.round {
                // Started again: what was denoised is of another image.
                self.round = scene.renderer.round;
                self.intel = None;
                self.intel_at = 0;
                self.intel_shown = false;
                scene.renderer.hold = false;
            }
            // (And once more on the last sample, where the image stops.)
            if !self.intel_shown
                && (toggled || (drawn && (scene.renderer.denoise_due() || (self.denoise && n >= most))))
                && let Some(cmd) = scene.renderer.denoise(if self.denoise { 1.0 } else { 0.0 })
            {
                queue.submit([cmd]);
            }
            if self.denoise && !moving && self.intel.is_none() && n != self.intel_at && n > 0 && (INTEL_AT.contains(&n) || n >= most) && massif_render::oidn::available()
                && let Some(mut s) = scene.renderer.read_samples()
            {
                let (tx, rx) = std::sync::mpsc::channel();
                std::thread::spawn(move || {
                    let result = massif_render::oidn::denoise(&mut s.color, Some(&s.albedo), Some(&s.normal), s.width as usize, s.height as usize).map(|()| (s.color, s.tone));
                    let _ = tx.send(result);
                });
                self.intel = Some((self.round, rx));
                self.intel_at = n;
            }
            if let Some((round, rx)) = &self.intel {
                match rx.try_recv() {
                    Ok(result) => {
                        match result {
                            Ok((linear, tone)) if *round == self.round && self.denoise => {
                                scene.renderer.show_linear(&linear, tone);
                                scene.renderer.hold = true;
                                self.intel_shown = true;
                            }
                            Ok(_) => {}
                            Err(e) => log::warn!("denoising the view: {e}"),
                        }
                        self.intel = None;
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => ui.ctx().request_repaint_after(Duration::from_millis(30)),
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => self.intel = None,
                }
            }
        }
        if let Some(movie) = &self.movie {
            match movie.result.try_recv() {
                Ok(result) => {
                    self.movie_saved = Some(result);
                    self.movie = None;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.movie = None,
                Err(std::sync::mpsc::TryRecvError::Empty) => ui.ctx().request_repaint_after(Duration::from_millis(100)),
            }
        }
        if let Some(rx) = &self.writing {
            match rx.try_recv() {
                Ok(result) => {
                    self.saved = Some(result);
                    self.writing = None;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.writing = None,
                Err(std::sync::mpsc::TryRecvError::Empty) => ui.ctx().request_repaint_after(Duration::from_millis(50)),
            }
        }
        let samples = scene.renderer.samples;
        if samples <= 1 || self.since.is_some_and(|(_, n)| samples < n) {
            self.since = (samples == 1).then(|| (Instant::now(), 1));
        }
        if let Some(id) = self.texture {
            // A still fits the view, its own shape kept.
            let [w, h] = self.texture_size;
            let fit = if self.still.is_some() {
                let k = (view.width() / w.max(1) as f32).min(view.height() / h.max(1) as f32);
                Rect::from_center_size(view.center(), vec2(w as f32 * k, h as f32 * k))
            } else {
                view
            };
            ui.painter().image(id, fit, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }

        // What it is at, over the bottom left of the image.
        let status = match (&scene.builder, scene.full) {
            _ if self.movie.is_some() => {
                let m = self.movie.as_ref().expect("checked");
                tr!("Rendering the animation: frame {n} of {all}", n = (m.frame + 1).min(m.frames), all = m.frames)
            }
            _ if self.still.is_some() => {
                let s = self.still.as_ref().expect("checked");
                let done = (scene.renderer.samples as f32 + s.row as f32 / s.size[1].max(1) as f32) / s.samples.max(1) as f32;
                tr!("Drawing the image: {percent} %", percent = (done * 100.0).floor() as u32)
            }
            (_, true) => tr!("The map is larger than the graphics card holds: part of it is missing").to_owned(),
            (Some(b), _) => {
                let (done, all) = b.progress();
                tr!("Building the scene: {done} of {all} tiles", done, all)
            }
            (None, _) => trn!("{n} sample a pixel", "{n} samples a pixel", n = scene.renderer.samples),
        };
        let at = view.left_bottom() + vec2(theme::PAD, -theme::PAD);
        let galley = ui.painter().layout_no_wrap(status, FontId::proportional(12.0), pal.text);
        let back = Rect::from_min_size(at - vec2(0.0, galley.size().y + 8.0), galley.size() + vec2(16.0, 8.0));
        ui.painter().rect(back, theme::PILL, pal.panel, Stroke::new(1.0, pal.border), StrokeKind::Inside);
        ui.painter().galley(back.min + vec2(8.0, 4.0), galley, pal.text);
        // The minimap, over the top right of the view (not while an image
        // or an animation is drawn).
        if self.still.is_none() && self.movie.is_none() {
            self.minimap_ui(ui, view, doc);
        }
    }

    /// Sun and Sky: a look to start from, the sun by the hour and the place,
    /// the air and the clouds.
    pub fn sky_ui(&mut self, ui: &mut Ui, doc: &mut Document) {
        ui.spacing_mut().item_spacing.y = 6.0;
        let sea = doc.world.meta.sea_level as f32;
        let mut keys = Keyer::new(doc, self.playback.playhead);
        let (light, time) = (&mut self.light, &mut self.time);
        let mut picked = None;
        title(ui, tr!("Sun and sky"));
        let before = *time;
        let (light_was, time_was) = (*light, *time);
        well(ui, |ui| {
            row(ui, tr!("Look"), |ui| {
                let name = PRESETS.get(self.preset).map_or(tr!("Custom"), |n| crate::i18n::tr_name(n));
                crate::widgets::combo("render preset").selected_text(name).show_ui(ui, |ui| {
                    for (k, n) in PRESETS.iter().enumerate() {
                        if ui.selectable_label(self.preset == k, crate::i18n::tr_name(n)).clicked() {
                            picked = Some(k);
                        }
                    }
                });
            });
            row(ui, tr!("Time"), |ui| {
                let r = ui.add(Field::new(&mut time.hour, 0.0..=24.0).suffix(" h").max_decimals(2));
                keys.diamond(ui, "hour", time.hour, &r);
            });
            row(ui, tr!("Day"), |ui| {
                ui.add(Field::new(&mut time.day, 1.0..=365.0).max_decimals(0));
            });
            row(ui, tr!("Latitude"), |ui| {
                ui.add(Field::new(&mut time.latitude, -66.0..=66.0).suffix("°").max_decimals(0));
            });
            row(ui, tr!("North"), |ui| {
                ui.add(Field::new(&mut time.north, 0.0..=360.0).suffix("°").max_decimals(0));
            });
            row(ui, tr!("Sun"), |ui| {
                let r = ui.add(Field::new(&mut light.sun_strength, 0.0..=20.0).max_decimals(1));
                keys.diamond(ui, "sun", light.sun_strength, &r);
            });
            row(ui, tr!("Sky"), |ui| {
                let r = ui.add(Field::new(&mut light.sky_strength, 0.0..=4.0).max_decimals(2));
                keys.diamond(ui, "sky", light.sky_strength, &r);
            });
        });
        if *time != before {
            light.set_time(time.hour, time.day, time.latitude, time.north);
        }
        title(ui, tr!("Atmosphere"));
        well(ui, |ui| {
            row(ui, tr!("Air"), |ui| {
                let r = ui.add(Field::new(&mut light.air, 0.0..=4.0).max_decimals(2));
                keys.diamond(ui, "air", light.air, &r);
            });
            row(ui, tr!("Haze"), |ui| {
                let r = ui.add(Field::new(&mut light.haze, 0.0..=8.0).max_decimals(1));
                keys.diamond(ui, "haze", light.haze, &r);
            });
            // Fog by how far one sees through it at its base (where it
            // takes 98 % of the light: an extinction of 3.9 over that
            // distance), from 40 blocks to 4000: never a white wall.
            let mut fog = light.fog > 0.0;
            row(ui, tr!("Fog"), |ui| {
                if crate::widgets::switch(ui, &mut fog).changed() {
                    light.fog = if fog { 3.912 / 400.0 } else { 0.0 };
                }
            });
            if fog {
                row(ui, tr!("Visibility"), |ui| {
                    let mut seen = (3.912 / light.fog.max(1e-6)).clamp(40.0, 4000.0);
                    let r = ui.add(Field::new(&mut seen, 40.0..=4000.0).logarithmic(true).suffix(" blocks").max_decimals(0));
                    if r.changed() {
                        light.fog = 3.912 / seen;
                    }
                    keys.diamond(ui, "fog", light.fog, &r);
                });
                row(ui, tr!("Fog height"), |ui| {
                    let r = ui.add(Field::new(&mut light.fog_height, sea - 64.0..=sea + 256.0).max_decimals(0));
                    keys.diamond(ui, "fog_height", light.fog_height, &r);
                });
                row(ui, tr!("Fog fade"), |ui| {
                    ui.add(Field::new(&mut light.fog_falloff, 2.0..=200.0).suffix(" blocks").max_decimals(0));
                });
            }
            row(ui, tr!("Shafts of light"), |ui| {
                crate::widgets::switch(ui, &mut light.shafts);
            });
            row(ui, tr!("Sea around the map"), |ui| {
                crate::widgets::switch(ui, &mut light.sea);
            });
        });
        title(ui, tr!("Clouds"));
        well(ui, |ui| {
            row(ui, tr!("Cover"), |ui| {
                let r = ui.add(Field::new(&mut light.cloud_cover, 0.0..=1.0).max_decimals(2));
                keys.diamond(ui, "cloud_cover", light.cloud_cover, &r);
            });
            row(ui, tr!("Height"), |ui| {
                let r = ui.add(Field::new(&mut light.cloud_height, sea..=sea + 4000.0).max_decimals(0));
                keys.diamond(ui, "cloud_height", light.cloud_height, &r);
            });
            row(ui, tr!("Thickness"), |ui| {
                ui.add(Field::new(&mut light.cloud_thickness, 50.0..=2000.0).max_decimals(0));
            });
            row(ui, tr!("Density"), |ui| {
                let r = ui.add(Field::new(&mut light.cloud_density, 0.002..=0.1).logarithmic(true).max_decimals(3));
                keys.diamond(ui, "cloud_density", light.cloud_density, &r);
            });
            row(ui, tr!("Size"), |ui| {
                let r = ui.add(Field::new(&mut light.cloud_size, 200.0..=6000.0).logarithmic(true).max_decimals(0));
                keys.diamond(ui, "cloud_size", light.cloud_size, &r);
            });
            row(ui, tr!("Wind"), |ui| {
                let r = ui.add(Field::new(&mut light.wind[0], 0.0..=20000.0).max_decimals(0));
                keys.diamond(ui, "wind", light.wind[0], &r);
            });
        });
        self.after_sky(picked, light_was, time_was, doc);
        if keys.hovered {
            self.key_hover = ui.ctx().cumulative_pass_nr();
        }
        keys.apply(doc);
    }

    /// After the Sun and Sky panel: the look `picked` put on, or a setting
    /// changed by hand (from `light_was`, `time_was`) making the look one of
    /// your own; the hour moved while the camera path has keyframes moving
    /// theirs as much, or the path would put the old hour back at once.
    fn after_sky(&mut self, picked: Option<usize>, light_was: Light, time_was: Time, doc: &mut Document) {
        let sea = doc.world.meta.sea_level as f32;
        let (light, time) = (&mut self.light, &mut self.time);
        let valley = self.scene.as_ref().and_then(|s| s.renderer.valley_floor()).map_or(sea, |v| (v as f32).max(sea));
        if let Some(k) = picked {
            self.preset = k;
            preset(k, light, time, sea, valley);
        } else if *light != light_was || *time != time_was {
            self.preset = usize::MAX;
        }
        let shift = time.hour - time_was.hour;
        if shift != 0.0
            && let Some(anim) = doc.world.meta.animation.as_mut()
            && !anim.keys.is_empty()
        {
            for key in &mut anim.keys {
                key.pose.hour = (key.pose.hour + shift).rem_euclid(24.0);
            }
            doc.mark_changed();
        }
    }

    /// Camera: its lens, where it stands and looks.
    pub fn camera_ui(&mut self, ui: &mut Ui, doc: &mut Document) {
        let pal = Palette::of(ui);
        ui.spacing_mut().item_spacing.y = 6.0;
        let mut keys = Keyer::new(doc, self.playback.playhead);
        title(ui, tr!("Lens"));
        let light = &mut self.light;
        well(ui, |ui| {
            row(ui, tr!("Angle"), |ui| {
                ui.add(Field::new(&mut self.fov, 10.0..=100.0).suffix("°").max_decimals(0));
            });
            row(ui, tr!("Aperture"), |ui| {
                let r = ui.add(Field::new(&mut light.aperture, 0.0..=4.0).suffix(" blocks").max_decimals(2));
                keys.diamond(ui, "aperture", light.aperture, &r);
            });
            row(ui, tr!("Focus"), |ui| {
                let r = ui.add(Field::new(&mut light.focus, 1.0..=5000.0).logarithmic(true).suffix(" blocks").max_decimals(0));
                keys.diamond(ui, "focus", light.focus, &r);
            });
        });
        title(ui, tr!("Position"));
        let extent = self.scene.as_ref().map(|s| s.map);
        match (&mut self.orbit, extent) {
            (Some(orbit), Some(extent)) => {
                let span = extent.width.max(extent.depth) as f64 * 16.0;
                let (min_y, max_y) = (doc.dimension().min_y as f64, doc.dimension().max_y as f64);
                well(ui, |ui| {
                    // Where the eye stands, to the tenth of a block.
                    let c = extent.center();
                    let mut eye = orbit.eye();
                    let before = eye;
                    row(ui, tr!("Location X"), |ui| {
                        ui.add(Field::new(&mut eye[0], c[0] - span * 2.0..=c[0] + span * 2.0).max_decimals(1));
                    });
                    row(ui, tr!("Location Y"), |ui| {
                        ui.add(Field::new(&mut eye[1], min_y..=max_y + 4096.0).max_decimals(1));
                    });
                    row(ui, tr!("Location Z"), |ui| {
                        ui.add(Field::new(&mut eye[2], c[2] - span * 2.0..=c[2] + span * 2.0).max_decimals(1));
                    });
                    if eye != before {
                        *orbit = Orbit::from_eye(eye, orbit.yaw, orbit.pitch, orbit.distance);
                    }
                    let mut turn = orbit.yaw.to_degrees().rem_euclid(360.0);
                    row(ui, tr!("Turn"), |ui| {
                        if ui.add(Field::new(&mut turn, 0.0..=360.0).suffix("°").max_decimals(0)).changed() {
                            orbit.yaw = turn.to_radians();
                        }
                    });
                    let mut tilt = orbit.pitch.to_degrees();
                    row(ui, tr!("Tilt"), |ui| {
                        if ui.add(Field::new(&mut tilt, -89.0..=68.0).suffix("°").max_decimals(0)).changed() {
                            orbit.pitch = tilt.to_radians();
                        }
                    });
                    row(ui, tr!("Distance"), |ui| {
                        ui.add(Field::new(&mut orbit.distance, 3.0..=span * 3.0 + 2000.0).logarithmic(true).suffix(" blocks").max_decimals(0));
                    });
                });
                if ui.button(theme::with_icon(Icon::Focus, tr!("Frame the whole map"))).clicked() {
                    *orbit = framing(&extent, render_reach(), doc.world.meta.sea_level as f64);
                }
                // The camera apart from the view, as Blender's: unlocked,
                // the view moves alone and stills, keyframes and the video
                // keep to the camera until it is aligned to the view.
                let view = *orbit;
                well(ui, |ui| {
                    row(ui, tr!("Lock Camera to View"), |ui| {
                        if crate::widgets::switch(ui, &mut self.lock_camera).changed() && !self.lock_camera {
                            self.camera = Some(view);
                        }
                    });
                });
                if !self.lock_camera {
                    ui.horizontal_wrapped(|ui| {
                        if ui.button(tr!("Align Camera to View")).on_hover_text(tr!("Put the camera where the view is (Ctrl+Alt+Numpad 0)")).clicked() {
                            self.camera = Some(view);
                        }
                        if ui.button(tr!("View through Camera")).on_hover_text(tr!("Put the view where the camera is (Numpad 0)")).clicked()
                            && let (Some(c), Some(o)) = (self.camera, self.orbit.as_mut())
                        {
                            *o = c;
                        }
                    });
                }
            }
            _ => {
                ui.label(RichText::new(tr!("The camera is placed once the Render Viewport shows the map.")).size(11.5).color(pal.weak));
            }
        }
        ui.add_space(4.0);
        ui.label(RichText::new(tr!("Drag: turn about the point there · Shift-drag: slide · wheel: toward the pointer · right-drag: look around · W A S D, Q E: walk (Shift faster, Alt slower, wheel: speed) · double-click or .: in close there · Shift-click: focus there · Numpad 1 3 7: views")).size(11.0).color(pal.weak));
        if keys.hovered {
            self.key_hover = ui.ctx().cumulative_pass_nr();
        }
        keys.apply(doc);
    }

    /// Render Output: how the image is exposed, and saving it.
    pub fn output_ui(&mut self, ui: &mut Ui, doc: &mut Document) {
        let pal = Palette::of(ui);
        ui.spacing_mut().item_spacing.y = 6.0;
        let mut keys = Keyer::new(doc, self.playback.playhead);
        title(ui, tr!("Grass"));
        well(ui, |ui| {
            row(ui, tr!("Grass"), |ui| {
                let labels = [tr!("Blocks"), tr!("Blades")];
                if let Some(k) = crate::widgets::button_group(ui, "render grass", &labels, &[!self.blades, self.blades]) {
                    self.blades = k == 1;
                }
            });
            if self.blades {
                row(ui, tr!("Density"), |ui| {
                    ui.add(Field::new(&mut self.light.grass_blades, 3..=60)).on_hover_text(tr!("Blades of grass a block: more, a fuller field and a slower image"));
                });
                row(ui, tr!("Height"), |ui| {
                    ui.add(Field::new(&mut self.light.grass_height, 0.2..=1.0).max_decimals(2).suffix(" blocks"));
                });
            }
        });
        let light = &mut self.light;
        // Blender's sections: Sampling, Light Paths, Color Management.
        title(ui, tr!("Sampling"));
        well(ui, |ui| {
            row(ui, tr!("Viewport samples"), |ui| {
                ui.add(Field::new(&mut self.most_samples, 16..=16384).logarithmic(true)).on_hover_text(tr!("The view stops adding samples there"));
            });
            row(ui, tr!("Denoise"), |ui| {
                crate::widgets::switch(ui, &mut self.denoise).on_hover_text(tr!("Smooth the grain of the first samples away while they add up, the textures kept sharp"));
            });
            row(ui, tr!("Clamp indirect"), |ui| {
                ui.add(Field::new(&mut light.clamp, 0.0..=100.0).logarithmic(true).max_decimals(1))
                    .on_hover_text(tr!("The most light a bounce brings: lower settles sooner, with fewer bright specks and a little less light; 0 for no limit"));
            });
        });
        title(ui, tr!("Light Paths"));
        well(ui, |ui| {
            row(ui, tr!("Bounces"), |ui| {
                ui.add(Field::new(&mut light.bounces, 0..=6));
            });
            row(ui, tr!("Glow"), |ui| {
                ui.add(Field::new(&mut light.emission, 0.0..=8.0).max_decimals(2));
            });
        });
        // Colour management, as Blender's: the display, the view transform
        // and its look, the exposure in stops, the gamma.
        title(ui, tr!("Color Management"));
        well(ui, |ui| {
            use massif_render::color;
            row(ui, tr!("Display Device"), |ui| {
                ui.label("sRGB");
            });
            row(ui, tr!("View Transform"), |ui| {
                crate::widgets::combo("render view transform").selected_text(color::VIEWS[(light.view as usize).min(color::VIEWS.len() - 1)]).show_ui(ui, |ui| {
                    for (k, name) in color::VIEWS.iter().enumerate() {
                        if ui.selectable_label(light.view == k as u32, *name).clicked() && light.view != k as u32 {
                            light.view = k as u32;
                            light.look = 0;
                        }
                    }
                });
            });
            let looks = color::looks(light.view);
            row(ui, tr!("Look"), |ui| {
                ui.add_enabled_ui(looks.len() > 1, |ui| {
                    crate::widgets::combo("render view look").selected_text(looks.get(light.look as usize).copied().unwrap_or("None")).show_ui(ui, |ui| {
                        for (k, name) in looks.iter().enumerate() {
                            ui.selectable_value(&mut light.look, k as u32, *name);
                        }
                    });
                });
            });
            row(ui, tr!("Exposure"), |ui| {
                let mut stops = light.exposure.max(1e-4).log2();
                let r = ui.add(Field::new(&mut stops, -10.0..=10.0).max_decimals(2));
                if r.changed() {
                    light.exposure = 2f32.powf(stops);
                }
                keys.diamond(ui, "exposure", light.exposure, &r);
            });
            row(ui, tr!("Gamma"), |ui| {
                ui.add(Field::new(&mut light.gamma, 0.1..=5.0).max_decimals(2));
            });
        });
        title(ui, tr!("Output: image"));
        let mut start = false;
        well(ui, |ui| {
            row(ui, tr!("Size"), |ui| {
                let (name, size) = STILL_SIZES[self.still_size.min(STILL_SIZES.len() - 1)];
                let text = size.map_or_else(|| crate::i18n::tr_name(name).to_owned(), |[w, h]| format!("{name} · {w} × {h}"));
                crate::widgets::combo("render still size").selected_text(text).show_ui(ui, |ui| {
                    for (k, (name, size)) in STILL_SIZES.iter().enumerate() {
                        let text = size.map_or_else(|| crate::i18n::tr_name(name).to_owned(), |[w, h]| format!("{name} · {w} × {h}"));
                        ui.selectable_value(&mut self.still_size, k, text);
                    }
                });
            });
            row(ui, tr!("Samples a pixel"), |ui| {
                ui.add(Field::new(&mut self.still_samples, 16..=4096).logarithmic(true));
            });
            row(ui, tr!("Denoise"), |ui| {
                crate::widgets::switch(ui, &mut self.still_denoise).on_hover_text(tr!("Denoise the image before it is saved"));
            });
        });
        let ready = self.scene.is_some() && self.orbit.is_some() && self.view_px != [0; 2];
        if let (Some(s), Some(scene)) = (&self.still, &self.scene) {
            let done = (scene.renderer.samples as f32 + s.row as f32 / s.size[1].max(1) as f32) / s.samples.max(1) as f32;
            let mut cancel = false;
            ui.horizontal(|ui| {
                ui.add(egui::ProgressBar::new(done.clamp(0.0, 1.0)).desired_width(ui.available_width() - 90.0));
                cancel = ui.button(tr!("Cancel")).clicked();
            });
            if cancel {
                self.still = None;
                if let Some(scene) = &mut self.scene {
                    scene.renderer.restart();
                }
            }
        } else if ui
            .add_enabled(ready && self.writing.is_none(), theme::primary_button(ui, theme::with_icon(Icon::Camera, tr!("Save image…"))))
            .on_hover_text(tr!("Render the view at full quality and save it as a PNG"))
            .on_disabled_hover_text(tr!("Show the Render Viewport first"))
            .clicked()
        {
            start = true;
        }
        match &self.saved {
            Some(Ok(name)) => {
                ui.label(RichText::new(tr!("Saved {name}", name)).size(11.5).color(pal.weak));
            }
            Some(Err(e)) => {
                ui.label(RichText::new(e.as_str()).size(11.5).color(pal.danger));
            }
            None => {}
        }
        if start
            && let (Some(orbit), true) = (self.shot(), self.scene.is_some())
            && let Some(scene) = &mut self.scene
        {
            let size = STILL_SIZES[self.still_size.min(STILL_SIZES.len() - 1)].1.unwrap_or(self.view_px);
            if !scene.renderer.fits(size[0], size[1]) {
                self.saved = Some(Err(tr!("This size is larger than the graphics card can draw").to_owned()));
            } else if let Some(path) = rfd::FileDialog::new().set_file_name(format!("{}.png", doc.title())).add_filter(tr!("PNG image"), &["png"]).save_file() {
                self.saved = None;
                self.still = Some(Still { path, size, samples: self.still_samples, camera: orbit.camera(self.fov), light: self.light, denoise: self.still_denoise, row: 0, started: Instant::now() });
                scene.renderer.restart();
            }
        }
        if keys.hovered {
            self.key_hover = ui.ctx().cumulative_pass_nr();
        }
        keys.apply(doc);
        self.movie_ui(ui, doc);
        let Some(scene) = &self.scene else { return };
        title(ui, tr!("Scene"));
        let memory = scene.renderer.memory();
        let mb = |b: u64| format!("{:.0} MB", b as f64 / (1 << 20) as f64);
        let samples = scene.renderer.samples;
        let most = if theme::reduce_motion(ui.ctx()) { 64.min(self.most_samples) } else { self.most_samples };
        well(ui, |ui| {
            row(ui, tr!("Tiles"), |ui| {
                ui.label(format!("{} / {}", scene.built.len(), scene.tiles));
            });
            row(ui, tr!("Chunks"), |ui| {
                ui.label(scene.renderer.chunks().to_string());
            });
            if let Some(took) = scene.took {
                row(ui, tr!("Built in"), |ui| {
                    ui.label(format!("{:.1} s", took.as_secs_f64()));
                });
            }
            row(ui, tr!("Graphics memory"), |ui| {
                ui.label(mb(memory.total()));
            });
            row(ui, tr!("Samples a second"), |ui| {
                let rate = self.since.filter(|_| samples > 4 && samples < most).map(|(t, n)| (samples - n) as f64 / t.elapsed().as_secs_f64().max(1e-3));
                ui.label(rate.map_or_else(|| "—".to_owned(), |r| format!("{r:.0}")));
            });
        });
    }

    /// Animation to save: the Timeline's camera path drawn frame by frame
    /// (each a still of the size and samples asked, denoised), written as
    /// an MP4 or as PNG images; its progress, the time left and Cancel.
    fn movie_ui(&mut self, ui: &mut Ui, doc: &Document) {
        let pal = Palette::of(ui);
        let anim = doc.world.meta.animation.clone().unwrap_or_default();
        title(ui, tr!("Output: animation"));
        well(ui, |ui| {
            row(ui, tr!("Format"), |ui| {
                let labels = [tr!("MP4 video"), tr!("PNG images")];
                let on = [self.movie_format == MovieFormat::Mp4, self.movie_format == MovieFormat::Png];
                if let Some(k) = crate::widgets::button_group(ui, "render movie format", &labels, &on) {
                    self.movie_format = if k == 0 && crate::video::mp4_supported() { MovieFormat::Mp4 } else { MovieFormat::Png };
                }
            });
            row(ui, tr!("Samples a frame"), |ui| {
                ui.add(Field::new(&mut self.movie_samples, 4..=4096).logarithmic(true));
            });
            row(ui, tr!("Frames"), |ui| {
                ui.label(format!("{} · {} fps · {:.1} s", anim.frames(), anim.fps, anim.duration));
            });
        });
        if !crate::video::mp4_supported() && self.movie_format == MovieFormat::Mp4 {
            self.movie_format = MovieFormat::Png;
        }
        if let Some(m) = &self.movie {
            let (done, all) = m.progress();
            let left = (done > 0).then(|| m.started.elapsed().as_secs_f64() / done as f64 * (all - done) as f64);
            let mut cancel = false;
            ui.horizontal(|ui| {
                ui.add(egui::ProgressBar::new(done as f32 / all.max(1) as f32).desired_width(ui.available_width() - 90.0));
                cancel = ui.button(tr!("Cancel")).clicked();
            });
            let left = left.map_or_else(String::new, |s| format!(" · {}", tr!("{min} min {sec} s left", min = (s / 60.0) as u32, sec = (s % 60.0) as u32)));
            ui.label(RichText::new(format!("{}{left}", tr!("{done} of {all} frames", done, all))).size(11.5).color(pal.weak));
            if cancel {
                // The frames drawn so far stay written (an MP4 closed on
                // them; a PNG sequence goes on from there next time).
                self.still = None;
                if let Some(m) = &mut self.movie {
                    m.tx = None;
                }
                if let Some(scene) = &mut self.scene {
                    scene.renderer.restart();
                }
            }
        } else {
            let ready = self.scene.is_some() && self.still.is_none() && self.writing.is_none() && anim.keys.len() >= 2;
            if ui
                .add_enabled(ready, theme::primary_button(ui, theme::with_icon(Icon::Film, tr!("Render animation…"))))
                .on_hover_text(tr!("Draw the Timeline's camera path frame by frame and save it"))
                .on_disabled_hover_text(tr!("Add two keyframes on the Timeline, with the Render Viewport shown"))
                .clicked()
            {
                self.start_movie(doc, anim);
            }
        }
        match &self.movie_saved {
            Some(Ok(name)) => {
                ui.label(RichText::new(tr!("Saved {name}", name)).size(11.5).color(pal.weak));
            }
            Some(Err(e)) => {
                ui.label(RichText::new(e.as_str()).size(11.5).color(pal.danger));
            }
            None => {}
        }
    }

    /// Ask where, then draw `anim`'s frames, written on a thread of their
    /// own as they come.
    fn start_movie(&mut self, doc: &Document, anim: Animation) {
        let stem = doc.title();
        let target = match self.movie_format {
            MovieFormat::Mp4 => rfd::FileDialog::new().set_file_name(format!("{stem}.mp4")).add_filter(tr!("MP4 video"), &["mp4"]).save_file(),
            MovieFormat::Png => rfd::FileDialog::new().set_title(tr!("A folder for the animation's images")).pick_folder(),
        };
        if let Some(target) = target {
            let size = STILL_SIZES[self.still_size.min(STILL_SIZES.len() - 1)].1.unwrap_or(self.view_px);
            self.launch_movie(&stem, anim, target, size);
        }
    }

    /// Draw `anim` at `size` (made even) into `target`: the MP4's file, or
    /// the folder of the PNG images, named after `stem`.
    fn launch_movie(&mut self, stem: &str, anim: Animation, target: PathBuf, size: [u32; 2]) {
        let Some(scene) = &mut self.scene else { return };
        // Even, as H.264 takes.
        let size = [size[0] & !1, size[1] & !1];
        if !scene.renderer.fits(size[0], size[1]) {
            self.movie_saved = Some(Err(tr!("This size is larger than the graphics card can draw").to_owned()));
            return;
        }
        let frames = anim.frames();
        let (sink, first, name) = match self.movie_format {
            MovieFormat::Mp4 => {
                let path = target;
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                match crate::video::Sink::mp4(&path, size[0], size[1], anim.fps) {
                    Ok(sink) => (sink, 0, name),
                    Err(e) => {
                        self.movie_saved = Some(Err(tr!("The animation could not be saved: {e}", e)));
                        return;
                    }
                }
            }
            MovieFormat::Png => {
                let dir = target;
                // Going on from the first frame not written yet.
                let first = (0..frames).find(|&k| !crate::video::png_path(&dir, stem, k).exists()).unwrap_or(frames);
                let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                (crate::video::Sink::Png { dir, stem: stem.to_owned() }, first, name)
            }
        };
        if first >= frames {
            self.movie_saved = Some(Ok(name));
            return;
        }
        let (tx, rx) = std::sync::mpsc::sync_channel::<(u32, FrameData)>(2);
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let written = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let count = written.clone();
        let started = Instant::now();
        std::thread::Builder::new()
            .name("massif animation writer".into())
            .spawn(move || {
                let mut sink = sink;
                let mut failed = None;
                for (index, data) in rx {
                    let (w, h, rgba) = match data {
                        FrameData::Image(w, h, rgba) => (w, h, rgba),
                        FrameData::Samples(mut s) => {
                            if let Err(e) = massif_render::oidn::denoise(&mut s.color, Some(&s.albedo), Some(&s.normal), s.width as usize, s.height as usize) {
                                log::warn!("denoising a frame: {e}");
                            }
                            (s.width, s.height, s.tone.map(&s.color))
                        }
                    };
                    if failed.is_none()
                        && let Err(e) = sink.push(index, &rgba, w, h)
                    {
                        failed = Some(e);
                    }
                    count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
                let result = match (failed, sink.finish()) {
                    (Some(e), _) | (None, Err(e)) => Err(tr!("The animation could not be saved: {e}", e)),
                    (None, Ok(())) => Ok(format!("{name} · {} × {} · {:.0} s", size[0], size[1], started.elapsed().as_secs_f64())),
                };
                let _ = done_tx.send(result);
            })
            .expect("a thread for the animation");
        let movie = Movie { anim, frames, first, frame: first, size, samples: self.movie_samples, denoise: self.denoise, light: self.light, time: self.time, tx: Some(tx), written, result: done_rx, started };
        self.still = movie.still(first);
        self.movie = Some(movie);
        self.movie_saved = None;
        scene.renderer.restart();
    }

    /// The minimap in the view's top right corner, and the button that
    /// shows or hides it (remembered): the whole map, the camera on it with
    /// what it sees, the Timeline's path. A click there puts the camera
    /// over that place, its height and its way kept; a drag slides it.
    fn minimap_ui(&mut self, ui: &mut Ui, view: Rect, doc: &Document) {
        let pal = Palette::of(ui);
        let id = egui::Id::unique("render minimap on");
        let mut on = ui.data_mut(|d| d.get_persisted::<bool>(id)).unwrap_or(false);
        let button = Rect::from_min_size(pos2(view.right() - theme::PAD - 28.0, view.top() + theme::PAD), vec2(28.0, 28.0));
        let response = ui.interact(button, egui::Id::unique("render minimap button"), Sense::click()).on_hover_text(if on { tr!("Hide the map") } else { tr!("Show the map") });
        ui.painter().rect(button, theme::PILL, if on { pal.accent } else { pal.panel }, Stroke::new(1.0, pal.border), StrokeKind::Inside);
        ui.painter().text(button.center(), Align2::CENTER_CENTER, theme::glyph(Icon::Map).to_string(), FontId::proportional(14.0), if on { pal.panel } else { pal.text });
        if response.clicked() {
            on = !on;
            ui.data_mut(|d| d.insert_persisted(id, on));
        }
        if !on {
            return;
        }
        // The picture, drawn again away from the frames when the map has
        // changed (at most every two seconds).
        let revision = doc.revision ^ doc.generation.rotate_left(32) ^ (doc.dim as u64).rotate_left(48);
        if let Some((rev, rx)) = &self.minimap_job {
            match rx.try_recv() {
                Ok(picture) => {
                    let rev = *rev;
                    self.minimap_job = None;
                    if let Some(p) = picture {
                        let image = egui::ColorImage::from_rgba_unmultiplied(p.size, &p.rgba);
                        let texture = ui.ctx().load_texture("render minimap", image, egui::TextureOptions::LINEAR);
                        self.minimap = Some((rev, texture, p.blocks));
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => ui.ctx().request_repaint_after(Duration::from_millis(100)),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.minimap_job = None,
            }
        }
        if self.minimap_job.is_none() && self.minimap.as_ref().is_none_or(|m| m.0 != revision) && self.minimap_asked.is_none_or(|t| t.elapsed() >= Duration::from_secs(2)) {
            let (tx, rx) = std::sync::mpsc::channel();
            let (world, dim) = (doc.world.copy_unfrozen(), doc.dim);
            std::thread::spawn(move || {
                let _ = tx.send(crate::render_minimap::draw(&world, dim, 256));
            });
            self.minimap_job = Some((revision, rx));
            self.minimap_asked = Some(Instant::now());
        }
        let Some((_, texture, blocks)) = &self.minimap else { return };
        let (bw, bh) = (blocks[2] - blocks[0], blocks[3] - blocks[1]);
        let side = 190.0f32.min(view.width() * 0.35).min(view.height() * 0.45);
        let k = side / bw.max(bh) as f32;
        let size = vec2(bw as f32 * k, bh as f32 * k);
        let rect = Rect::from_min_size(pos2(button.right() - size.x, button.bottom() + 6.0), size);
        self.minimap_shown = Some(rect);
        let frame = rect.expand(3.0);
        ui.painter().rect(frame, theme::TILE_RADIUS, pal.panel, Stroke::new(1.0, pal.border), StrokeKind::Inside);
        ui.painter().image(texture.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        let to_screen = |x: f64, z: f64| pos2(rect.left() + ((x - blocks[0]) / bw) as f32 * size.x, rect.top() + ((z - blocks[1]) / bh) as f32 * size.y);
        let to_world = |p: egui::Pos2| (blocks[0] + ((p.x - rect.left()) / size.x) as f64 * bw, blocks[1] + ((p.y - rect.top()) / size.y) as f64 * bh);
        // The Timeline's path.
        if let Some(anim) = doc.world.meta.animation.as_ref().filter(|a| a.keys.len() > 1) {
            let points: Vec<egui::Pos2> = (0..=64).filter_map(|k| anim.sample(anim.duration * k as f32 / 64.0)).map(|p| to_screen(p.eye[0], p.eye[2])).collect();
            ui.painter().add(egui::Shape::line(points, Stroke::new(1.5, pal.accent.gamma_multiply(0.7))));
        }
        let response = ui.interact(rect, egui::Id::unique("render minimap"), Sense::click_and_drag());
        let Some(orbit) = self.orbit.as_mut() else { return };
        // A click: the camera over that place; a drag: the view slides
        // with the pointer.
        if (response.clicked() || response.drag_started())
            && let Some(at) = response.interact_pointer_pos()
        {
            let eye = orbit.eye();
            let (x, z) = to_world(at);
            *orbit = Orbit::from_eye([x, eye[1], z], orbit.yaw, orbit.pitch, orbit.distance);
        } else if response.dragged() {
            let d = response.drag_delta();
            let eye = orbit.eye();
            let (dx, dz) = (d.x as f64 / size.x as f64 * bw, d.y as f64 / size.y as f64 * bh);
            *orbit = Orbit::from_eye([eye[0] + dx, eye[1], eye[2] + dz], orbit.yaw, orbit.pitch, orbit.distance);
        }
        // The camera and what it sees.
        let camera = orbit.camera(self.fov);
        let eye = to_screen(camera.eye[0], camera.eye[2]);
        let half = (self.fov.to_radians() * 0.5 * view.width() / view.height().max(1.0)).min(1.4);
        let reach = 26.0;
        let side_ray = |a: f32| eye + vec2((camera.yaw + a).sin(), -(camera.yaw + a).cos()) * reach;
        ui.painter().add(egui::Shape::convex_polygon(vec![eye, side_ray(-half), side_ray(half)], pal.accent.gamma_multiply(0.25), Stroke::new(1.0, pal.accent)));
        ui.painter().circle(eye, 3.5, pal.accent, Stroke::new(1.0, pal.panel));
    }

    /// The Timeline: keyframes of the camera and the sun over time, a
    /// playhead to scrub, play and step from key to key. Scrubbed, the
    /// Render Viewport shows the animation there.
    pub fn timeline_ui(&mut self, ui: &mut Ui, doc: &mut Document) {
        let pal = Palette::of(ui);
        let reduce = theme::reduce_motion(ui.ctx());
        let mut anim = doc.world.meta.animation.clone().unwrap_or_default();
        let before = anim.clone();
        let pb = &mut self.playback;
        pb.playhead = pb.playhead.clamp(0.0, anim.duration);
        if reduce {
            pb.playing = false;
        }
        if pb.playing {
            let dt = ui.input(|i| i.stable_dt).min(0.1);
            pb.playhead += dt;
            if pb.playhead >= anim.duration {
                pb.playhead = 0.0;
            }
            ui.ctx().request_repaint();
        }
        let pose = self.pose();
        let pb = &mut self.playback;
        // Blender's keys, unless a field is being typed in: Space plays,
        // I (or K) keeps the camera at the playhead, Alt+I removes the key
        // there, ← → a frame, ↑ ↓ the previous or next key, Shift+← → the
        // start or the end, Home the whole map.
        let key_hover = self.key_hover + 2 >= ui.ctx().cumulative_pass_nr();
        if !ui.ctx().egui_wants_keyboard_input() {
            use egui::{Key, Modifiers};
            let frame = 1.0 / anim.fps.max(1) as f32;
            let (prev_key, next_key) = anim.neighbours(pb.playhead);
            ui.input_mut(|i| {
                if i.consume_key(Modifiers::NONE, Key::Space) && !reduce && anim.keys.len() > 1 {
                    pb.playing = !pb.playing;
                }
                // (Alt+I first: a key without Alt matches it too. Over a
                // setting, I keys the setting instead: its diamond answers.)
                if key_hover {
                    // (The setting's diamond keys it.)
                } else if i.consume_key(Modifiers::ALT, Key::I) {
                    if let Some(k) = anim.key_at(pb.playhead) {
                        anim.remove(k);
                        pb.selected = None;
                    }
                } else if (i.consume_key(Modifiers::NONE, Key::I) || i.consume_key(Modifiers::NONE, Key::K))
                    && let Some(pose) = pose
                {
                    pb.selected = Some(anim.set(pb.playhead, pose));
                    pb.applied = Some(pb.playhead);
                }
                if i.consume_key(Modifiers::SHIFT, Key::ArrowLeft) {
                    pb.playhead = 0.0;
                    pb.playing = false;
                }
                if i.consume_key(Modifiers::SHIFT, Key::ArrowRight) {
                    pb.playhead = anim.duration;
                    pb.playing = false;
                }
                if i.consume_key(Modifiers::NONE, Key::ArrowLeft) {
                    pb.playhead = ((pb.playhead / frame).round() - 1.0).max(0.0) * frame;
                    pb.playing = false;
                }
                if i.consume_key(Modifiers::NONE, Key::ArrowRight) {
                    pb.playhead = (((pb.playhead / frame).round() + 1.0) * frame).min(anim.duration);
                    pb.playing = false;
                }
                if i.consume_key(Modifiers::NONE, Key::ArrowDown)
                    && let Some(k) = prev_key
                {
                    pb.playhead = anim.keys[k].time;
                    pb.selected = Some(k);
                }
                if i.consume_key(Modifiers::NONE, Key::ArrowUp)
                    && let Some(k) = next_key
                {
                    pb.playhead = anim.keys[k].time;
                    pb.selected = Some(k);
                }
                if i.consume_key(Modifiers::NONE, Key::Home) {
                    pb.frame_all = true;
                }
            });
        }
        let at_key = anim.key_at(pb.playhead);
        let (prev, next) = anim.neighbours(pb.playhead);

        // The commands, in a row that wraps.
        let icon = |i: Icon| RichText::new(theme::glyph(i).to_string()).size(14.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            if ui.button(icon(Icon::SkipBack)).on_hover_text(tr!("To the start (Shift+←)")).clicked() {
                pb.playhead = 0.0;
            }
            if ui.add_enabled(prev.is_some(), egui::Button::new(icon(Icon::ChevronFirst))).on_hover_text(tr!("Previous keyframe (↓)")).clicked()
                && let Some(k) = prev
            {
                pb.playhead = anim.keys[k].time;
                pb.selected = Some(k);
            }
            let play = if pb.playing { icon(Icon::Pause) } else { icon(Icon::Play) };
            if ui
                .add_enabled(!reduce && anim.keys.len() > 1, egui::Button::new(play))
                .on_hover_text(if pb.playing { tr!("Pause (Space)") } else { tr!("Play (Space)") })
                .on_disabled_hover_text(if reduce { tr!("With less motion on, scrub the timeline instead") } else { tr!("Add two keyframes to play the path between them") })
                .clicked()
            {
                pb.playing = !pb.playing;
            }
            if ui.add_enabled(next.is_some(), egui::Button::new(icon(Icon::ChevronLast))).on_hover_text(tr!("Next keyframe (↑)")).clicked()
                && let Some(k) = next
            {
                pb.playhead = anim.keys[k].time;
                pb.selected = Some(k);
            }
            if ui.button(icon(Icon::SkipForward)).on_hover_text(tr!("To the end (Shift+→)")).clicked() {
                pb.playhead = anim.duration;
            }
            ui.separator();
            ui.label(RichText::new(format!("{:.2} s / {:.2} s", pb.playhead, anim.duration)).monospace().size(12.0));
            ui.separator();
            let add = if at_key.is_some() { tr!("Update keyframe") } else { tr!("Add keyframe") };
            if ui
                .add_enabled(pose.is_some(), egui::Button::new(theme::with_icon(Icon::DiamondPlus, add)))
                .on_hover_text(tr!("Keep the camera, its angle and the hour at the playhead (I or K)"))
                .on_disabled_hover_text(tr!("Show the Render Viewport first"))
                .clicked()
                && let Some(pose) = pose
            {
                pb.selected = Some(anim.set(pb.playhead, pose));
                pb.applied = Some(pb.playhead);
            }
            let remove = pb.selected.filter(|&k| k < anim.keys.len()).or(at_key);
            if ui.add_enabled(remove.is_some(), egui::Button::new(theme::with_icon(Icon::DiamondMinus, tr!("Remove")))).on_hover_text(tr!("Remove the keyframe picked")).clicked()
                && let Some(k) = remove
            {
                anim.remove(k);
                pb.selected = None;
            }
            ui.separator();
            ui.label(RichText::new(tr!("Length")).color(pal.weak));
            let shortest = anim.keys.last().map_or(0.5, |k| k.time.max(0.5));
            ui.add(Field::new(&mut anim.duration, shortest..=600.0).suffix(" s").max_decimals(1));
            ui.label(RichText::new(tr!("Frames a second")).color(pal.weak));
            anim.fps = anim.fps.clamp(12, 60);
            ui.add(Field::new(&mut anim.fps, 12..=60));
        });
        ui.add_space(6.0);

        // The track: seconds along, keyframes as diamonds, the playhead.
        let height = ui.available_height().clamp(44.0, 120.0);
        let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        painter.rect(rect, theme::TILE_RADIUS, pal.well(), Stroke::new(1.0, pal.border), StrokeKind::Inside);
        let track = rect.shrink2(vec2(12.0, 0.0));
        let duration = anim.duration.max(1e-3);
        let x_of = |t: f32| track.left() + (t / duration).clamp(0.0, 1.0) * track.width();
        let t_of = |x: f32| ((x - track.left()) / track.width().max(1.0)).clamp(0.0, 1.0) * duration;
        // A tick a second, or every 5, 10, 30 or 60 when they would crowd.
        let step = [1.0, 2.0, 5.0, 10.0, 30.0, 60.0, 120.0].into_iter().find(|s| track.width() / (duration / s) >= 36.0).unwrap_or(300.0);
        let ruler = rect.top() + 16.0;
        let mut s = 0.0;
        while s <= duration + 1e-3 {
            let x = x_of(s);
            painter.vline(x, rect.top() + 2.0..=ruler, Stroke::new(1.0, pal.border));
            let (at, align) = if x + 28.0 > rect.right() { (pos2(x - 3.0, rect.top() + 2.0), Align2::RIGHT_TOP) } else { (pos2(x + 3.0, rect.top() + 2.0), Align2::LEFT_TOP) };
            painter.text(at, align, format!("{s:.0} s"), FontId::proportional(10.0), pal.weak);
            s += step;
        }
        painter.hline(rect.x_range(), ruler, Stroke::new(1.0, pal.border));
        let lane = (ruler + rect.bottom()) / 2.0;
        // The path between the keys.
        if let (Some(first), Some(last)) = (anim.keys.first(), anim.keys.last()) {
            painter.hline(x_of(first.time)..=x_of(last.time), lane, Stroke::new(2.0, pal.accent.gamma_multiply(0.45)));
        }
        let pointer = response.interact_pointer_pos().or(response.hover_pos());
        let near_key = |anim: &Animation, p: egui::Pos2| anim.keys.iter().position(|k| (x_of(k.time) - p.x).abs() <= 7.0 && (p.y - lane).abs() <= 10.0);
        if response.drag_started()
            && let Some(p) = pointer
        {
            pb.dragging = near_key(&anim, p);
            if let Some(k) = pb.dragging {
                pb.selected = Some(k);
            }
        }
        if response.dragged()
            && let Some(p) = pointer
        {
            match pb.dragging {
                Some(k) => {
                    let k = anim.move_key(k, t_of(p.x));
                    pb.dragging = Some(k);
                    pb.selected = Some(k);
                    pb.playhead = anim.keys[k].time;
                }
                None => pb.playhead = t_of(p.x),
            }
        }
        if response.drag_stopped() {
            pb.dragging = None;
        }
        if response.clicked()
            && let Some(p) = pointer
        {
            match near_key(&anim, p) {
                Some(k) => {
                    pb.selected = Some(k);
                    pb.playhead = anim.keys[k].time;
                }
                None => {
                    pb.selected = None;
                    pb.playhead = t_of(p.x);
                }
            }
        }
        for (k, key) in anim.keys.iter().enumerate() {
            let c = pos2(x_of(key.time), lane);
            let r = 6.0;
            let diamond = vec![c + vec2(0.0, -r), c + vec2(r, 0.0), c + vec2(0.0, r), c + vec2(-r, 0.0)];
            let picked = pb.selected == Some(k);
            let fill = if picked { pal.accent } else { pal.panel };
            painter.add(egui::Shape::convex_polygon(diamond, fill, Stroke::new(1.5, if picked { pal.accent } else { pal.text })));
        }
        if response.hovered() && pointer.is_some_and(|p| near_key(&anim, p).is_some()) {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }
        // The settings' keys, small, under the camera's.
        for t in anim.value_key_times() {
            let c = pos2(x_of(t), (lane + rect.bottom()) / 2.0 + 2.0);
            let r = 3.5;
            painter.add(egui::Shape::convex_polygon(vec![c + vec2(0.0, -r), c + vec2(r, 0.0), c + vec2(0.0, r), c + vec2(-r, 0.0)], pal.weak, Stroke::NONE));
        }
        let x = x_of(pb.playhead);
        painter.vline(x, rect.top()..=rect.bottom(), Stroke::new(2.0, pal.accent));
        painter.add(egui::Shape::convex_polygon(vec![pos2(x - 5.0, rect.top()), pos2(x + 5.0, rect.top()), pos2(x, rect.top() + 7.0)], pal.accent, Stroke::NONE));
        if anim.keys.is_empty() {
            painter.text(pos2(rect.center().x, lane), Align2::CENTER_CENTER, tr!("Place the camera, then Add keyframe; move the playhead and add another"), FontId::proportional(11.5), pal.weak);
        }

        // The view follows the playhead, once moved.
        let moved = pb.applied != Some(pb.playhead);
        let follow = moved.then(|| anim.sample(pb.playhead)).flatten();
        let at = pb.playhead;
        pb.applied = Some(pb.playhead);
        if let Some(pose) = follow {
            self.put(&pose);
            ui.ctx().request_repaint();
        }
        if moved && !anim.tracks.is_empty() {
            apply_tracks(&anim, at, &mut self.light, &mut self.time);
            ui.ctx().request_repaint();
        }
        if anim != before {
            doc.world.meta.animation = (!anim.is_empty() || anim.duration != Animation::default().duration || anim.fps != Animation::default().fps).then_some(anim);
            doc.mark_changed();
        }
    }
}

/// How far from the point the camera looks at the Render builds the map,
/// loading around the view (the views' view distance), in blocks; None
/// with the whole map.
fn render_reach() -> Option<f64> {
    let load = crate::graphics::loading();
    (!load.whole).then_some(load.distance as f64)
}

/// The tiles of the map in `extent`, nearest `target` first.
fn wanted(doc: &Document, extent: &Extent, target: [f64; 3]) -> Vec<TilePos> {
    let n = massif_render::map_source::TILE_CHUNKS;
    let (x0, z0) = (extent.x0.div_euclid(n), extent.z0.div_euclid(n));
    let (x1, z1) = (x0 + extent.width as i32 / n, z0 + extent.depth as i32 / n);
    tiles_by_distance(&doc.world, doc.dim, target[0], target[2]).into_iter().filter(|p| (x0..x1).contains(&p.x) && (z0..z1).contains(&p.z)).collect()
}

/// The whole map, seen from above its south-east, a little high; loading
/// around the view, its middle within `reach`.
fn framing(map: &Extent, reach: Option<f64>, sea: f64) -> Orbit {
    let c = map.center();
    let extent = reach.and_then(|r| map.around(c[0], c[2], r)).unwrap_or(*map);
    let span = extent.width.max(extent.depth) as f64 * 16.0;
    Orbit { target: [c[0], sea, c[2]], yaw: 0.65, pitch: -0.55, distance: span * 0.8 }
}

/// How fast to walk with something `depth` blocks ahead (or below): a
/// share of that distance a second, so that two blocks over the grass and
/// two kilometres over the map are both in hand; times `factor` (the wheel
/// while walking), Shift four times, Alt a quarter.
fn walk_speed(depth: f64, factor: f64, fast: bool, slow: bool) -> f64 {
    depth.clamp(2.0, 6000.0) * 0.8 * factor * if fast { 4.0 } else { 1.0 } * if slow { 0.25 } else { 1.0 }
}

#[cfg(test)]
mod nav_tests {
    use super::*;

    fn close(a: [f64; 3], b: [f64; 3], tol: f64) -> bool {
        (0..3).all(|k| (a[k] - b[k]).abs() < tol)
    }

    /// Orbiting about a point keeps it as far and in the same place on the
    /// screen (the direction to it from the eye, in the view's own frame).
    #[test]
    fn orbiting_about_a_point_keeps_it_in_place() {
        let mut o = Orbit { target: [100.0, 70.0, -40.0], yaw: 0.7, pitch: -0.4, distance: 80.0 };
        let pivot = [130.0, 64.0, -90.0];
        let rel = |o: &Orbit| {
            let e = o.eye();
            let b = o.camera(50.0).basis();
            let d = glam::DVec3::new(pivot[0] - e[0], pivot[1] - e[1], pivot[2] - e[2]);
            let f = |v: glam::Vec3| glam::DVec3::new(v.x as f64, v.y as f64, v.z as f64);
            [d.dot(f(b.right)), d.dot(f(b.up)), d.dot(f(b.forward))]
        };
        let before = rel(&o);
        o.orbit_about(pivot, 0.5, 0.2);
        assert!(close(rel(&o), before, 1e-3), "{:?} {:?}", rel(&o), before);
        assert!((o.yaw - 1.2).abs() < 1e-6 && (o.pitch + 0.2).abs() < 1e-6);
    }

    /// Zooming toward the point under the pointer keeps it under the pointer
    /// (on the same ray from the eye) and comes that much closer.
    #[test]
    fn zooming_keeps_the_point_under_the_pointer() {
        let mut o = Orbit { target: [0.0, 64.0, 0.0], yaw: 2.0, pitch: -0.5, distance: 300.0 };
        let point = [40.0, 60.0, 25.0];
        let ray = |o: &Orbit| {
            let e = o.eye();
            let d = glam::DVec3::new(point[0] - e[0], point[1] - e[1], point[2] - e[2]);
            (d.normalize(), d.length())
        };
        let (dir, far) = ray(&o);
        o.zoom_toward(point, 0.5);
        let (dir2, far2) = ray(&o);
        assert!(dir.dot(dir2) > 1.0 - 1e-9 && (far2 - far * 0.5).abs() < 1e-6);
        // At 2 blocks and at 2000, the same wheel step is the same share.
        let mut near = Orbit { target: [0.0, 64.0, 0.0], yaw: 0.0, pitch: -0.3, distance: 2.0 };
        near.zoom_toward(near.target, 0.8);
        assert!((near.distance - 1.6).abs() < 1e-9);
    }

    /// Sliding by pixels at a depth moves the view by that many pixels'
    /// worth of blocks across it.
    #[test]
    fn sliding_follows_the_pointer() {
        let mut o = Orbit { target: [0.0, 64.0, 0.0], yaw: 0.3, pitch: -0.6, distance: 50.0 };
        let eye = o.eye();
        o.pan(10.0, 0.0, 0.5);
        let moved = o.eye();
        let d = ((moved[0] - eye[0]).powi(2) + (moved[1] - eye[1]).powi(2) + (moved[2] - eye[2]).powi(2)).sqrt();
        assert!((d - 5.0).abs() < 1e-6, "{d}");
    }

    /// Walking goes a share of the distance ahead a second: slow over the
    /// grass, fast over the map; Shift and Alt as in Blender.
    #[test]
    fn walking_speed_follows_the_distance() {
        assert!(walk_speed(2.0, 1.0, false, false) < 2.0);
        assert!(walk_speed(2000.0, 1.0, false, false) > 1000.0);
        assert_eq!(walk_speed(100.0, 1.0, true, false), walk_speed(100.0, 1.0, false, false) * 4.0);
        assert_eq!(walk_speed(100.0, 1.0, false, true), walk_speed(100.0, 1.0, false, false) / 4.0);
    }
}
