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

/// The sun and the sky, and how the image is exposed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Light {
    /// Of the sun: from north toward east, and up from the horizon, radians.
    pub sun_azimuth: f32,
    pub sun_elevation: f32,
    /// The sun's angular radius, radians (the real one: 0.27°).
    pub sun_radius: f32,
    pub sun_color: [f32; 3],
    /// Irradiance of the sun on a surface facing it.
    pub sun_strength: f32,
    pub zenith: [f32; 3],
    pub horizon: [f32; 3],
    pub sky_strength: f32,
    /// Bounces of light after the first surface met.
    pub bounces: u32,
    pub exposure: f32,
}

impl Default for Light {
    fn default() -> Self {
        Self {
            sun_azimuth: 2.4,
            sun_elevation: 0.62,
            sun_radius: 0.0047,
            sun_color: [1.0, 0.94, 0.86],
            sun_strength: 5.0,
            zenith: [0.22, 0.42, 0.85],
            horizon: [0.72, 0.82, 0.95],
            sky_strength: 1.0,
            bounces: 2,
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
}
