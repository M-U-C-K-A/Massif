//! Camera paths: the keyframes of an animation of the Render side, kept
//! with the map.
//!
//! A keyframe holds where the camera is and what it looks at, its field
//! of view and the hour of the day, at a time in seconds. Between them,
//! positions follow a centripetal Catmull-Rom spline (smooth through every
//! key, without loops or overshoot where keys bunch up), the field of view
//! and the hour ease from one key to the next. [`Animation::sample`] gives
//! the pose at any time, which is all a renderer needs to draw frame `n`
//! at `n / fps` seconds: rendering a sequence of images is that loop.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The pose of the camera and the sun at a time.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Pose {
    /// Where the camera is, in blocks.
    pub eye: [f64; 3],
    /// The point it looks at, in blocks.
    pub target: [f64; 3],
    /// Vertical field of view, degrees.
    pub fov: f32,
    /// Local solar time, hours (0..24).
    pub hour: f32,
}

/// A keyframe: a pose at a time.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Key {
    /// Seconds from the start.
    pub time: f32,
    pub pose: Pose,
}

/// An animation: keyframes in the order of their times, how long it runs
/// and how many frames a second it is drawn at.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Animation {
    pub keys: Vec<Key>,
    /// Seconds.
    pub duration: f32,
    pub fps: u32,
    /// Keyframes of settings, as Blender keys any property: by the
    /// setting's name, `[time, value]` in the order of their times. Between
    /// two keys a value eases from one to the other; before the first and
    /// after the last it holds.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tracks: BTreeMap<String, Vec<[f32; 2]>>,
}

impl Default for Animation {
    fn default() -> Self {
        Self { keys: Vec::new(), duration: 10.0, fps: 30, tracks: BTreeMap::new() }
    }
}

/// The shortest time between two keyframes: closer, the later replaces the
/// earlier.
pub const MIN_GAP: f32 = 1.0 / 120.0;

impl Animation {
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty() && self.tracks.is_empty()
    }

    /// Key setting `name` at `time` to `value` (the key there already, if
    /// any, takes it). The animation grows to hold it.
    pub fn key_value(&mut self, name: &str, time: f32, value: f32) {
        let time = time.max(0.0);
        let tolerance = (0.5 / self.fps.max(1) as f32).max(MIN_GAP);
        let track = self.tracks.entry(name.to_owned()).or_default();
        match track.iter().position(|k| (k[0] - time).abs() <= tolerance) {
            Some(k) => track[k][1] = value,
            None => {
                let at = track.partition_point(|k| k[0] < time);
                track.insert(at, [time, value]);
            }
        }
        self.duration = self.duration.max(time);
    }

    /// Take setting `name`'s key at `time` away; whether there was one.
    pub fn unkey_value(&mut self, name: &str, time: f32) -> bool {
        let tolerance = (0.5 / self.fps.max(1) as f32).max(MIN_GAP);
        let Some(track) = self.tracks.get_mut(name) else { return false };
        let Some(k) = track.iter().position(|k| (k[0] - time).abs() <= tolerance) else { return false };
        track.remove(k);
        if track.is_empty() {
            self.tracks.remove(name);
        }
        true
    }

    /// Whether setting `name` has a key at `time`.
    pub fn has_value_key(&self, name: &str, time: f32) -> bool {
        let tolerance = (0.5 / self.fps.max(1) as f32).max(MIN_GAP);
        self.tracks.get(name).is_some_and(|t| t.iter().any(|k| (k[0] - time).abs() <= tolerance))
    }

    /// Setting `name`'s value at `time`, if it is keyed: eased between its
    /// keys, held before the first and after the last.
    pub fn value(&self, name: &str, time: f32) -> Option<f32> {
        let track = self.tracks.get(name)?;
        let (first, last) = (track.first()?, track.last()?);
        if time <= first[0] {
            return Some(first[1]);
        }
        if time >= last[0] {
            return Some(last[1]);
        }
        let i = track.partition_point(|k| k[0] <= time).clamp(1, track.len() - 1) - 1;
        let (a, b) = (track[i], track[i + 1]);
        let t = smooth(((time - a[0]) / (b[0] - a[0]).max(1e-6)).clamp(0.0, 1.0));
        Some(a[1] + (b[1] - a[1]) * t)
    }

    /// The times of every setting's keys, each once, in order.
    pub fn value_key_times(&self) -> Vec<f32> {
        let mut times: Vec<f32> = self.tracks.values().flatten().map(|k| k[0]).collect();
        times.sort_by(f32::total_cmp);
        times.dedup_by(|a, b| (*a - *b).abs() < MIN_GAP);
        times
    }

    /// How many frames it is drawn in.
    pub fn frames(&self) -> u32 {
        (self.duration * self.fps.max(1) as f32).round().max(1.0) as u32
    }

    /// The time of frame `n`.
    pub fn time_of(&self, frame: u32) -> f32 {
        frame as f32 / self.fps.max(1) as f32
    }

    /// The keyframe at `time` (within half a frame), if any.
    pub fn key_at(&self, time: f32) -> Option<usize> {
        let tolerance = (0.5 / self.fps.max(1) as f32).max(MIN_GAP);
        self.keys.iter().position(|k| (k.time - time).abs() <= tolerance)
    }

    /// Put a keyframe at `time`: the one there already takes the new pose.
    /// Returns its index. The animation grows to hold it.
    pub fn set(&mut self, time: f32, pose: Pose) -> usize {
        let time = time.max(0.0);
        if let Some(k) = self.key_at(time) {
            self.keys[k].pose = pose;
            return k;
        }
        let at = self.keys.partition_point(|k| k.time < time);
        self.keys.insert(at, Key { time, pose });
        self.duration = self.duration.max(time);
        at
    }

    /// Remove keyframe `index`.
    pub fn remove(&mut self, index: usize) -> Option<Key> {
        (index < self.keys.len()).then(|| self.keys.remove(index))
    }

    /// Move keyframe `index` to `time` (kept in order, clear of its
    /// neighbours); returns its new index.
    pub fn move_key(&mut self, index: usize, time: f32) -> usize {
        if index >= self.keys.len() {
            return index;
        }
        let key = self.keys.remove(index);
        let time = time.clamp(0.0, self.duration.max(0.0));
        if self.key_at(time).is_some() {
            // Onto another: it stays where it was.
            self.keys.insert(index, key);
            return index;
        }
        let at = self.keys.partition_point(|k| k.time < time);
        self.keys.insert(at, Key { time, ..key });
        at
    }

    /// The keyframe before `time` and the one after, by index.
    pub fn neighbours(&self, time: f32) -> (Option<usize>, Option<usize>) {
        let eps = MIN_GAP * 0.5;
        let before = self.keys.iter().rposition(|k| k.time < time - eps);
        let after = self.keys.iter().position(|k| k.time > time + eps);
        (before, after)
    }

    /// The pose at `time`: none without keyframes; the first one's before
    /// it, the last one's after it.
    pub fn sample(&self, time: f32) -> Option<Pose> {
        let keys = &self.keys;
        let (first, last) = (keys.first()?, keys.last()?);
        if keys.len() == 1 || time <= first.time {
            return Some(first.pose);
        }
        if time >= last.time {
            return Some(last.pose);
        }
        let i = keys.partition_point(|k| k.time <= time).clamp(1, keys.len() - 1) - 1;
        let (k1, k2) = (&keys[i], &keys[i + 1]);
        let span = (k2.time - k1.time).max(1e-6);
        let t = ((time - k1.time) / span).clamp(0.0, 1.0);
        // The keys around, mirrored at the ends so the path starts and ends
        // heading where it goes.
        let k0 = if i > 0 { keys[i - 1].pose } else { mirror(&k2.pose, &k1.pose) };
        let k3 = if i + 2 < keys.len() { keys[i + 2].pose } else { mirror(&k1.pose, &k2.pose) };
        let eased = smooth(t);
        Some(Pose {
            eye: catmull_rom(k0.eye, k1.pose.eye, k2.pose.eye, k3.eye, t as f64),
            target: catmull_rom(k0.target, k1.pose.target, k2.pose.target, k3.target, t as f64),
            fov: lerp(k1.pose.fov, k2.pose.fov, eased),
            hour: lerp_hour(k1.pose.hour, k2.pose.hour, eased),
        })
    }
}

/// `b` reflected through `a`: a key before the first (or after the last).
fn mirror(b: &Pose, a: &Pose) -> Pose {
    let m = |b: [f64; 3], a: [f64; 3]| [2.0 * a[0] - b[0], 2.0 * a[1] - b[1], 2.0 * a[2] - b[2]];
    Pose { eye: m(b.eye, a.eye), target: m(b.target, a.target), ..*a }
}

fn smooth(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// From hour `a` to hour `b` the short way round midnight.
fn lerp_hour(a: f32, b: f32, t: f32) -> f32 {
    let mut d = b - a;
    if d > 12.0 {
        d -= 24.0;
    } else if d < -12.0 {
        d += 24.0;
    }
    (a + d * t).rem_euclid(24.0)
}

/// Centripetal Catmull-Rom between `p1` and `p2` (Barry and Goldman's
/// pyramid, alpha 0.5), at `t` in 0..1.
fn catmull_rom(p0: [f64; 3], p1: [f64; 3], p2: [f64; 3], p3: [f64; 3], t: f64) -> [f64; 3] {
    let dist = |a: [f64; 3], b: [f64; 3]| ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt().sqrt().max(1e-4);
    let t0 = 0.0;
    let t1 = t0 + dist(p0, p1);
    let t2 = t1 + dist(p1, p2);
    let t3 = t2 + dist(p2, p3);
    let u = t1 + (t2 - t1) * t;
    let mix = |a: [f64; 3], b: [f64; 3], ta: f64, tb: f64| {
        let (wa, wb) = ((tb - u) / (tb - ta), (u - ta) / (tb - ta));
        [a[0] * wa + b[0] * wb, a[1] * wa + b[1] * wb, a[2] * wa + b[2] * wb]
    };
    let a1 = mix(p0, p1, t0, t1);
    let a2 = mix(p1, p2, t1, t2);
    let a3 = mix(p2, p3, t2, t3);
    let b1 = mix(a1, a2, t0, t2);
    let b2 = mix(a2, a3, t1, t3);
    mix(b1, b2, t1, t2)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pose(x: f64, fov: f32, hour: f32) -> Pose {
        Pose { eye: [x, 100.0, 0.0], target: [x, 60.0, -50.0], fov, hour }
    }

    #[test]
    fn keys_stay_in_order_and_replace_each_other() {
        let mut a = Animation::default();
        assert!(a.sample(1.0).is_none());
        a.set(4.0, pose(40.0, 50.0, 12.0));
        a.set(0.0, pose(0.0, 50.0, 12.0));
        a.set(2.0, pose(20.0, 50.0, 12.0));
        assert_eq!(a.keys.iter().map(|k| k.time).collect::<Vec<_>>(), [0.0, 2.0, 4.0]);
        a.set(2.01, pose(25.0, 50.0, 12.0));
        assert_eq!(a.keys.len(), 3, "within half a frame: the same key");
        assert_eq!(a.keys[1].pose.eye[0], 25.0);
        a.set(14.0, pose(0.0, 50.0, 12.0));
        assert_eq!(a.duration, 14.0, "grows to hold a key");
        assert_eq!(a.move_key(3, 1.0), 1);
        assert_eq!(a.keys[1].time, 1.0);
        assert_eq!(a.move_key(1, 2.0), 1, "not onto another key");
        assert!(a.remove(1).is_some());
        assert_eq!(a.keys.len(), 3);
        assert_eq!(a.neighbours(2.0), (Some(0), Some(2)));
    }

    #[test]
    fn the_path_goes_through_every_key_smoothly() {
        let mut a = Animation::default();
        for (t, x, fov, hour) in [(0.0, 0.0, 40.0, 23.0), (2.0, 100.0, 60.0, 1.0), (3.0, 110.0, 60.0, 2.0), (6.0, 400.0, 30.0, 6.0)] {
            a.set(t, pose(x, fov, hour));
        }
        for k in &a.keys {
            let p = a.sample(k.time).unwrap();
            assert!((p.eye[0] - k.pose.eye[0]).abs() < 1e-6 && p.fov == k.pose.fov);
        }
        assert_eq!(a.sample(-1.0).unwrap(), a.keys[0].pose);
        assert_eq!(a.sample(9.0).unwrap(), a.keys[3].pose);
        // Midnight crossed the short way.
        let h = a.sample(1.0).unwrap().hour;
        assert!(!(1.0..23.0).contains(&h), "{h}");
        // No jumps: frame to frame the camera moves a little.
        let mut last = a.sample(0.0).unwrap();
        for f in 1..=a.frames().min(180) {
            let p = a.sample(a.time_of(f)).unwrap();
            assert!((p.eye[0] - last.eye[0]).abs() < 12.0, "frame {f}");
            assert!(p.eye[0] >= last.eye[0] - 1e-6, "no overshoot backwards at frame {f}");
            last = p;
        }
        assert_eq!(a.frames(), 300);
        // Saved with the map and read back.
        let json = serde_json::to_string(&a).unwrap();
        assert_eq!(serde_json::from_str::<Animation>(&json).unwrap(), a);
    }

    /// A setting keyed at two times eases between them and holds outside;
    /// a key again at the same time replaces it; the setting saves with the
    /// map and an animation without them reads as before.
    #[test]
    fn settings_are_keyed_and_eased() {
        let mut a = Animation::default();
        a.key_value("cloud_cover", 2.0, 0.2);
        a.key_value("cloud_cover", 6.0, 0.8);
        assert_eq!(a.value("cloud_cover", 0.0), Some(0.2));
        assert_eq!(a.value("cloud_cover", 9.0), Some(0.8));
        assert!((a.value("cloud_cover", 4.0).unwrap() - 0.5).abs() < 1e-6, "halfway, eased");
        assert!(a.value("cloud_cover", 2.5).unwrap() < 0.2 + 0.6 * 0.125, "eases in");
        a.key_value("cloud_cover", 2.0 + MIN_GAP * 0.5, 0.3);
        assert_eq!(a.tracks["cloud_cover"].len(), 2, "the key there takes the new value");
        assert!(a.has_value_key("cloud_cover", 2.0) && !a.has_value_key("cloud_cover", 3.0));
        assert_eq!(a.value("haze", 1.0), None);
        a.key_value("haze", 6.0, 2.0);
        assert_eq!(a.value_key_times(), vec![2.0, 6.0]);
        assert!(a.unkey_value("haze", 6.0) && !a.tracks.contains_key("haze"));
        let json = serde_json::to_string(&a).unwrap();
        assert_eq!(serde_json::from_str::<Animation>(&json).unwrap(), a);
        let old: Animation = serde_json::from_str(r#"{"keys":[],"duration":10.0,"fps":30}"#).unwrap();
        assert!(old.tracks.is_empty());
    }
}
