//! Directions, quarter turns and boxes, as the game's structures use them.

/// A side of a block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Dir {
    Down,
    Up,
    North,
    South,
    West,
    East,
}

impl Dir {
    pub fn parse(name: &str) -> Option<Dir> {
        Some(match name {
            "down" => Dir::Down,
            "up" => Dir::Up,
            "north" => Dir::North,
            "south" => Dir::South,
            "west" => Dir::West,
            "east" => Dir::East,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Dir::Down => "down",
            Dir::Up => "up",
            Dir::North => "north",
            Dir::South => "south",
            Dir::West => "west",
            Dir::East => "east",
        }
    }

    pub fn step(self) -> [i32; 3] {
        match self {
            Dir::Down => [0, -1, 0],
            Dir::Up => [0, 1, 0],
            Dir::North => [0, 0, -1],
            Dir::South => [0, 0, 1],
            Dir::West => [-1, 0, 0],
            Dir::East => [1, 0, 0],
        }
    }

    pub fn opposite(self) -> Dir {
        match self {
            Dir::Down => Dir::Up,
            Dir::Up => Dir::Down,
            Dir::North => Dir::South,
            Dir::South => Dir::North,
            Dir::West => Dir::East,
            Dir::East => Dir::West,
        }
    }

    pub fn is_horizontal(self) -> bool {
        !matches!(self, Dir::Up | Dir::Down)
    }

    /// Turned `turn` quarter turns clockwise seen from above: north goes
    /// east.
    pub fn turned(self, turn: u8) -> Dir {
        let mut d = self;
        for _ in 0..turn & 3 {
            d = match d {
                Dir::North => Dir::East,
                Dir::East => Dir::South,
                Dir::South => Dir::West,
                Dir::West => Dir::North,
                v => v,
            };
        }
        d
    }

    pub const HORIZONTAL: [Dir; 4] = [Dir::North, Dir::East, Dir::South, Dir::West];
}

/// `(x, z)` turned `turn` quarter turns clockwise about the origin, as the
/// game turns a template's blocks about its corner (east goes south).
#[inline]
pub fn turn_xz(x: i32, z: i32, turn: u8) -> (i32, i32) {
    massif_core::objects::turn_xz(x, z, turn)
}

/// A point of a template turned about its corner, then moved to `origin`.
#[inline]
pub fn place(local: [i32; 3], origin: [i32; 3], turn: u8) -> [i32; 3] {
    let (x, z) = turn_xz(local[0], local[2], turn);
    [origin[0] + x, origin[1] + local[1], origin[2] + z]
}

/// An entity's place in a template (in blocks, from the template's corner)
/// turned as the game turns it: about the corner of the block, so that a
/// point in the middle of a block stays in the middle of the block it goes
/// to.
pub fn place_exact(local: [f64; 3], origin: [i32; 3], turn: u8) -> [f64; 3] {
    let (x, z) = (local[0], local[2]);
    let (tx, tz) = match turn & 3 {
        0 => (x, z),
        1 => (1.0 - z, x),
        2 => (1.0 - x, 1.0 - z),
        _ => (z, 1.0 - x),
    };
    [origin[0] as f64 + tx, origin[1] as f64 + local[1], origin[2] as f64 + tz]
}

/// A box of blocks, both corners inclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BBox {
    pub min: [i32; 3],
    pub max: [i32; 3],
}

impl BBox {
    pub fn new(a: [i32; 3], b: [i32; 3]) -> Self {
        Self { min: [a[0].min(b[0]), a[1].min(b[1]), a[2].min(b[2])], max: [a[0].max(b[0]), a[1].max(b[1]), a[2].max(b[2])] }
    }

    /// The box of a template of `size` turned and moved to `origin`.
    pub fn of_template(size: [i32; 3], origin: [i32; 3], turn: u8) -> Self {
        let far = [size[0] - 1, size[1] - 1, size[2] - 1].map(|v| v.max(0));
        Self::new(place([0, 0, 0], origin, turn), place(far, origin, turn))
    }

    pub fn contains(&self, p: [i32; 3]) -> bool {
        (0..3).all(|a| p[a] >= self.min[a] && p[a] <= self.max[a])
    }

    /// Whether the two overlap by more than a touch (the game shrinks a
    /// box by a quarter block to ask).
    pub fn overlaps(&self, o: &BBox) -> bool {
        (0..3).all(|a| self.min[a] <= o.max[a] && o.min[a] <= self.max[a])
    }

    /// Whether `o` lies wholly inside.
    pub fn holds(&self, o: &BBox) -> bool {
        (0..3).all(|a| o.min[a] >= self.min[a] && o.max[a] <= self.max[a])
    }

    pub fn moved(&self, d: [i32; 3]) -> Self {
        Self { min: [self.min[0] + d[0], self.min[1] + d[1], self.min[2] + d[2]], max: [self.max[0] + d[0], self.max[1] + d[1], self.max[2] + d[2]] }
    }

    pub fn union(&self, o: &BBox) -> Self {
        Self { min: [0, 1, 2].map(|a| self.min[a].min(o.min[a])), max: [0, 1, 2].map(|a| self.max[a].max(o.max[a])) }
    }

    pub fn span(&self, axis: usize) -> i32 {
        self.max[axis] - self.min[axis] + 1
    }

    pub fn center(&self) -> [i32; 3] {
        [0, 1, 2].map(|a| (self.min[a] + self.max[a]) / 2)
    }
}

/// The room left for pieces: a box minus the boxes taken out of it, as the
/// game's shape of free space.
#[derive(Debug, Clone)]
pub struct Free {
    pub bounds: BBox,
    pub taken: Vec<BBox>,
}

impl Free {
    pub fn new(bounds: BBox) -> Self {
        Self { bounds, taken: Vec::new() }
    }

    /// Whether `b` fits: inside the bounds, and over none of the boxes
    /// taken out.
    pub fn fits(&self, b: &BBox) -> bool {
        self.bounds.holds(b) && !self.taken.iter().any(|t| t.overlaps(b))
    }

    pub fn take(&mut self, b: BBox) {
        self.taken.push(b);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates_turn_about_their_corner() {
        // A template 3 wide (x) and 5 long (z), turned a quarter: it runs
        // 5 along x, west of its origin, and 3 along z.
        let b = BBox::of_template([3, 2, 5], [10, 64, 20], 1);
        assert_eq!(b, BBox { min: [6, 64, 20], max: [10, 65, 22] });
        assert_eq!(place([2, 0, 4], [10, 64, 20], 1), [6, 64, 22]);
        for turn in 0..4 {
            let b = BBox::of_template([3, 2, 5], [0, 0, 0], turn);
            for p in [[0, 0, 0], [2, 1, 4], [1, 0, 3]] {
                assert!(b.contains(place(p, [0, 0, 0], turn)), "turn {turn}");
            }
        }
        assert_eq!(Dir::North.turned(1), Dir::East);
        assert_eq!(Dir::West.turned(3), Dir::South);
        assert_eq!(Dir::Up.turned(1), Dir::Up);
        // The middle of block (2, 4) goes to the middle of block (-4, 2).
        let p = place_exact([2.5, 0.0, 4.5], [0, 0, 0], 1);
        assert_eq!((p[0], p[2]), (-3.5, 2.5));
        assert_eq!(place([2, 0, 4], [0, 0, 0], 1), [-4, 0, 2]);
    }

    #[test]
    fn free_space_takes_boxes_out() {
        let mut f = Free::new(BBox::new([0, 0, 0], [20, 10, 20]));
        let a = BBox::new([0, 0, 0], [4, 4, 4]);
        assert!(f.fits(&a));
        f.take(a);
        assert!(!f.fits(&BBox::new([4, 0, 0], [8, 4, 4])));
        assert!(f.fits(&BBox::new([5, 0, 0], [8, 4, 4])), "touching is fine");
        assert!(!f.fits(&BBox::new([18, 0, 0], [21, 4, 4])), "past the bounds");
    }
}
