//! Walking through the 3D Sculpt view as in Minecraft: W A S D walk, Space
//! jumps (twice quickly: flies, and twice again: falls back), Q sneaks
//! (flying: goes down), Shift runs (or W twice). The body is Minecraft's
//! (0.6 × 1.8 blocks, its eyes at 1.62) and bumps into the blocks of the map
//! as composed: levels and sculpted matter, with water to swim in.

use massif_core::compose::{Span, TileStack};
use massif_core::coords::{TilePos, local_index};
use massif_core::world::Dimension;
use std::collections::HashMap;

/// Minecraft's numbers, in blocks and seconds.
pub const WIDTH: f64 = 0.6;
pub const HEIGHT: f64 = 1.8;
pub const SNEAK_HEIGHT: f64 = 1.5;
pub const EYE: f64 = 1.62;
pub const SNEAK_EYE: f64 = 1.27;
const WALK: f64 = 4.317;
const RUN: f64 = 5.612;
const SNEAK: f64 = 1.295;
const SWIM: f64 = 2.2;
const FLY: f64 = 10.92;
const FLY_RUN: f64 = 21.6;
const FLY_UP: f64 = 7.5;
const GRAVITY: f64 = 32.0;
/// Up from the ground at this speed: a jump of about 1.24 blocks, over one.
const JUMP: f64 = 8.9;
const TERMINAL: f64 = 78.4;
/// Two presses within this long (Minecraft's 7 ticks): fly or fall back
/// (Space), run (W).
const DOUBLE_TAP: f64 = 0.35;
/// The longest step of the simulation; a longer frame is cut in steps.
const STEP: f64 = 1.0 / 120.0;
const EPS: f64 = 1e-7;

/// The keys held (and pressed) for a frame.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Keys {
    /// Forward (W) minus back (S).
    pub forward: f64,
    /// Right (D) minus left (A).
    pub right: f64,
    /// W went down this frame (not a repeat): twice quickly runs.
    pub forward_pressed: bool,
    /// Space held: jump, swim up, fly up.
    pub jump: bool,
    /// Space went down this frame (not a repeat).
    pub jump_pressed: bool,
    /// Q: sneak, or down while flying.
    pub sneak: bool,
    /// Shift: run.
    pub run: bool,
}

/// What the body bumps into and swims in.
pub trait Blocks {
    fn solid(&mut self, x: i32, y: i32, z: i32) -> bool;
    fn fluid(&mut self, x: i32, y: i32, z: i32) -> bool;
    /// Below this nothing holds: the player falls back where they stood.
    fn bottom(&self) -> i32;
}

/// The blocks of a map, column by column as asked.
pub struct MapBlocks<'a> {
    dim: &'a Dimension,
    stacks: HashMap<TilePos, Option<TileStack<'a>>>,
    columns: HashMap<(i32, i32), Column>,
}

/// A column of the map: its solid and its fluid intervals `[y0, y1)`.
#[derive(Default)]
struct Column {
    solid: Vec<(i32, i32)>,
    fluid: Vec<(i32, i32)>,
}

impl<'a> MapBlocks<'a> {
    pub fn new(dim: &'a Dimension) -> Self {
        Self {
            dim,
            stacks: HashMap::new(),
            columns: HashMap::new(),
        }
    }

    fn column(&mut self, x: i32, z: i32) -> &Column {
        let (dim, stacks) = (self.dim, &mut self.stacks);
        self.columns.entry((x, z)).or_insert_with(|| {
            let tile = TilePos::containing(x, z);
            let mut column = Column::default();
            if let Some(stack) = stacks.entry(tile).or_insert_with(|| TileStack::new(dim, tile, false)) {
                let cell = local_index(x, z);
                let mut spans: Vec<Span> = Vec::new();
                stack.spans(cell, &mut spans);
                stack.fluids(cell, &spans, &mut column.fluid);
                column.solid = spans.iter().map(|s| (s.y0, s.y1)).collect();
            }
            column
        })
    }

    /// The top of the highest solid block of column `(x, z)`.
    pub fn top(&mut self, x: i32, z: i32) -> Option<i32> {
        self.column(x, z).solid.iter().map(|s| s.1).max()
    }
}

impl Blocks for MapBlocks<'_> {
    fn solid(&mut self, x: i32, y: i32, z: i32) -> bool {
        self.column(x, z).solid.iter().any(|&(a, b)| a <= y && y < b)
    }

    fn fluid(&mut self, x: i32, y: i32, z: i32) -> bool {
        self.column(x, z).fluid.iter().any(|&(a, b)| a <= y && y < b)
    }

    fn bottom(&self) -> i32 {
        self.dim.min_y
    }
}

/// Someone walking through the map.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Player {
    /// The middle of the soles, in blocks.
    pub feet: [f64; 3],
    /// Blocks a second.
    pub velocity: [f64; 3],
    pub flying: bool,
    pub on_ground: bool,
    pub sneaking: bool,
    pub swimming: bool,
    pub running: bool,
    /// Walking into a step of one block jumps onto it (Minecraft's
    /// Auto-Jump).
    pub auto_jump: bool,
    /// The height of the eyes over the feet, easing as the body crouches.
    eye: f64,
    /// Seconds since Space, and W, last went down.
    since_jump: f64,
    since_forward: f64,
    /// Where the player last stood, to come back to after a fall.
    safe: [f64; 3],
}

impl Player {
    pub fn new(feet: [f64; 3]) -> Self {
        Self {
            feet,
            velocity: [0.0; 3],
            flying: false,
            on_ground: false,
            sneaking: false,
            swimming: false,
            running: false,
            auto_jump: true,
            eye: EYE,
            since_jump: f64::INFINITY,
            since_forward: f64::INFINITY,
            safe: feet,
        }
    }

    /// Where the eyes are.
    pub fn eye(&self) -> [f64; 3] {
        [self.feet[0], self.feet[1] + self.eye, self.feet[2]]
    }

    /// The box of the body standing (or crouching) at `feet`.
    fn body(&self, feet: [f64; 3], sneaking: bool) -> ([f64; 3], [f64; 3]) {
        let h = if sneaking { SNEAK_HEIGHT } else { HEIGHT };
        let w = WIDTH / 2.0;
        ([feet[0] - w, feet[1], feet[2] - w], [feet[0] + w, feet[1] + h, feet[2] + w])
    }

    /// Whether moving: something to draw again.
    pub fn moving(&self) -> bool {
        !self.on_ground && !self.flying || self.velocity.iter().any(|v| v.abs() > 1e-3) || (self.eye - self.target_eye()).abs() > 1e-3
    }

    fn target_eye(&self) -> f64 {
        if self.sneaking { SNEAK_EYE } else { EYE }
    }

    /// Live `dt` seconds with `keys` held, facing `yaw` (0 looks north,
    /// toward −z; turning right lowers it).
    pub fn step(&mut self, blocks: &mut impl Blocks, keys: Keys, yaw: f32, dt: f64) {
        let dt = dt.clamp(0.0, 0.1);
        // Presses count once, in the first step.
        self.since_jump += dt;
        self.since_forward += dt;
        if keys.jump_pressed {
            if self.since_jump < DOUBLE_TAP {
                self.flying = !self.flying;
                self.velocity[1] = 0.0;
                self.since_jump = f64::INFINITY;
            } else {
                self.since_jump = 0.0;
            }
        }
        if keys.forward_pressed {
            if self.since_forward < DOUBLE_TAP {
                self.running = true;
            }
            self.since_forward = 0.0;
        }
        let n = (dt / STEP).ceil().max(1.0);
        for _ in 0..n as usize {
            self.tick(blocks, keys, yaw, dt / n);
        }
    }

    fn tick(&mut self, blocks: &mut impl Blocks, keys: Keys, yaw: f32, dt: f64) {
        // Inside matter (built over the player, or arrived there): out
        // through the top.
        let (lo, hi) = self.body(self.feet, self.sneaking);
        if overlaps(blocks, lo, hi) {
            let mut y = self.feet[1].floor();
            for _ in 0..512 {
                y += 1.0;
                let (lo, hi) = self.body([self.feet[0], y, self.feet[2]], self.sneaking);
                if !overlaps(blocks, lo, hi) {
                    break;
                }
            }
            self.feet[1] = y;
            self.velocity[1] = 0.0;
        }
        // Crouching on the ground or not; up only where there is room.
        let wants = keys.sneak && !self.flying;
        if self.sneaking && !wants {
            let (lo, hi) = self.body(self.feet, false);
            self.sneaking = overlaps(blocks, lo, hi);
        } else {
            self.sneaking = wants;
        }
        self.eye += (self.target_eye() - self.eye) * (1.0 - (-dt * 18.0).exp());
        let head = [self.feet[0].floor() as i32, (self.feet[1] + 0.4).floor() as i32, self.feet[2].floor() as i32];
        self.swimming = !self.flying && blocks.fluid(head[0], head[1], head[2]);
        if keys.forward <= 0.0 || self.sneaking {
            self.running = false;
        }
        let running = (keys.run || self.running) && keys.forward > 0.0 && !self.sneaking;

        // Across: toward what the keys ask, quickly on the ground, slowly
        // in the air.
        let (sy, cy) = (yaw.sin() as f64, yaw.cos() as f64);
        let mut wish = [-sy * keys.forward + cy * keys.right, -cy * keys.forward - sy * keys.right];
        let len = wish[0].hypot(wish[1]);
        if len > 1.0 {
            wish = [wish[0] / len, wish[1] / len];
        }
        let speed = if self.flying {
            if running { FLY_RUN } else { FLY }
        } else if self.sneaking {
            SNEAK
        } else if self.swimming {
            SWIM
        } else if running {
            RUN
        } else {
            WALK
        };
        let grip = if self.flying {
            7.0
        } else if self.on_ground {
            20.0
        } else if self.swimming {
            8.0
        } else {
            3.5
        };
        let k = 1.0 - (-grip * dt).exp();
        self.velocity[0] += (wish[0] * speed - self.velocity[0]) * k;
        self.velocity[2] += (wish[1] * speed - self.velocity[2]) * k;

        // Up and down.
        if self.flying {
            let want = (keys.jump as i32 - keys.sneak as i32) as f64 * FLY_UP;
            self.velocity[1] += (want - self.velocity[1]) * (1.0 - (-10.0 * dt).exp());
        } else if self.swimming {
            let want = if keys.jump { 3.4 } else { -2.0 };
            self.velocity[1] += (want - self.velocity[1]) * (1.0 - (-4.0 * dt).exp());
        } else if self.on_ground && keys.jump {
            // Held, it jumps again on landing.
            self.velocity[1] = JUMP;
            self.on_ground = false;
        } else {
            self.velocity[1] = (self.velocity[1] - GRAVITY * dt).max(-TERMINAL);
        }

        // Move, an axis at a time, stopping at the blocks.
        let delta = [self.velocity[0] * dt, self.velocity[1] * dt, self.velocity[2] * dt];
        let (lo, hi) = self.body(self.feet, self.sneaking);
        let dy = sweep(blocks, lo, hi, 1, delta[1]);
        self.feet[1] += dy;
        let landed = delta[1] < 0.0 && dy > delta[1];
        if dy != delta[1] {
            self.velocity[1] = 0.0;
        }
        self.on_ground = landed || (delta[1] == 0.0 && self.standing(blocks, self.feet));
        if landed && self.flying {
            // Flying down to the ground lands.
            self.flying = false;
        }
        for axis in [0, 2] {
            let d = delta[axis];
            if d == 0.0 {
                continue;
            }
            let (lo, hi) = self.body(self.feet, self.sneaking);
            let mut moved = sweep(blocks, lo, hi, axis, d);
            // Crouched, the player does not step off an edge.
            if self.sneaking && self.on_ground {
                let mut to = self.feet;
                to[axis] += moved;
                if !self.standing(blocks, to) {
                    moved = 0.0;
                }
            }
            if moved != d {
                // A step of one block, walked into: up onto it, if there
                // is room.
                if self.auto_jump && self.on_ground && !self.flying && !self.sneaking && !self.swimming && (keys.forward != 0.0 || keys.right != 0.0) {
                    let mut up = self.feet;
                    up[1] += 1.0;
                    up[axis] += d;
                    let (lo, hi) = self.body(up, false);
                    if !overlaps(blocks, lo, hi) {
                        self.velocity[1] = JUMP;
                        self.on_ground = false;
                    }
                }
                self.velocity[axis] = 0.0;
            }
            self.feet[axis] += moved;
        }
        if self.on_ground && self.standing(blocks, self.feet) {
            self.safe = self.feet;
        }
        // Fallen out of the map: back where they last stood.
        if self.feet[1] < (blocks.bottom() - 64) as f64 {
            self.feet = self.safe;
            self.velocity = [0.0; 3];
        }
    }

    /// Whether the body at `feet` stands on something.
    fn standing(&self, blocks: &mut impl Blocks, feet: [f64; 3]) -> bool {
        let (lo, hi) = self.body(feet, self.sneaking);
        sweep(blocks, lo, hi, 1, -0.01) > -0.01
    }
}

/// The blocks a box covers along an axis, from `lo` to `hi`.
fn cells(lo: f64, hi: f64) -> std::ops::RangeInclusive<i32> {
    (lo + EPS).floor() as i32..=(hi - EPS).floor() as i32
}

/// Whether the box `lo`..`hi` is in a solid block.
fn overlaps(blocks: &mut impl Blocks, lo: [f64; 3], hi: [f64; 3]) -> bool {
    for x in cells(lo[0], hi[0]) {
        for z in cells(lo[2], hi[2]) {
            for y in cells(lo[1], hi[1]) {
                if blocks.solid(x, y, z) {
                    return true;
                }
            }
        }
    }
    false
}

/// Whether a layer of blocks across `axis`, at `c`, meets the box's
/// shadow on it.
fn layer(blocks: &mut impl Blocks, lo: [f64; 3], hi: [f64; 3], axis: usize, c: i32) -> bool {
    let (a, b) = match axis {
        0 => (1, 2),
        1 => (0, 2),
        _ => (0, 1),
    };
    for u in cells(lo[a], hi[a]) {
        for v in cells(lo[b], hi[b]) {
            let mut p = [0; 3];
            p[axis] = c;
            p[a] = u;
            p[b] = v;
            if blocks.solid(p[0], p[1], p[2]) {
                return true;
            }
        }
    }
    false
}

/// How far the box `lo`..`hi` goes when moved by `d` along `axis`, up to
/// the first solid block.
fn sweep(blocks: &mut impl Blocks, lo: [f64; 3], hi: [f64; 3], axis: usize, d: f64) -> f64 {
    if d > 0.0 {
        let first = (hi[axis] - EPS).floor() as i32 + 1;
        let last = (hi[axis] + d - EPS).floor() as i32;
        for c in first..=last {
            if layer(blocks, lo, hi, axis, c) {
                return (c as f64 - hi[axis]).clamp(0.0, d);
            }
        }
    } else if d < 0.0 {
        let first = (lo[axis] + EPS).floor() as i32 - 1;
        let last = (lo[axis] + d + EPS).floor() as i32;
        for c in (last..=first).rev() {
            if layer(blocks, lo, hi, axis, c) {
                return ((c + 1) as f64 - lo[axis]).clamp(d, 0.0);
            }
        }
    }
    d
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// A test world: ground up to y 64, some blocks, some water.
    #[derive(Default)]
    struct World {
        blocks: HashSet<(i32, i32, i32)>,
        water: HashSet<(i32, i32, i32)>,
        /// The ground ends here along x (a cliff into the void).
        edge: Option<i32>,
    }

    impl Blocks for World {
        fn solid(&mut self, x: i32, y: i32, z: i32) -> bool {
            ((0..64).contains(&y) && self.edge.is_none_or(|e| x < e)) || self.blocks.contains(&(x, y, z))
        }
        fn fluid(&mut self, x: i32, y: i32, z: i32) -> bool {
            self.water.contains(&(x, y, z))
        }
        fn bottom(&self) -> i32 {
            0
        }
    }

    fn run(p: &mut Player, w: &mut World, keys: Keys, secs: f64) {
        // The first frame says what was pressed; the others only what is held.
        let mut k = keys;
        let mut t = 0.0;
        while t < secs {
            p.step(w, k, 0.0, 1.0 / 60.0);
            k.jump_pressed = false;
            k.forward_pressed = false;
            t += 1.0 / 60.0;
        }
    }

    const NORTH: Keys = Keys {
        forward: 1.0,
        right: 0.0,
        forward_pressed: false,
        jump: false,
        jump_pressed: false,
        sneak: false,
        run: false,
    };

    #[test]
    fn falls_onto_the_ground_and_walks() {
        let mut w = World::default();
        let mut p = Player::new([0.5, 70.0, 0.5]);
        run(&mut p, &mut w, Keys::default(), 1.5);
        assert!(p.on_ground);
        assert!((p.feet[1] - 64.0).abs() < 1e-9, "stands on the ground: {:?}", p.feet);
        // A second north (−z) at walking speed.
        run(&mut p, &mut w, NORTH, 1.0);
        assert!((-4.4..-3.6).contains(&(p.feet[2] - 0.5)), "walked {:?}", p.feet);
        assert!((p.feet[1] - 64.0).abs() < 1e-9);
        assert!((p.eye()[1] - 65.62).abs() < 1e-6);
    }

    #[test]
    fn jumps_onto_a_step_but_not_over_a_wall() {
        let mut w = World::default();
        // A step of one block at z −3, a wall of two at z −8.
        for x in -3..=3 {
            for z in -10..=-3 {
                w.blocks.insert((x, 64, z));
            }
            w.blocks.insert((x, 65, -8));
            w.blocks.insert((x, 66, -8));
        }
        let mut p = Player::new([0.5, 64.0, 0.5]);
        p.auto_jump = false;
        // Walking, the step stops the player.
        run(&mut p, &mut w, NORTH, 1.5);
        assert!((p.feet[2] - (-1.7)).abs() < 1e-6, "against the step: {:?}", p.feet);
        // Jumping, over it.
        run(&mut p, &mut w, Keys { jump: true, jump_pressed: true, ..NORTH }, 0.8);
        run(&mut p, &mut w, NORTH, 1.5);
        assert!((p.feet[1] - 65.0).abs() < 1e-9, "on the step: {:?}", p.feet);
        // The wall is too high to jump over.
        run(&mut p, &mut w, Keys { jump: true, jump_pressed: true, ..NORTH }, 2.0);
        assert!(p.feet[2] > -7.0 + 0.29, "stopped by the wall: {:?}", p.feet);
        assert!(!p.flying, "held, Space jumps again, it does not fly");
    }

    #[test]
    fn walking_into_a_step_jumps_onto_it() {
        let mut w = World::default();
        // Steps of one block every three, then a wall of two.
        for x in -3..=3 {
            for (z, top) in [(-3, 65), (-4, 65), (-5, 65), (-6, 66), (-7, 66), (-8, 66), (-9, 68)] {
                for y in 64..top {
                    w.blocks.insert((x, y, z));
                }
            }
        }
        let mut p = Player::new([0.5, 64.0, 0.5]);
        run(&mut p, &mut w, NORTH, 4.0);
        assert!((p.feet[1] - 66.0).abs() < 1e-9 && p.on_ground, "up both steps: {:?}", p.feet);
        assert!(p.feet[2] > -8.0 + 0.29, "but not up the wall: {:?}", p.feet);
        // Crouched, it does not.
        let mut p = Player::new([0.5, 64.0, 0.5]);
        run(&mut p, &mut w, Keys { sneak: true, ..NORTH }, 4.0);
        assert!((p.feet[1] - 64.0).abs() < 1e-9, "{:?}", p.feet);
    }

    #[test]
    fn space_twice_flies_and_q_lands() {
        let mut w = World::default();
        let mut p = Player::new([0.5, 64.0, 0.5]);
        run(&mut p, &mut w, Keys::default(), 0.2);
        let tap = Keys { jump: true, jump_pressed: true, ..Keys::default() };
        run(&mut p, &mut w, tap, 0.1);
        run(&mut p, &mut w, Keys::default(), 0.1);
        run(&mut p, &mut w, tap, 0.05);
        assert!(p.flying);
        // Space held: up; nothing: it stays.
        run(&mut p, &mut w, Keys { jump: true, ..Keys::default() }, 1.0);
        let high = p.feet[1];
        assert!(high > 70.0, "{high}");
        run(&mut p, &mut w, Keys::default(), 1.0);
        assert!((p.feet[1] - high).abs() < 1.5 && p.flying, "hovers: {:?}", p.feet);
        // Q: down to the ground, where flying ends.
        run(&mut p, &mut w, Keys { sneak: true, ..Keys::default() }, 3.0);
        assert!(!p.flying && p.on_ground && (p.feet[1] - 64.0).abs() < 1e-9, "{p:?}");
        // Flying fast with Shift.
        let mut p = Player::new([0.5, 64.0, 0.5]);
        run(&mut p, &mut w, Keys::default(), 0.1);
        run(&mut p, &mut w, tap, 0.1);
        run(&mut p, &mut w, Keys::default(), 0.1);
        run(&mut p, &mut w, tap, 0.05);
        let z = p.feet[2];
        run(&mut p, &mut w, Keys { run: true, ..NORTH }, 1.0);
        assert!(z - p.feet[2] > 15.0, "{:?}", p.feet);
    }

    #[test]
    fn crouched_the_player_stays_on_the_edge() {
        let mut w = World { edge: Some(3), ..World::default() };
        let east = Keys { forward: 0.0, right: 1.0, ..Keys::default() };
        let mut p = Player::new([0.5, 64.0, 0.5]);
        run(&mut p, &mut w, Keys { sneak: true, ..east }, 4.0);
        assert!(p.on_ground && (p.feet[1] - 64.0).abs() < 1e-9, "{p:?}");
        assert!(p.feet[0] < 3.3 && p.feet[0] > 2.6, "at the edge: {:?}", p.feet);
        assert!(p.eye()[1] < 65.4, "crouched");
        // Standing, over it, and back up from the void.
        run(&mut p, &mut w, east, 0.5);
        assert!(p.feet[0] > 3.3);
        run(&mut p, &mut w, Keys::default(), 5.0);
        assert!(p.on_ground && p.feet[0] < 3.3 && (p.feet[1] - 64.0).abs() < 1e-9, "back on the land: {p:?}");
    }

    #[test]
    fn matter_built_over_the_player_lifts_them() {
        let mut w = World::default();
        let mut p = Player::new([0.5, 64.0, 0.5]);
        run(&mut p, &mut w, Keys::default(), 0.2);
        for y in 64..67 {
            w.blocks.insert((0, y, 0));
        }
        run(&mut p, &mut w, Keys::default(), 0.2);
        assert!((p.feet[1] - 67.0).abs() < 1e-9, "{:?}", p.feet);
    }

    #[test]
    fn swims_in_water() {
        let mut w = World::default();
        for x in -4..4 {
            for z in -4..4 {
                for y in 64..70 {
                    w.water.insert((x, y, z));
                }
            }
        }
        let mut p = Player::new([0.5, 69.0, 0.5]);
        run(&mut p, &mut w, Keys::default(), 0.5);
        assert!(p.swimming && p.velocity[1] > -2.5, "sinks slowly: {p:?}");
        let y = p.feet[1];
        run(&mut p, &mut w, Keys { jump: true, ..Keys::default() }, 0.6);
        assert!(p.feet[1] > y, "swims up");
    }
}
