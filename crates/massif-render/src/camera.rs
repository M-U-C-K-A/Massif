//! Where the image is seen from, and the light it is seen in.

use glam::Vec3;

/// A camera: its eye in the world's blocks, where it looks.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    pub eye: [f64; 3],
    /// Turned from north (−z) toward east (+x), radians.
    pub yaw: f32,
    /// Up from the horizon, radians.
    pub pitch: f32,
    /// Vertical field of view, radians.
    pub fov_y: f32,
}

/// The directions of a camera's view.
#[derive(Debug, Clone, Copy)]
pub struct Basis {
    pub forward: Vec3,
    pub right: Vec3,
    pub up: Vec3,
}

impl Camera {
    pub fn forward(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        Vec3::new(sy * cp, sp, -cy * cp)
    }

    pub fn basis(&self) -> Basis {
        let forward = self.forward();
        let right = forward.cross(Vec3::Y).try_normalize().unwrap_or(Vec3::X);
        let up = right.cross(forward);
        Basis { forward, right, up }
    }

    /// A camera at `distance` from `target`, looking at it from `yaw` and
    /// `pitch` (down: negative).
    pub fn orbit(target: [f64; 3], yaw: f32, pitch: f32, distance: f64, fov_y: f32) -> Self {
        let mut c = Camera { eye: target, yaw, pitch, fov_y };
        let f = c.forward();
        c.eye = [target[0] - f.x as f64 * distance, target[1] - f.y as f64 * distance, target[2] - f.z as f64 * distance];
        c
    }
}

/// The sun, the sky, the air and the lens: everything the image is lit
/// and seen through.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Light {
    /// Of the sun: from north toward east, and up from the horizon, radians.
    pub sun_azimuth: f32,
    pub sun_elevation: f32,
    /// The sun's angular radius, radians (the real one: 0.27°).
    pub sun_radius: f32,
    /// Illuminance of the sun above the air, and a tint for it.
    pub sun_strength: f32,
    pub sun_tint: [f32; 3],
    /// The sky's light, times.
    pub sky_strength: f32,
    /// The air's scattering (1: the Earth's) and the aerosols' (haze), with
    /// their anisotropy.
    pub air: f32,
    pub haze: f32,
    pub haze_g: f32,
    /// Fog: its extinction a block at its base height (world y), how many
    /// blocks it takes to thin by e above it, and its anisotropy.
    pub fog: f32,
    pub fog_height: f32,
    pub fog_falloff: f32,
    pub fog_g: f32,
    /// Shadows in the air: shafts of light through the fog and the haze.
    pub shafts: bool,
    /// A sea at sea level all around the map, out to the horizon.
    pub sea: bool,
    /// Water as water (a surface that mirrors and lets see through, its
    /// depth taking the light), or as the game's texture; its waves' slope
    /// and size (blocks), and how much light it takes (1: clear sea water).
    pub real_water: bool,
    pub waves: f32,
    pub wave_size: f32,
    pub water_clarity: f32,
    /// After rain: how wet everything is, and how much of flat ground holds
    /// puddles, of what size (blocks).
    pub wetness: f32,
    pub puddles: f32,
    pub puddle_size: f32,
    /// Seconds, for what moves: the waves.
    pub time: f32,
    /// Light of the blocks that glow, times.
    pub emission: f32,
    /// The moon's light, a share of the sun's.
    pub moon: f32,
    /// Clouds: how much of the sky they cover (0: none), the height of
    /// their base (world y), how thick and how dense they are (extinction
    /// a block), how wide their shapes (blocks), and how far the wind
    /// has moved them (blocks, x and z).
    pub cloud_cover: f32,
    pub cloud_height: f32,
    pub cloud_thickness: f32,
    pub cloud_density: f32,
    pub cloud_size: f32,
    pub wind: [f32; 2],
    /// The lens's opening (radius, blocks; 0: all sharp) and the distance
    /// in focus.
    pub aperture: f32,
    pub focus: f32,
    /// Bounces of light after the first surface met.
    pub bounces: u32,
    /// The most light a bounce brings, in its brightest channel (0: no
    /// limit): rare bright paths cut down, so that the image settles sooner.
    pub clamp: f32,
    /// The detailed grass's blades a block, and their height (blocks).
    pub grass_blades: u32,
    pub grass_height: f32,
    /// Colour management, as Blender's: the view transform, its look (see
    /// [`crate::color`]) and the display's gamma.
    pub view: u32,
    pub look: u32,
    pub gamma: f32,
    pub exposure: f32,
}

impl Default for Light {
    fn default() -> Self {
        Self {
            sun_azimuth: 2.4,
            sun_elevation: 0.62,
            sun_radius: 0.0047,
            sun_strength: 6.0,
            sun_tint: [1.0; 3],
            sky_strength: 1.0,
            air: 1.0,
            haze: 1.0,
            haze_g: 0.8,
            fog: 0.0,
            fog_height: 64.0,
            fog_falloff: 24.0,
            fog_g: 0.6,
            shafts: true,
            sea: true,
            real_water: true,
            waves: 0.12,
            wave_size: 4.0,
            water_clarity: 1.0,
            wetness: 0.0,
            puddles: 0.0,
            puddle_size: 6.0,
            time: 0.0,
            emission: 1.0,
            moon: 1.0 / 300.0,
            cloud_cover: 0.0,
            cloud_height: 900.0,
            cloud_thickness: 600.0,
            cloud_density: 0.02,
            cloud_size: 1600.0,
            wind: [0.0; 2],
            aperture: 0.0,
            focus: 100.0,
            bounces: 2,
            clamp: 10.0,
            grass_blades: 45,
            grass_height: 0.8,
            view: crate::color::AGX,
            look: 0,
            gamma: 1.0,
            exposure: 1.0,
        }
    }
}

impl Light {
    /// Toward the sun.
    pub fn sun_direction(&self) -> Vec3 {
        let (sa, ca) = self.sun_azimuth.sin_cos();
        let (se, ce) = self.sun_elevation.sin_cos();
        Vec3::new(sa * ce, se, -ca * ce).normalize()
    }

    /// What lights the scene from the sky: the sun, or once it is 6° under
    /// the horizon (past the blue hour), the moon facing it (full), bluish and some 300
    /// times dimmer. Its direction, illuminance and tint.
    /// How its light becomes the display's values.
    pub fn tone(&self) -> crate::color::Tone {
        crate::color::Tone { exposure: self.exposure, view: self.view, look: self.look, gamma: self.gamma }
    }

    pub fn source(&self) -> (Vec3, f32, [f32; 3]) {
        let sun = self.sun_direction();
        if self.sun_elevation > -6f32.to_radians() {
            return (sun, self.sun_strength, self.sun_tint);
        }
        let moon = Vec3::new(-sun.x, -sun.y, -sun.z);
        (moon, self.sun_strength * self.moon, [0.72 * self.sun_tint[0], 0.8 * self.sun_tint[1], 1.0 * self.sun_tint[2]])
    }

    /// Put the sun where it is at `hour` (local solar time) on day `day` of
    /// the year at `latitude` (degrees), the map's north turned `north`
    /// degrees from the world's.
    pub fn set_time(&mut self, hour: f32, day: f32, latitude: f32, north: f32) {
        let (azimuth, elevation) = sun_position(hour, day, latitude);
        self.sun_azimuth = azimuth + north.to_radians();
        self.sun_elevation = elevation;
    }
}

/// The sun's azimuth (from north toward east) and elevation, radians, at
/// `hour` of day `day` at `latitude` degrees: the usual approximation, good
/// to a degree or so.
pub fn sun_position(hour: f32, day: f32, latitude: f32) -> (f32, f32) {
    let declination = 23.44f32.to_radians() * (std::f32::consts::TAU * (284.0 + day) / 365.0).sin();
    let h = (15.0 * (hour - 12.0)).to_radians();
    let lat = latitude.to_radians();
    let elevation = (lat.sin() * declination.sin() + lat.cos() * declination.cos() * h.cos()).clamp(-1.0, 1.0).asin();
    let azimuth = h.sin().atan2(h.cos() * lat.sin() - declination.tan() * lat.cos()) + std::f32::consts::PI;
    (azimuth, elevation)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_orbit_looks_at_its_target() {
        let c = Camera::orbit([100.0, 64.0, -50.0], 0.7, -0.5, 200.0, 0.9);
        let f = c.forward();
        let back = [c.eye[0] + f.x as f64 * 200.0, c.eye[1] + f.y as f64 * 200.0, c.eye[2] + f.z as f64 * 200.0];
        assert!((back[0] - 100.0).abs() < 1e-3 && (back[1] - 64.0).abs() < 1e-3 && (back[2] + 50.0).abs() < 1e-3);
        assert!(c.eye[1] > 64.0, "looking down from above");
        let b = c.basis();
        assert!(b.right.dot(b.forward).abs() < 1e-5 && b.up.y > 0.0);
        assert!(Camera { eye: [0.0; 3], yaw: 0.0, pitch: 0.0, fov_y: 1.0 }.forward().z < -0.99, "yaw 0 looks north");
    }

    #[test]
    fn the_sun_rises_east_peaks_south_and_sets_west() {
        let deg = |(a, e): (f32, f32)| (a.to_degrees(), e.to_degrees());
        // Midsummer at 45° north.
        let (az, el) = deg(sun_position(12.0, 172.0, 45.0));
        assert!((az - 180.0).abs() < 1.0 && (el - 68.4).abs() < 1.0, "noon: {az} {el}");
        let (az, el) = deg(sun_position(6.0, 172.0, 45.0));
        assert!(az > 50.0 && az < 90.0 && el > 0.0 && el < 20.0, "morning: {az} {el}");
        let (az, el) = deg(sun_position(19.0, 172.0, 45.0));
        assert!(az > 270.0 && az < 320.0 && el > -5.0 && el < 10.0, "evening: {az} {el}");
        assert!(deg(sun_position(0.0, 172.0, 45.0)).1 < -15.0, "night");
    }
}
