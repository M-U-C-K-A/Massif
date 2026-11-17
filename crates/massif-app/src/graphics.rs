//! How much the graphics card is asked to do (Settings ▸ Graphics): the
//! size the 3D views are drawn at, their smoothing, the light of the
//! Terrain viewport, its sky and clouds, the water's reflections, the fog,
//! the game's textures in the 3D views, how much of a huge map the 3D
//! view keeps on the card, the map view's images there, and how often an
//! idle window draws.
//!
//! Three presets set all of it at once; a change to any one makes it
//! Custom. The views read what is current with [`get`], each frame, and
//! follow a change at once: no restart.
//!
//! What each costs, measured (an Apple M4, 2,560 × 1,440, a map of 16,384
//! blocks a side with much water; `view3d::graphics_perf` and
//! `graphics_settings_in_the_viewport`): the render scale is what saves the
//! most in the 3D view (5.6 ms a frame at 100 %, 2.9 at 67 %); smoothing,
//! fog and reflections cost no time there that could be measured, smoothing
//! 85 MB of the card's memory. Keeping the map's every block on the card
//! (High) takes 1.2 GB against 0.3 GB, and draws its water as thousands of
//! meshes: 35 to 200 ms a frame against about 6 ms with a column every two
//! blocks. In the Terrain viewport the clouds cost the most (a still frame
//! 476 ms with them, 116 ms without), then the light's steps (Low: −58 %).
//!
//! On the very first run a preset is picked from the card wgpu reports and
//! a short trial of frames (see [`suggest`]); a preset the user chose is
//! never changed. When frames stay slow for a while, a quiet note offers
//! one step lower (see [`SlowWatch`]).

use serde::{Deserialize, Serialize};
use std::sync::atomic::Ordering;

/// A set of graphics settings, by name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Preset {
    Low,
    #[default]
    Medium,
    High,
    /// Any other mix, set by hand.
    Custom,
}

impl Preset {
    pub const NAMED: [Preset; 3] = [Preset::Low, Preset::Medium, Preset::High];

    /// The preset one step lighter, if any.
    pub fn lower(self) -> Option<Preset> {
        match self {
            Preset::High => Some(Preset::Medium),
            Preset::Medium | Preset::Custom => Some(Preset::Low),
            Preset::Low => None,
        }
    }
}

/// How finely the Terrain viewport works its light out: the steps marched
/// toward the sun for shadows, through the clouds, and around each point
/// for how much sky it sees.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Light {
    /// The steps it takes while the view moves, still too.
    Low,
    #[default]
    Medium,
    High,
}

/// What the graphics card is asked to do.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Graphics {
    /// The side of the 3D views' images, as a share of their pixels (the
    /// image is stretched to fit). While a view moves, it is smaller still.
    pub render_scale: f32,
    /// Edges smoothed: four samples a pixel.
    pub msaa: bool,
    /// The Terrain viewport's shadows and the sky each point sees.
    pub light: Light,
    /// The Terrain viewport's clouds, and the game's over the player's view.
    pub clouds: bool,
    /// Water mirrors the sky and glints in the sun.
    pub reflections: bool,
    /// The land fades into the distance.
    pub fog: bool,
    /// The game's own textures on the blocks of the 3D views: in the
    /// player's view, and in the 3D View and 3D Sculpt editors' Terrain look
    /// (a flat colour a block without).
    pub textures: bool,
    /// Pages of 1,024 × 1,024 columns the 3D view keeps on the card (about
    /// 4.8 MB each): a larger map is drawn with a column every 2, 4, 8…
    /// blocks.
    pub gpu_pages: u32,
    /// Images of 128 × 128 pixels the map view keeps on the card (64 kB
    /// each), at most.
    pub map_images: u32,
    /// Frames a second at most while nothing is touched (0: no limit).
    pub idle_fps: u32,
}

impl Default for Graphics {
    fn default() -> Self {
        Self::of(Preset::Medium)
    }
}

/// Bytes of one page of the 3D view's tracer, its pyramid included.
pub const PAGE_BYTES: u64 = 4_980_000;
/// Bytes of one image of the map view on the card.
pub const MAP_IMAGE_BYTES: u64 = 64 << 10;

impl Graphics {
    /// The settings of a named preset (Custom: Medium's, to start from).
    pub fn of(preset: Preset) -> Self {
        match preset {
            Preset::Low => Self {
                render_scale: 0.67,
                msaa: false,
                light: Light::Low,
                clouds: false,
                reflections: true,
                fog: true,
                textures: false,
                gpu_pages: 64,
                map_images: 2048,
                idle_fps: 20,
            },
            Preset::Medium | Preset::Custom => Self {
                render_scale: 1.0,
                msaa: true,
                light: Light::Medium,
                clouds: true,
                reflections: true,
                fog: true,
                textures: true,
                gpu_pages: 160,
                map_images: 8192,
                idle_fps: 30,
            },
            Preset::High => Self {
                render_scale: 1.0,
                msaa: true,
                light: Light::High,
                clouds: true,
                reflections: true,
                fog: true,
                textures: true,
                gpu_pages: 400,
                map_images: 32768,
                idle_fps: 0,
            },
        }
    }

    /// The named preset these settings are, or Custom.
    pub fn preset(&self) -> Preset {
        Preset::NAMED.into_iter().find(|p| Self::of(*p) == *self).unwrap_or(Preset::Custom)
    }

    /// Samples a pixel of the 3D views.
    pub fn samples(&self) -> u32 {
        if self.msaa { 4 } else { 1 }
    }

    /// Settings an earlier version kept, as this one has them: before 1.0
    /// only the player's view had the game's textures, and Low kept them;
    /// Low draws flat colours now that the 3D View has them too.
    pub fn updated(self) -> Self {
        let mut old_low = Self::of(Preset::Low);
        old_low.textures = true;
        if self == old_low { Self::of(Preset::Low) } else { self }
    }

    /// The same, kept within what the settings offer (a file edited by hand).
    pub fn sane(mut self) -> Self {
        self.render_scale = if self.render_scale.is_finite() { self.render_scale.clamp(0.4, 1.0) } else { 1.0 };
        self.gpu_pages = self.gpu_pages.clamp(16, 1024);
        self.map_images = self.map_images.clamp(256, 32768);
        self.idle_fps = if self.idle_fps == 0 { 0 } else { self.idle_fps.clamp(5, 240) };
        self
    }
}

#[cfg(not(test))]
static CURRENT: std::sync::RwLock<Option<Graphics>> = std::sync::RwLock::new(None);

// Each test on its own thread has its own settings: tests that change them
// do not change how the others draw.
#[cfg(test)]
thread_local! {
    static CURRENT: std::cell::Cell<Option<Graphics>> = const { std::cell::Cell::new(None) };
}

/// What the views draw with now.
pub fn get() -> Graphics {
    #[cfg(test)]
    return CURRENT.with(|c| c.get()).unwrap_or_default();
    #[cfg(not(test))]
    CURRENT.read().ok().and_then(|g| *g).unwrap_or_default()
}

/// Draw with `g` from the next frame on.
pub fn set(g: Graphics) {
    #[cfg(test)]
    CURRENT.with(|c| c.set(Some(g)));
    #[cfg(not(test))]
    if let Ok(mut c) = CURRENT.write() {
        *c = Some(g);
    }
}

static DREW_3D: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// A 3D view drew this frame (for the watch on slow frames).
pub fn drew_3d() {
    DREW_3D.store(true, Ordering::Relaxed);
}

/// Whether a 3D view drew since the last call.
pub fn take_drew_3d() -> bool {
    DREW_3D.swap(false, Ordering::Relaxed)
}


// ------------------------------------------------------------ first run --

/// What wgpu says of the graphics card.
#[derive(Debug, Clone, PartialEq)]
pub struct Card {
    pub name: String,
    pub device_type: wgpu::DeviceType,
    pub backend: wgpu::Backend,
}

impl Card {
    pub fn of(info: &wgpu::AdapterInfo) -> Self {
        Self { name: info.name.clone(), device_type: info.device_type, backend: info.backend }
    }
}

/// The preset for a card, from what wgpu says of it and `probe_ms`, the
/// time a trial frame took (None: not timed).
///
/// - OpenGL, a software renderer or an old integrated chip: Low.
/// - Other integrated chips (Apple's, Intel Arc/Iris Xe, AMD's APUs):
///   Medium, Low if the trial was slow.
/// - A discrete card: High, Medium if the trial was slow, Low if very slow.
pub fn suggest(card: &Card, probe_ms: Option<f32>) -> Preset {
    use wgpu::DeviceType as D;
    let name = card.name.to_lowercase();
    // Intel's HD and UHD Graphics, and others of their age, before Iris Xe.
    let old_integrated = ["hd graphics", "uhd graphics", "hd 4", "hd 5", "hd 6", "radeon(tm) r", "mali", "adreno", "llvmpipe", "swiftshader", "microsoft basic"].iter().any(|n| name.contains(n));
    let slow = |ms: f32| probe_ms.is_some_and(|p| p > ms);
    if matches!(card.device_type, D::Cpu) || card.backend == wgpu::Backend::Gl || old_integrated {
        return Preset::Low;
    }
    match card.device_type {
        D::DiscreteGpu => {
            if slow(40.0) {
                Preset::Low
            } else if slow(14.0) {
                Preset::Medium
            } else {
                Preset::High
            }
        }
        _ => {
            if slow(20.0) {
                Preset::Low
            } else {
                Preset::Medium
            }
        }
    }
}

// --------------------------------------------------------- slow frames --

/// Watches how long the frames of the 3D views take, and says when they
/// have stayed slow long enough to suggest a lighter preset: once a run at
/// most, and never while the user is busy changing the settings.
#[derive(Debug, Default)]
pub struct SlowWatch {
    /// Seconds of views drawn, and of those slow.
    seen: f64,
    slow: f64,
    /// Said already this run.
    pub said: bool,
}

/// A frame longer than this counts as slow (under 20 frames a second).
const SLOW_FRAME: f64 = 0.05;
/// Seconds of drawing watched before anything is said.
const WATCH: f64 = 20.0;

impl SlowWatch {
    /// A frame of a 3D view took `dt` seconds. Returns true the moment the
    /// note should show.
    pub fn frame(&mut self, dt: f64) -> bool {
        if self.said || !(0.0..1.0).contains(&dt) {
            return false;
        }
        self.seen += dt;
        if dt > SLOW_FRAME {
            self.slow += dt;
        }
        if self.seen >= WATCH {
            // Three quarters of the time in slow frames.
            let tell = self.slow > self.seen * 0.75;
            self.seen = 0.0;
            self.slow = 0.0;
            if tell {
                self.said = true;
                return true;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_are_told_apart() {
        for p in Preset::NAMED {
            assert_eq!(Graphics::of(p).preset(), p);
            assert_eq!(Graphics::of(p).sane(), Graphics::of(p));
        }
        let mut g = Graphics::of(Preset::High);
        g.fog = false;
        assert_eq!(g.preset(), Preset::Custom);
    }

    #[test]
    fn low_draws_flat_colours_from_now_on() {
        let low = Graphics::of(Preset::Low);
        assert!(!low.textures && Graphics::of(Preset::Medium).textures && Graphics::of(Preset::High).textures);
        // Low as an earlier version kept it is Low still; anything else stays.
        let old = Graphics { textures: true, ..low };
        assert_eq!(old.preset(), Preset::Custom);
        assert_eq!(old.updated(), low);
        let custom = Graphics { textures: true, fog: false, ..low };
        assert_eq!(custom.updated(), custom);
        assert_eq!(Graphics::of(Preset::Medium).updated(), Graphics::of(Preset::Medium));
    }

    #[test]
    fn cards_get_a_preset() {
        let card = |name: &str, t, b| Card { name: name.into(), device_type: t, backend: b };
        use wgpu::{Backend as B, DeviceType as D};
        assert_eq!(suggest(&card("Intel(R) HD Graphics 4600", D::IntegratedGpu, B::Gl), None), Preset::Low);
        assert_eq!(suggest(&card("Intel(R) HD Graphics 4600", D::IntegratedGpu, B::Vulkan), None), Preset::Low);
        assert_eq!(suggest(&card("Apple M4", D::IntegratedGpu, B::Metal), Some(6.0)), Preset::Medium);
        assert_eq!(suggest(&card("AMD Radeon RX 9070 XT", D::DiscreteGpu, B::Vulkan), Some(3.0)), Preset::High);
        assert_eq!(suggest(&card("AMD Radeon RX 6400", D::DiscreteGpu, B::Vulkan), Some(20.0)), Preset::Medium);
        assert_eq!(suggest(&card("llvmpipe", D::Cpu, B::Vulkan), None), Preset::Low);
    }

    #[test]
    fn slow_frames_are_told_once() {
        let mut w = SlowWatch::default();
        let mut told = 0;
        for _ in 0..2000 {
            told += w.frame(0.016) as u32;
        }
        assert_eq!(told, 0);
        for _ in 0..2000 {
            told += w.frame(0.08) as u32;
        }
        assert_eq!(told, 1);
    }
}
