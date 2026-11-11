//! Window layout: workspaces made of areas.
//!
//! A workspace is a binary tree. Leaves are *areas*, each showing one editor;
//! inner nodes split their rectangle in two, side by side or stacked. Areas
//! can be split, closed, swapped and dropped onto each other, which is all it
//! takes to build any arrangement.

use crate::theme::Icon;
use serde::{Deserialize, Serialize};

/// What an area shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EditorKind {
    Empty,
    Map,
    View3d,
    Tools,
    ToolSettings,
    BrushSettings,
    Brushes,
    CustomBrushes,
    Terrain,
    /// Every real block of the game, to paint.
    Blocks,
    Biomes,
    Levels,
    Info,
    /// The terrain graph (procedural generation and erosion).
    Graph,
}

impl EditorKind {
    /// Editors offered in the area menu, by group.
    pub const GROUPS: [(&'static str, &'static [EditorKind]); 4] = [
        ("View", &[EditorKind::Map, EditorKind::View3d]),
        (
            "Tools",
            &[
                EditorKind::Tools,
                EditorKind::ToolSettings,
                EditorKind::BrushSettings,
                EditorKind::Brushes,
                EditorKind::CustomBrushes,
            ],
        ),
        (
            "Paint",
            &[EditorKind::Terrain, EditorKind::Blocks, EditorKind::Biomes, EditorKind::Levels],
        ),
        ("World", &[EditorKind::Info, EditorKind::Graph]),
    ];

    pub const fn label(self) -> &'static str {
        match self {
            EditorKind::Empty => "Empty area",
            EditorKind::Map => "Map",
            EditorKind::View3d => "3D View",
            EditorKind::Tools => "Tools",
            EditorKind::ToolSettings => "Tool Settings",
            EditorKind::BrushSettings => "Brush Settings",
            EditorKind::Brushes => "Brushes",
            EditorKind::CustomBrushes => "Custom Brushes",
            EditorKind::Terrain => "Terrain",
            EditorKind::Blocks => "Blocks",
            EditorKind::Biomes => "Biomes",
            EditorKind::Levels => "Levels",
            EditorKind::Info => "Info",
            EditorKind::Graph => "Terrain Graph",
        }
    }

    pub const fn icon(self) -> Icon {
        match self {
            EditorKind::Empty => Icon::LayoutGrid,
            EditorKind::Map => Icon::Map,
            EditorKind::View3d => Icon::Box,
            EditorKind::Tools => Icon::Wrench,
            EditorKind::ToolSettings => Icon::SlidersHorizontal,
            EditorKind::BrushSettings => Icon::Settings2,
            EditorKind::Brushes => Icon::Brush,
            EditorKind::CustomBrushes => Icon::Images,
            EditorKind::Terrain => Icon::Mountain,
            EditorKind::Blocks => Icon::Blocks,
            EditorKind::Biomes => Icon::Trees,
            EditorKind::Levels => Icon::Layers,
            EditorKind::Info => Icon::Info,
            EditorKind::Graph => Icon::Workflow,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Dir {
    /// Side by side: `a` on the left, `b` on the right.
    Row,
    /// Stacked: `a` on top, `b` below.
    Column,
}

/// Where an area is dropped on another one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Zone {
    Left,
    Right,
    Top,
    Bottom,
    /// Swap the two areas.
    Center,
}

pub type AreaId = u64;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Node {
    Area {
        id: AreaId,
        editor: EditorKind,
    },
    Split {
        dir: Dir,
        /// Share of the rectangle given to `a` (0..1).
        ratio: f32,
        a: Box<Node>,
        b: Box<Node>,
    },
}

impl Node {
    #[cfg(test)]
    pub fn areas(&self, out: &mut Vec<(AreaId, EditorKind)>) {
        match self {
            Node::Area { id, editor } => out.push((*id, *editor)),
            Node::Split { a, b, .. } => {
                a.areas(out);
                b.areas(out);
            }
        }
    }

    pub fn count(&self) -> usize {
        match self {
            Node::Area { .. } => 1,
            Node::Split { a, b, .. } => a.count() + b.count(),
        }
    }

    /// Id of the first (top-left) area under this node.
    #[cfg(test)]
    pub fn first_area(&self) -> AreaId {
        match self {
            Node::Area { id, .. } => *id,
            Node::Split { a, .. } => a.first_area(),
        }
    }

    pub fn editor_of(&self, target: AreaId) -> Option<EditorKind> {
        match self {
            Node::Area { id, editor } => (*id == target).then_some(*editor),
            Node::Split { a, b, .. } => a.editor_of(target).or_else(|| b.editor_of(target)),
        }
    }

    pub fn set_editor(&mut self, target: AreaId, new: EditorKind) -> bool {
        match self {
            Node::Area { id, editor } if *id == target => {
                *editor = new;
                true
            }
            Node::Area { .. } => false,
            Node::Split { a, b, .. } => a.set_editor(target, new) || b.set_editor(target, new),
        }
    }

    /// Replace area `target` by a split holding it and `new`.
    pub fn split(&mut self, target: AreaId, dir: Dir, new_first: bool, new: Node) -> bool {
        match self {
            Node::Area { id, .. } if *id == target => {
                let old = std::mem::replace(
                    self,
                    Node::Area {
                        id: 0,
                        editor: EditorKind::Empty,
                    },
                );
                let (a, b) = if new_first { (new, old) } else { (old, new) };
                *self = Node::Split {
                    dir,
                    ratio: 0.5,
                    a: Box::new(a),
                    b: Box::new(b),
                };
                true
            }
            Node::Area { .. } => false,
            Node::Split { a, b, .. } => {
                // `new` can only be consumed once: find the side first.
                if a.editor_of(target).is_some() {
                    a.split(target, dir, new_first, new)
                } else {
                    b.split(target, dir, new_first, new)
                }
            }
        }
    }

    /// Remove area `target`; its sibling takes the freed space. The root
    /// area cannot be removed.
    pub fn remove(&mut self, target: AreaId) -> Option<Node> {
        let Node::Split { a, b, .. } = self else {
            return None;
        };
        let is = |n: &Node| matches!(n, Node::Area { id, .. } if *id == target);
        let (a_is, b_is) = (is(a), is(b));
        if a_is || b_is {
            let placeholder = Node::Area {
                id: 0,
                editor: EditorKind::Empty,
            };
            let Node::Split { a, b, .. } = std::mem::replace(self, placeholder) else {
                unreachable!()
            };
            let (removed, kept) = if a_is { (*a, *b) } else { (*b, *a) };
            *self = kept;
            return Some(removed);
        }
        a.remove(target).or_else(|| b.remove(target))
    }

    fn area_mut(&mut self, target: AreaId) -> Option<(&mut AreaId, &mut EditorKind)> {
        match self {
            Node::Area { id, editor } if *id == target => Some((id, editor)),
            Node::Area { .. } => None,
            Node::Split { a, b, .. } => a.area_mut(target).or_else(|| b.area_mut(target)),
        }
    }

    /// Exchange two areas (their identity travels with them, so each keeps
    /// its own view state).
    pub fn swap(&mut self, x: AreaId, y: AreaId) -> bool {
        let (Some(ex), Some(ey)) = (self.editor_of(x), self.editor_of(y)) else {
            return false;
        };
        if x == y {
            return false;
        }
        // Two passes through a sentinel id keep the borrows simple.
        const SENTINEL: AreaId = u64::MAX;
        if let Some((id, editor)) = self.area_mut(x) {
            (*id, *editor) = (SENTINEL, ey);
        }
        if let Some((id, editor)) = self.area_mut(y) {
            (*id, *editor) = (x, ex);
        }
        if let Some((id, _)) = self.area_mut(SENTINEL) {
            *id = y;
        }
        true
    }

    /// Move area `src` next to (or onto) area `dst`.
    pub fn drop_on(&mut self, src: AreaId, dst: AreaId, zone: Zone) -> bool {
        if src == dst || self.editor_of(src).is_none() || self.editor_of(dst).is_none() {
            return false;
        }
        if zone == Zone::Center {
            return self.swap(src, dst);
        }
        let Some(moved) = self.remove(src) else {
            return false;
        };
        let (dir, first) = match zone {
            Zone::Left => (Dir::Row, true),
            Zone::Right => (Dir::Row, false),
            Zone::Top => (Dir::Column, true),
            Zone::Bottom => (Dir::Column, false),
            Zone::Center => unreachable!(),
        };
        self.split(dst, dir, first, moved)
    }

    /// The area sharing `target`'s split, when it is a single area:
    /// `(its id, its editor, the split direction, whether target comes first)`.
    pub fn sibling(&self, target: AreaId) -> Option<(AreaId, EditorKind, Dir, bool)> {
        let Node::Split { dir, a, b, .. } = self else {
            return None;
        };
        let is = |n: &Node| matches!(n, Node::Area { id, .. } if *id == target);
        let area = |n: &Node| match n {
            Node::Area { id, editor } => Some((*id, *editor)),
            _ => None,
        };
        if is(a) {
            return area(b).map(|(id, e)| (id, e, *dir, true));
        }
        if is(b) {
            return area(a).map(|(id, e)| (id, e, *dir, false));
        }
        a.sibling(target).or_else(|| b.sibling(target))
    }

    /// Set the ratio of the split that holds `target` directly.
    pub fn set_parent_ratio(&mut self, target: AreaId, value: f32) -> bool {
        let Node::Split { ratio, a, b, .. } = self else {
            return false;
        };
        let is = |n: &Node| matches!(n, Node::Area { id, .. } if *id == target);
        if is(a) || is(b) {
            *ratio = value;
            return true;
        }
        a.set_parent_ratio(target, value) || b.set_parent_ratio(target, value)
    }

    /// The newest area of the tree (tests).
    #[cfg(test)]
    pub fn max_id_pub(&self) -> Option<AreaId> {
        Some(self.max_id())
    }

    fn max_id(&self) -> AreaId {
        match self {
            Node::Area { id, .. } => *id,
            Node::Split { a, b, .. } => a.max_id().max(b.max_id()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Workspace {
    pub name: String,
    pub root: Node,
    /// Area temporarily shown alone.
    #[serde(default)]
    pub maximized: Option<AreaId>,
}

/// All workspaces of the window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Layout {
    pub workspaces: Vec<Workspace>,
    pub active: usize,
    next_id: AreaId,
}

/// Builds trees with fresh area ids.
struct Builder<'a>(&'a mut AreaId);

impl Builder<'_> {
    fn area(&mut self, editor: EditorKind) -> Node {
        *self.0 += 1;
        Node::Area {
            id: *self.0,
            editor,
        }
    }

    fn row(&mut self, ratio: f32, a: Node, b: Node) -> Node {
        Node::Split {
            dir: Dir::Row,
            ratio,
            a: Box::new(a),
            b: Box::new(b),
        }
    }

    fn column(&mut self, ratio: f32, a: Node, b: Node) -> Node {
        Node::Split {
            dir: Dir::Column,
            ratio,
            a: Box::new(a),
            b: Box::new(b),
        }
    }
}

impl Default for Layout {
    fn default() -> Self {
        use EditorKind::*;
        let mut next_id = 0;
        let mut b = Builder(&mut next_id);
        let mut workspaces = Vec::new();
        let mut add = |name: &str, root: Node| {
            workspaces.push(Workspace {
                name: name.into(),
                root,
                maximized: None,
            });
        };

        // Layout: tools and their settings on the left, the map in the
        // middle, 3D, brushes, terrain and levels on the right.
        let left = {
            let (tools, settings, brush) = (b.area(Tools), b.area(ToolSettings), b.area(BrushSettings));
            let lower = b.column(0.5, settings, brush);
            b.column(0.26, tools, lower)
        };
        let right = {
            let view = b.area(View3d);
            let (brushes, terrain) = (b.area(Brushes), b.area(Terrain));
            let palettes = b.row(0.5, brushes, terrain);
            let (levels, info) = (b.area(Levels), b.area(Info));
            let world = b.row(0.5, levels, info);
            let lower = b.column(0.58, palettes, world);
            b.column(0.4, view, lower)
        };
        let map = b.area(Map);
        let centre = b.row(0.64, map, right);
        add("Layout", b.row(0.17, left, centre));

        // Sculpting: a big map, the brushes and the 3D view beside it.
        let left = {
            let (tools, settings, brush) = (b.area(Tools), b.area(ToolSettings), b.area(BrushSettings));
            let lower = b.column(0.45, settings, brush);
            b.column(0.3, tools, lower)
        };
        let right = {
            let view = b.area(View3d);
            let (brushes, custom) = (b.area(Brushes), b.area(CustomBrushes));
            let lower = b.column(0.6, brushes, custom);
            b.column(0.45, view, lower)
        };
        let map = b.area(Map);
        let views = b.row(0.7, map, right);
        add("Sculpting", b.row(0.16, left, views));

        // Painting: palettes on both sides of the map.
        let left = {
            let (tools, settings, brush) = (b.area(Tools), b.area(ToolSettings), b.area(BrushSettings));
            let lower = b.column(0.45, settings, brush);
            b.column(0.3, tools, lower)
        };
        let right = {
            let (terrain, blocks, biomes) = (b.area(Terrain), b.area(Blocks), b.area(Biomes));
            let lower = b.column(0.62, blocks, biomes);
            b.column(0.4, terrain, lower)
        };
        let map = b.area(Map);
        let centre = b.row(0.72, map, right);
        add("Painting", b.row(0.16, left, centre));

        // 3D: the preview takes the stage.
        let side = {
            let (map, levels) = (b.area(Map), b.area(Levels));
            b.column(0.6, map, levels)
        };
        let view = b.area(View3d);
        add("3D", b.row(0.72, view, side));

        // Maps: two map views side by side, to compare display modes.
        let (m1, m2) = (b.area(Map), b.area(Map));
        add("Maps", b.row(0.5, m1, m2));

        // Generate: the terrain graph, with the map and the 3D view to see
        // what it makes.
        let graph = b.area(Graph);
        let (map, view) = (b.area(Map), b.area(View3d));
        let side = b.column(0.5, map, view);
        add("Generate", b.row(0.62, graph, side));

        // Minimal: just tools and the map.
        let (tools, map) = (b.area(Tools), b.area(Map));
        add("Minimal", b.row(0.12, tools, map));

        Layout {
            workspaces,
            active: 0,
            next_id,
        }
    }
}

impl Layout {
    /// Make a deserialized layout safe to use.
    pub fn sanitize(mut self) -> Self {
        if self.workspaces.is_empty() {
            return Self::default();
        }
        self.active = self.active.min(self.workspaces.len() - 1);
        let max = self.workspaces.iter().map(|w| w.root.max_id()).max();
        self.next_id = self.next_id.max(max.unwrap_or(0));
        self
    }

    pub fn current(&self) -> &Workspace {
        &self.workspaces[self.active]
    }

    pub fn current_mut(&mut self) -> &mut Workspace {
        &mut self.workspaces[self.active]
    }

    fn fresh_id(&mut self) -> AreaId {
        self.next_id += 1;
        self.next_id
    }

    /// Split an area of the current workspace; the new half starts empty.
    pub fn split(&mut self, target: AreaId, dir: Dir) -> AreaId {
        let id = self.fresh_id();
        let new = Node::Area {
            id,
            editor: EditorKind::Empty,
        };
        self.current_mut().root.split(target, dir, false, new);
        id
    }

    /// Split an area at `ratio` of its width (`Dir::Row`) or height; the new,
    /// empty half comes first when `new_first`.
    pub fn split_at(&mut self, target: AreaId, dir: Dir, new_first: bool, ratio: f32) -> AreaId {
        let id = self.fresh_id();
        let new = Node::Area {
            id,
            editor: EditorKind::Empty,
        };
        let root = &mut self.current_mut().root;
        if root.split(target, dir, new_first, new) {
            root.set_parent_ratio(id, ratio.clamp(0.05, 0.95));
        }
        id
    }

    /// Merge `target` with the area sharing its split: the sibling goes, the
    /// target takes its place.
    pub fn join(&mut self, target: AreaId) -> bool {
        match self.current().root.sibling(target) {
            Some((other, ..)) => {
                self.close(other);
                true
            }
            None => false,
        }
    }

    pub fn close(&mut self, target: AreaId) {
        let ws = self.current_mut();
        if ws.root.remove(target).is_some() && ws.maximized == Some(target) {
            ws.maximized = None;
        }
    }

    pub fn toggle_maximize(&mut self, target: AreaId) {
        let ws = self.current_mut();
        ws.maximized = match ws.maximized {
            Some(_) => None,
            None => Some(target),
        };
    }

    /// Add a copy of the current workspace and switch to it.
    pub fn duplicate_current(&mut self) {
        let mut copy = self.current().clone();
        // Give the copy its own area ids so view state is not shared.
        fn renumber(node: &mut Node, next: &mut AreaId) {
            match node {
                Node::Area { id, .. } => {
                    *next += 1;
                    *id = *next;
                }
                Node::Split { a, b, .. } => {
                    renumber(a, next);
                    renumber(b, next);
                }
            }
        }
        renumber(&mut copy.root, &mut self.next_id);
        copy.maximized = None;
        let base = copy.name.trim_end_matches(|c: char| c.is_ascii_digit() || c == ' ');
        let n = self.workspaces.len() + 1;
        copy.name = format!("{base} {n}");
        self.workspaces.push(copy);
        self.active = self.workspaces.len() - 1;
    }

    pub fn remove_workspace(&mut self, index: usize) {
        if self.workspaces.len() > 1 && index < self.workspaces.len() {
            self.workspaces.remove(index);
            if self.active >= index && self.active > 0 {
                self.active -= 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(node: &Node) -> Vec<AreaId> {
        let mut v = Vec::new();
        node.areas(&mut v);
        v.into_iter().map(|(id, _)| id).collect()
    }

    fn area(id: AreaId, editor: EditorKind) -> Node {
        Node::Area { id, editor }
    }

    fn tree() -> Node {
        // (1 | (2 / 3))
        Node::Split {
            dir: Dir::Row,
            ratio: 0.3,
            a: Box::new(area(1, EditorKind::Tools)),
            b: Box::new(Node::Split {
                dir: Dir::Column,
                ratio: 0.5,
                a: Box::new(area(2, EditorKind::Map)),
                b: Box::new(area(3, EditorKind::View3d)),
            }),
        }
    }

    #[test]
    fn default_layout_is_well_formed() {
        let layout = Layout::default();
        assert_eq!(layout.workspaces.len(), 7);
        let mut seen = std::collections::HashSet::new();
        for ws in &layout.workspaces {
            for id in ids(&ws.root) {
                assert!(id > 0 && seen.insert(id), "duplicate area id {id}");
            }
        }
        assert_eq!(seen.len() as u64, layout.next_id);
        let first = &layout.workspaces[0].root;
        assert_eq!(first.count(), 9);
        // Layouts survive serialisation (they are persisted between runs).
        let json = serde_json::to_string(&layout).unwrap();
        let back: Layout = serde_json::from_str(&json).unwrap();
        assert_eq!(back.sanitize(), layout);
    }

    #[test]
    fn corner_splits_and_joins() {
        // Minimal: tools | map.
        let mut layout = Layout {
            active: 6,
            ..Layout::default()
        };
        let map = layout.current().root.first_area() + 1;
        assert_eq!(layout.current().root.editor_of(map), Some(EditorKind::Map));
        let new = layout.split_at(map, Dir::Row, false, 0.7);
        let (sib, editor, dir, first) = layout.current().root.sibling(new).unwrap();
        assert_eq!((sib, editor, dir, first), (map, EditorKind::Map, Dir::Row, false));
        assert!(layout.join(map));
        assert_eq!(layout.current().root.editor_of(new), None);
        assert_eq!(layout.current().root.count(), 2);
    }

    #[test]
    fn split_and_close_are_inverse() {
        let mut t = tree();
        assert!(t.split(2, Dir::Row, false, area(9, EditorKind::Empty)));
        assert_eq!(ids(&t), vec![1, 2, 9, 3]);
        assert!(t.split(1, Dir::Column, true, area(8, EditorKind::Info)));
        assert_eq!(ids(&t), vec![8, 1, 2, 9, 3]);
        assert!(!t.split(77, Dir::Row, false, area(10, EditorKind::Empty)));

        assert_eq!(t.remove(9), Some(area(9, EditorKind::Empty)));
        assert_eq!(t.remove(8), Some(area(8, EditorKind::Info)));
        assert_eq!(t, tree());
        assert_eq!(t.remove(77), None);
    }

    #[test]
    fn the_last_area_cannot_be_closed() {
        let mut t = tree();
        assert!(t.remove(1).is_some());
        assert!(t.remove(2).is_some());
        assert_eq!(t, area(3, EditorKind::View3d));
        assert_eq!(t.remove(3), None);
    }

    #[test]
    fn swap_exchanges_positions() {
        let mut t = tree();
        assert!(t.swap(1, 3));
        assert_eq!(ids(&t), vec![3, 2, 1]);
        assert_eq!(t.editor_of(3), Some(EditorKind::View3d));
        assert_eq!(t.editor_of(1), Some(EditorKind::Tools));
        assert!(!t.swap(1, 1));
        assert!(!t.swap(1, 42));
    }

    #[test]
    fn drop_moves_an_area_next_to_another() {
        let mut t = tree();
        assert!(t.drop_on(1, 3, Zone::Bottom));
        // 1 left the row, so (2 / (3 / 1)) remains.
        assert_eq!(ids(&t), vec![2, 3, 1]);
        assert!(t.drop_on(1, 2, Zone::Left));
        assert_eq!(ids(&t), vec![1, 2, 3]);
        assert!(t.drop_on(3, 1, Zone::Center));
        assert_eq!(ids(&t), vec![3, 2, 1]);
        assert!(!t.drop_on(2, 2, Zone::Left));
        assert_eq!(t.count(), 3);
    }

    #[test]
    fn layout_operations() {
        let mut layout = Layout::default();
        let before = layout.current().root.count();
        let target = layout.current().root.first_area();
        let new = layout.split(target, Dir::Row);
        assert_eq!(layout.current().root.count(), before + 1);
        assert_eq!(layout.current().root.editor_of(new), Some(EditorKind::Empty));

        layout.toggle_maximize(new);
        assert_eq!(layout.current().maximized, Some(new));
        layout.close(new);
        assert_eq!(layout.current().maximized, None);
        assert_eq!(layout.current().root.count(), before);

        let n = layout.workspaces.len();
        layout.duplicate_current();
        assert_eq!((layout.workspaces.len(), layout.active), (n + 1, n));
        assert_eq!(layout.current().name, "Layout 8");
        let (a, b) = (ids(&layout.workspaces[0].root), ids(&layout.current().root));
        assert!(a.iter().all(|id| !b.contains(id)));
        layout.remove_workspace(n);
        assert_eq!((layout.workspaces.len(), layout.active), (n, n - 1));
    }
}
