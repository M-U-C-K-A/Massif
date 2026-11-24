//! The panel's edits beyond a node and a wire: copies (beside, or through
//! the clipboard as JSON, between documents too), composites made, opened
//! as a copy and unfolded, nodes left out (bypass) and named, frames
//! nested in frames, and places snapped, aligned and arranged.

use super::panel::{TerrainPanel, base_kind};
use massif_terrain::format::Op;
use massif_terrain::lower::component;
use massif_terrain::model::{Component, Frame, Graph, Node, NodeUi, Source, is_component_type};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// The grid nodes snap to, in graph units.
pub const GRID: f32 = 12.0;

/// What the clipboard holds of a graph: nodes with their places and the
/// wires among them, the frames gathering only them, the document's
/// composites they are made of.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Clip {
    pub format: String,
    pub nodes: BTreeMap<String, Node>,
    #[serde(default)]
    pub ui: BTreeMap<String, NodeUi>,
    #[serde(default)]
    pub frames: BTreeMap<String, Frame>,
    #[serde(default)]
    pub components: BTreeMap<String, Component>,
}

pub const CLIP_FORMAT: &str = "massif.terrain-nodes";

impl TerrainPanel {
    /// The selected nodes, as the clipboard keeps them.
    pub fn clip(&self, ids: &[String]) -> Option<Clip> {
        let set: BTreeSet<&String> = ids.iter().filter(|id| self.graph.nodes.contains_key(*id)).collect();
        if set.is_empty() {
            return None;
        }
        let mut c = Clip { format: CLIP_FORMAT.to_owned(), ..Clip::default() };
        for id in &set {
            let mut n = self.graph.nodes[*id].clone();
            // Wires from outside the copy are not copied.
            n.inputs.retain(|_, s| set.contains(&Source::parse(s).node.to_owned()));
            c.nodes.insert((*id).clone(), n);
            if let Some(u) = self.graph.ui.get(*id) {
                c.ui.insert((*id).clone(), u.clone());
            }
        }
        for (fid, f) in &self.graph.frames {
            if !f.members.is_empty() && f.members.iter().all(|m| set.contains(m)) {
                let mut f = f.clone();
                f.parent = f.parent.filter(|p| self.graph.frames.get(p).is_some_and(|pf| pf.members.iter().all(|m| set.contains(m))));
                c.frames.insert(fid.clone(), f);
            }
        }
        // The document's composites they use, and those these use.
        let mut todo: Vec<String> = c.nodes.values().filter(|n| n.kind.starts_with("project:")).map(|n| base_kind(&n.kind).to_owned()).collect();
        while let Some(k) = todo.pop() {
            if c.components.contains_key(&k) {
                continue;
            }
            if let Some(comp) = self.graph.components.get(&k) {
                todo.extend(comp.nodes.values().filter(|n| n.kind.starts_with("project:")).map(|n| base_kind(&n.kind).to_owned()));
                c.components.insert(k, comp.clone());
            }
        }
        Some(c)
    }

    /// The selected nodes as text for the clipboard.
    pub fn copy_text(&self) -> Option<String> {
        serde_json::to_string_pretty(&self.clip(&self.selected)?).ok()
    }

    /// Puts copies of a clipboard's nodes into the graph shown, their
    /// places moved by `offset` (or around `at`, in graph units); selects
    /// them. Whether the text held nodes.
    pub fn paste(&mut self, text: &str, at: Option<[f32; 2]>) -> bool {
        let Ok(clip) = serde_json::from_str::<Clip>(text) else { return false };
        if clip.format != CLIP_FORMAT || clip.nodes.is_empty() || self.read_only() {
            return false;
        }
        self.insert_clip(clip, at, [GRID * 3.0, GRID * 3.0])
    }

    fn insert_clip(&mut self, clip: Clip, at: Option<[f32; 2]>, offset: [f32; 2]) -> bool {
        // Composites the document lacks come with the nodes (from another
        // document); one it has under the same name is kept as it is.
        let missing: Vec<(String, Component)> = clip.components.into_iter().filter(|(k, _)| !self.graph.components.contains_key(k)).collect();
        if !missing.is_empty() {
            let g = self.edit();
            g.components.extend(missing);
        }
        let lo = clip.ui.values().fold([f32::MAX; 2], |a, u| [a[0].min(u.pos[0]), a[1].min(u.pos[1])]);
        let shift = match at {
            Some(q) if lo[0] < f32::MAX => [q[0] - lo[0], q[1] - lo[1]],
            _ => offset,
        };
        // Each node added under a fresh id, then the wires among them.
        let mut ops = Vec::new();
        let mut ids: Vec<&String> = clip.nodes.keys().collect();
        ids.sort();
        let mut names: HashMap<String, String> = HashMap::new();
        let mut taken: BTreeSet<String> = self.graph.nodes.keys().cloned().collect();
        for id in &ids {
            let n = &clip.nodes[*id];
            let new = fresh(id, &taken);
            taken.insert(new.clone());
            names.insert((*id).clone(), new.clone());
            ops.push(Op::Add { id: Some(new.clone()), kind: n.kind.clone(), params: n.params.clone(), inputs: BTreeMap::new() });
            if n.name.is_some() {
                ops.push(Op::Rename { node: new.clone(), to: n.name.clone() });
            }
            if n.bypass {
                ops.push(Op::Bypass { node: new.clone(), on: true });
            }
        }
        for id in &ids {
            for (input, src) in &clip.nodes[*id].inputs {
                let s = Source::parse(src);
                let Some(from) = names.get(s.node) else { continue };
                let from = match s.output {
                    Some(o) => format!("{from}.{o}"),
                    None => from.clone(),
                };
                ops.push(Op::Link { from, to: names[*id].clone(), input: input.clone() });
            }
        }
        // Frames that gather only copied nodes come too.
        let mut fnames: HashMap<String, String> = HashMap::new();
        let mut ftaken: BTreeSet<String> = self.graph.frames.keys().cloned().collect();
        let mut frames: Vec<(&String, &Frame)> = clip.frames.iter().collect();
        // Parents first.
        frames.sort_by_key(|(_, f)| f.parent.is_some());
        for (fid, f) in &frames {
            let new = fresh(fid, &ftaken);
            ftaken.insert(new.clone());
            fnames.insert((*fid).clone(), new.clone());
            ops.push(Op::Frame { id: Some(new), title: f.title.clone(), color: f.color.clone(), description: f.description.clone(), members: f.members.iter().filter_map(|m| names.get(m).cloned()).collect(), parent: f.parent.as_ref().and_then(|p| fnames.get(p).cloned()) });
        }
        let editing = std::mem::take(&mut self.selected);
        match self.ops(ops) {
            Ok(_) => {
                for (old, new) in &names {
                    let pos = clip.ui.get(old).map_or([0.0, 0.0], |u| u.pos);
                    let collapsed = clip.ui.get(old).is_some_and(|u| u.collapsed);
                    self.graph.ui.insert(new.clone(), NodeUi { pos: [pos[0] + shift[0], pos[1] + shift[1]], collapsed });
                }
                for (old, new) in &fnames {
                    if let (Some(f), Some(src)) = (self.graph.frames.get_mut(new), clip.frames.get(old)) {
                        f.rect = [src.rect[0] + shift[0], src.rect[1] + shift[1], src.rect[2], src.rect[3]];
                        f.collapsed = src.collapsed;
                    }
                }
                self.selected = ids.iter().map(|id| names[*id].clone()).collect();
                true
            }
            Err(e) => {
                self.selected = editing;
                self.notice = Some((e, 0.0));
                false
            }
        }
    }

    /// Copies of nodes, beside them, wired among themselves as they were.
    pub fn duplicate(&mut self, ids: &[String]) {
        if let Some(clip) = self.clip(ids) {
            self.insert_clip(clip, None, [GRID * 3.0, GRID * 3.0]);
        }
    }

    /// Gives a node a name to show (`None`, empty: its type's).
    pub fn set_name(&mut self, id: &str, name: Option<String>) {
        let to = name.map(|s| s.trim().to_owned()).filter(|s| !s.is_empty());
        if self.graph.nodes.get(id).is_some_and(|n| n.name != to) {
            let _ = self.op(Op::Rename { node: id.to_owned(), to });
        }
    }

    /// Leaves nodes out, or puts them back.
    pub fn set_bypass(&mut self, ids: &[String], on: bool) {
        let ops: Vec<Op> = ids.iter().filter(|id| self.graph.nodes.get(*id).is_some_and(|n| n.bypass != on && !n.kind.starts_with("output."))).map(|id| Op::Bypass { node: id.clone(), on }).collect();
        if !ops.is_empty() {
            let _ = self.ops(ops);
        }
    }

    /// Bypass on the selected nodes, or off if they all have it.
    pub fn toggle_bypass(&mut self) {
        let ids = self.selected.clone();
        let all = !ids.is_empty() && ids.iter().all(|id| self.graph.nodes.get(id).is_some_and(|n| n.bypass));
        self.set_bypass(&ids, !all);
    }

    /// Gathers the selected nodes into a new composite of the document, one
    /// node of it in their place, selected.
    pub fn group_selected(&mut self, label: &str) -> Result<String, String> {
        let ids: Vec<String> = self.selected.iter().filter(|id| self.graph.nodes.contains_key(*id)).cloned().collect();
        let at = ids.iter().map(|id| self.pos(id)).fold([f32::MAX; 2], |a, p| [a[0].min(p[0]), a[1].min(p[1])]);
        let inside: BTreeMap<String, NodeUi> = ids.iter().filter_map(|id| self.graph.ui.get(id).map(|u| (id.clone(), u.clone()))).collect();
        let id = self.op(Op::Group { nodes: ids.clone(), label: label.to_owned() })?;
        // The new node stands where they stood; inside, they keep their
        // places.
        self.graph.ui.insert(id.clone(), NodeUi { pos: at, collapsed: false });
        if let Some(kind) = self.graph.nodes.get(&id).map(|n| base_kind(&n.kind).to_owned())
            && let Some(c) = self.graph.components.get_mut(&kind)
        {
            for (k, u) in inside {
                if c.nodes.contains_key(&k) {
                    c.ui.insert(k, u);
                }
            }
        }
        self.selected = vec![id.clone()];
        Ok(id)
    }

    /// Puts a composite node's nodes back in its place, selected.
    pub fn ungroup(&mut self, id: &str) -> Result<(), String> {
        let at = self.pos(id);
        let comp = self.graph.nodes.get(id).and_then(|n| component(&self.graph, &n.kind)).map(|c| c.into_owned());
        let before: BTreeSet<String> = self.graph.nodes.keys().cloned().collect();
        self.op(Op::Ungroup { node: id.to_owned() })?;
        let added: Vec<String> = self.graph.nodes.keys().filter(|k| !before.contains(*k)).cloned().collect();
        // Their places inside, around where the composite stood.
        if let Some(c) = comp {
            let lo = c.ui.values().fold([f32::MAX; 2], |a, u| [a[0].min(u.pos[0]), a[1].min(u.pos[1])]);
            for new in &added {
                let inner = new.strip_prefix(&format!("{id}_")).unwrap_or(new);
                if let Some(u) = c.ui.get(inner) {
                    self.graph.ui.insert(new.clone(), NodeUi { pos: [at[0] + u.pos[0] - lo[0], at[1] + u.pos[1] - lo[1]], collapsed: u.collapsed });
                }
            }
        }
        self.auto_place();
        self.selected = added;
        Ok(())
    }

    /// Makes one of Massif's composites the document's own: a copy, under a
    /// name of its own, that node `id` then is; its settings kept. Gives
    /// the new type.
    pub fn edit_copy(&mut self, id: &str) -> Result<String, String> {
        let Some(node) = self.graph.nodes.get(id) else { return Err(format!("no node “{id}”")) };
        if !is_component_type(&node.kind) || self.graph.components.contains_key(base_kind(&node.kind)) {
            return Err(t!("Only Massif's composites are edited as a copy").to_owned());
        }
        let Some(comp) = component(&self.graph, &node.kind).map(|c| c.into_owned()) else { return Err(format!("there is no composite “{}”", node.kind)) };
        let label = t!("{name} (copy)", name = super::text::name_owned(&comp.label));
        let kind = self.op(Op::Fork { node: id.to_owned(), label })?;
        self.end_edit();
        Ok(kind)
    }

    /// Edits a copy of the composite open (one of Massif's): the copy
    /// replaces it in the scope around, and opens.
    pub fn edit_open_copy(&mut self) -> Result<String, String> {
        let depth = self.outer.len();
        let Some(o) = self.outer.last().cloned() else { return Err("no composite open".into()) };
        // Up to the first of Massif's composites in the way.
        let first = self.outer.iter().position(|o| !self.graph.components.contains_key(base_kind(&o.crumb.kind))).unwrap_or(depth - 1);
        let path: Vec<String> = self.outer[first..].iter().map(|o| o.crumb.node.clone()).collect();
        self.close_to(first);
        let kind = self.edit_copy(&path[0])?;
        for id in &path {
            if !self.open(id) {
                break;
            }
        }
        let _ = o;
        Ok(kind)
    }

    // ---- Frames ------------------------------------------------------

    /// The frames from the outermost in: each after the frame it sits in.
    pub fn frames_in_order(&self) -> Vec<String> {
        let depth = |id: &str| {
            let mut d = 0;
            let mut at = self.graph.frames.get(id).and_then(|f| f.parent.clone());
            while let Some(p) = at {
                d += 1;
                if d > 32 {
                    break;
                }
                at = self.graph.frames.get(&p).and_then(|f| f.parent.clone());
            }
            d
        };
        let mut ids: Vec<String> = self.graph.frames.keys().cloned().collect();
        ids.sort_by_key(|id| depth(id));
        ids
    }

    /// Frame `id` and every frame inside it, however deep.
    pub fn frame_tree(&self, id: &str) -> Vec<String> {
        let mut out = vec![id.to_owned()];
        let mut k = 0;
        while k < out.len() && out.len() < 256 {
            let parent = out[k].clone();
            out.extend(self.graph.frames.iter().filter(|(_, f)| f.parent.as_deref() == Some(&parent)).map(|(fid, _)| fid.clone()));
            k += 1;
        }
        out
    }

    /// Every node a frame holds, in it or in a frame inside it.
    pub fn frame_nodes(&self, id: &str) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for f in self.frame_tree(id) {
            if let Some(fr) = self.graph.frames.get(&f) {
                for m in &fr.members {
                    if !out.contains(m) {
                        out.push(m.clone());
                    }
                }
            }
        }
        out
    }

    /// Whether frame `id` or one it sits in is folded.
    pub fn frame_hidden(&self, id: &str) -> bool {
        let mut at = self.graph.frames.get(id).and_then(|f| f.parent.clone());
        let mut guard = 0;
        while let Some(p) = at {
            let Some(f) = self.graph.frames.get(&p) else { break };
            if f.collapsed {
                return true;
            }
            guard += 1;
            if guard > 32 {
                break;
            }
            at = f.parent.clone();
        }
        false
    }

    /// The folded frame hiding node `id`, the outermost one, if any.
    pub fn hidden_in(&self, id: &str) -> Option<String> {
        let mut found = None;
        for (fid, f) in &self.graph.frames {
            if f.collapsed && !self.frame_hidden(fid) && self.frame_nodes(fid).iter().any(|m| m == id) {
                found = Some(fid.clone());
            }
        }
        found
    }

    /// The innermost frame node `id` belongs to.
    pub fn frame_of(&self, id: &str) -> Option<String> {
        let mut best: Option<(String, usize)> = None;
        for fid in self.frames_in_order() {
            if self.graph.frames[&fid].members.iter().any(|m| m == id) {
                let d = self.frame_tree(&fid).len();
                if best.as_ref().is_none_or(|b| d <= b.1) {
                    best = Some((fid, d));
                }
            }
        }
        best.map(|b| b.0)
    }

    /// A frame's rectangle as drawn: its own, grown round what it holds.
    pub fn frame_rect(&self, id: &str) -> [f32; 4] {
        self.frame_rect_in(id, 0, &[])
    }

    /// As [`Self::frame_rect`], not grown round the nodes `skip` (those
    /// being dragged in or out).
    fn frame_rect_in(&self, id: &str, depth: usize, skip: &[String]) -> [f32; 4] {
        let Some(f) = self.graph.frames.get(id) else { return [0.0; 4] };
        let mut lo = [f.rect[0], f.rect[1]];
        let mut hi = [f.rect[0] + f.rect[2].max(120.0), f.rect[1] + f.rect[3].max(60.0)];
        let mut grow = |a: [f32; 2], b: [f32; 2]| {
            lo = [lo[0].min(a[0]), lo[1].min(a[1])];
            hi = [hi[0].max(b[0]), hi[1].max(b[1])];
        };
        for m in &f.members {
            if !self.graph.nodes.contains_key(m) || skip.contains(m) {
                continue;
            }
            let p = self.pos(m);
            let s = super::canvas::node_size(self, m);
            grow([p[0] - 16.0, p[1] - 40.0], [p[0] + s.x + 16.0, p[1] + s.y + 16.0]);
        }
        if depth < 16 {
            for (cid, c) in &self.graph.frames {
                if c.parent.as_deref() == Some(id) {
                    let r = self.frame_rect_in(cid, depth + 1, skip);
                    let h = if c.collapsed { 30.0 } else { r[3] };
                    grow([r[0] - 14.0, r[1] - 38.0], [r[0] + r[2] + 14.0, r[1] + h + 14.0]);
                }
            }
        }
        if f.collapsed {
            return [lo[0], lo[1], (hi[0] - lo[0]).clamp(180.0, 320.0), 30.0];
        }
        [lo[0], lo[1], hi[0] - lo[0], hi[1] - lo[1]]
    }

    /// A frame round the selected nodes, in the frame they all sit in; its
    /// title then edited.
    pub fn frame_selection(&mut self) -> Option<String> {
        let members: Vec<String> = self.selected.iter().filter(|id| self.graph.nodes.contains_key(*id)).cloned().collect();
        if members.is_empty() {
            return None;
        }
        let parent = {
            let first = self.frame_of(&members[0]);
            first.filter(|f| members.iter().all(|m| self.frame_nodes(f).contains(m)))
        };
        // Nodes in the new frame leave the frame around it: they sit in it
        // through the new one.
        let mut ops = Vec::new();
        if let Some(p) = &parent
            && let Some(pf) = self.graph.frames.get(p)
        {
            let rest: Vec<String> = pf.members.iter().filter(|m| !members.contains(m)).cloned().collect();
            ops.push(Op::Frame { id: Some(p.clone()), title: pf.title.clone(), color: pf.color.clone(), description: pf.description.clone(), members: rest, parent: pf.parent.clone() });
        }
        ops.push(Op::Frame { id: None, title: t!("Frame").to_owned(), color: None, description: String::new(), members: members.clone(), parent });
        let lo = members.iter().fold([f32::MAX; 2], |a, m| {
            let p = self.pos(m);
            [a[0].min(p[0]), a[1].min(p[1])]
        });
        let done = self.ops(ops).ok()?;
        let id = done.last()?.clone();
        if let Some(f) = self.graph.frames.get_mut(&id) {
            f.rect = [lo[0] - 16.0, lo[1] - 40.0, 120.0, 60.0];
        }
        self.renaming_frame = Some(id.clone());
        self.selected_frame = Some(id.clone());
        Some(id)
    }

    /// Changes a frame's title, colour or note.
    pub fn set_frame(&mut self, id: &str, title: Option<String>, color: Option<Option<String>>, description: Option<String>) {
        let Some(f) = self.graph.frames.get(id).cloned() else { return };
        let op = Op::Frame { id: Some(id.to_owned()), title: title.unwrap_or(f.title), color: color.unwrap_or(f.color), description: description.unwrap_or(f.description), members: f.members, parent: f.parent };
        let editing = self.is_editing();
        let _ = self.op(op);
        if editing {
            self.keep_editing();
        }
    }

    /// Removes a frame, leaving what it holds (in the frame around it).
    pub fn unframe(&mut self, id: &str) {
        let Some(f) = self.graph.frames.get(id).cloned() else { return };
        let mut ops = Vec::new();
        if let Some(p) = &f.parent
            && let Some(pf) = self.graph.frames.get(p)
        {
            let mut members = pf.members.clone();
            members.extend(f.members.iter().filter(|m| !members.contains(m)).cloned().collect::<Vec<_>>());
            ops.push(Op::Frame { id: Some(p.clone()), title: pf.title.clone(), color: pf.color.clone(), description: pf.description.clone(), members, parent: pf.parent.clone() });
        }
        ops.push(Op::Unframe { id: id.to_owned() });
        let _ = self.ops(ops);
        if self.selected_frame.as_deref() == Some(id) {
            self.selected_frame = None;
        }
    }

    /// Folds a frame (its nodes hidden, wires to them on its bar) or opens
    /// it.
    pub fn toggle_frame(&mut self, id: &str) {
        if let Some(f) = self.edit().frames.get_mut(id) {
            f.collapsed = !f.collapsed;
        }
        self.end_edit();
    }

    /// Moves a frame with everything it holds.
    pub fn move_frame(&mut self, id: &str, d: [f32; 2]) {
        let nodes = self.frame_nodes(id);
        let frames = self.frame_tree(id);
        let g = self.edit();
        for n in nodes {
            if let Some(u) = g.ui.get_mut(&n) {
                u.pos[0] += d[0];
                u.pos[1] += d[1];
            }
        }
        for f in frames {
            if let Some(fr) = g.frames.get_mut(&f) {
                fr.rect[0] += d[0];
                fr.rect[1] += d[1];
            }
        }
    }

    /// Nodes let go after a drag: each goes into the innermost frame its
    /// place falls in, or out of the frames it left.
    pub fn refile(&mut self, ids: &[String]) {
        let order = self.frames_in_order();
        let mut members: BTreeMap<String, Vec<String>> = self.graph.frames.iter().map(|(k, f)| (k.clone(), f.members.clone())).collect();
        let mut changed = false;
        for id in ids {
            if !self.graph.nodes.contains_key(id) {
                continue;
            }
            let p = self.pos(id);
            let s = super::canvas::node_size(self, id);
            let c = [p[0] + s.x / 2.0, p[1] + 14.0];
            // The innermost open frame whose rectangle holds its top.
            let inside = order.iter().rev().find(|fid| {
                let f = &self.graph.frames[*fid];
                if f.collapsed || self.frame_hidden(fid) {
                    return false;
                }
                let r = self.frame_rect_in(fid, 0, ids);
                c[0] >= r[0] && c[0] <= r[0] + r[2] && c[1] >= r[1] && c[1] <= r[1] + r[3]
            });
            let now = self.frame_of(id);
            if now.as_ref() == inside {
                continue;
            }
            // Still in the frame it was in (dragged within it, in a frame
            // inside that it does not belong to): kept.
            if let (Some(n), Some(i)) = (&now, inside)
                && self.frame_tree(i).contains(n)
                && !self.frame_tree(n).contains(i)
            {
                continue;
            }
            for m in members.values_mut() {
                m.retain(|x| x != id);
            }
            if let Some(i) = inside {
                members.get_mut(i).expect("a frame").push(id.clone());
            }
            changed = true;
        }
        if !changed {
            return;
        }
        let ops: Vec<Op> = members
            .into_iter()
            .filter(|(k, m)| self.graph.frames[k].members != *m)
            .map(|(k, m)| {
                let f = &self.graph.frames[&k];
                Op::Frame { id: Some(k.clone()), title: f.title.clone(), color: f.color.clone(), description: f.description.clone(), members: m, parent: f.parent.clone() }
            })
            .collect();
        let _ = self.ops(ops);
    }

    /// A frame let go after a drag: it sits in the innermost frame that
    /// holds its title bar, or in none.
    pub fn refile_frame(&mut self, id: &str) {
        let r = self.frame_rect(id);
        let c = [r[0] + 20.0, r[1] + 12.0];
        let tree = self.frame_tree(id);
        let order = self.frames_in_order();
        let inside = order.iter().rev().find(|fid| {
            if tree.contains(fid) || self.graph.frames[*fid].collapsed {
                return false;
            }
            let o = self.frame_rect(fid);
            c[0] >= o[0] && c[0] <= o[0] + o[2] && c[1] >= o[1] && c[1] <= o[1] + o[3]
        });
        let Some(f) = self.graph.frames.get(id).cloned() else { return };
        if f.parent.as_ref() == inside {
            return;
        }
        let _ = self.op(Op::Frame { id: Some(id.to_owned()), title: f.title, color: f.color, description: f.description, members: f.members, parent: inside.cloned() });
    }

    // ---- Places ------------------------------------------------------

    /// Puts the selected nodes on the grid.
    pub fn snap(&mut self, ids: &[String]) {
        if self.layout.no_snap {
            return;
        }
        let g = &mut self.graph;
        let mut moved = false;
        for id in ids {
            if let Some(u) = g.ui.get_mut(id) {
                let s = [(u.pos[0] / GRID).round() * GRID, (u.pos[1] / GRID).round() * GRID];
                moved |= s != u.pos;
                u.pos = s;
            }
        }
        if moved {
            self.dirty_layout();
        }
    }

    /// Lines the selected nodes up: their left edges (`x`), or their tops.
    pub fn align(&mut self, x: bool) {
        let ids: Vec<String> = self.selected.iter().filter(|id| self.graph.nodes.contains_key(*id)).cloned().collect();
        if ids.len() < 2 {
            return;
        }
        let k = if x { 0 } else { 1 };
        let to = ids.iter().map(|id| self.pos(id)[k]).fold(f32::MAX, f32::min);
        let g = self.edit();
        for id in &ids {
            if let Some(u) = g.ui.get_mut(id) {
                u.pos[k] = to;
            }
        }
        self.end_edit();
    }

    /// Spaces the selected nodes evenly, along x or y, as wide apart as
    /// they are.
    pub fn distribute(&mut self, x: bool) {
        let mut ids: Vec<String> = self.selected.iter().filter(|id| self.graph.nodes.contains_key(*id)).cloned().collect();
        if ids.len() < 3 {
            return;
        }
        let k = if x { 0 } else { 1 };
        ids.sort_by(|a, b| self.pos(a)[k].total_cmp(&self.pos(b)[k]));
        let sizes: Vec<f32> = ids.iter().map(|id| super::canvas::node_size(self, id)[k]).collect();
        let first = self.pos(&ids[0])[k];
        let last = self.pos(ids.last().expect("three")).k_end(k, *sizes.last().expect("three"));
        let total: f32 = sizes.iter().sum();
        let gap = ((last - first - total) / (ids.len() - 1) as f32).max(GRID);
        let mut at = first;
        let g = self.edit();
        for (id, s) in ids.iter().zip(&sizes) {
            if let Some(u) = g.ui.get_mut(id) {
                u.pos[k] = at;
            }
            at += s + gap;
        }
        self.end_edit();
    }

    /// Lays the selected nodes (or all) out in columns by depth, from their
    /// sources to the output, where the first of them stood.
    pub fn arrange(&mut self) {
        let ids: Vec<String> = if self.selected.len() > 1 { self.selected.clone() } else { self.graph.nodes.keys().cloned().collect() };
        let set: BTreeSet<&String> = ids.iter().collect();
        if ids.is_empty() {
            return;
        }
        let origin = ids.iter().map(|id| self.pos(id)).fold([f32::MAX; 2], |a, p| [a[0].min(p[0]), a[1].min(p[1])]);
        let mut sub = Graph::default();
        for id in &ids {
            let mut n = self.graph.nodes[id].clone();
            n.inputs.retain(|_, s| set.contains(&Source::parse(s).node.to_owned()));
            sub.nodes.insert(id.clone(), n);
        }
        let mut depth: BTreeMap<String, usize> = BTreeMap::new();
        fn depth_of(g: &Graph, id: &str, memo: &mut BTreeMap<String, usize>, guard: usize) -> usize {
            if let Some(d) = memo.get(id) {
                return *d;
            }
            let d = if guard > 64 { 0 } else { g.nodes.get(id).map_or(0, |n| n.inputs.values().map(|s| depth_of(g, Source::parse(s).node, memo, guard + 1) + 1).max().unwrap_or(0)) };
            memo.insert(id.to_owned(), d);
            d
        }
        for id in &ids {
            depth_of(&sub, id, &mut depth, 0);
        }
        // Columns by depth, each node in the order of what it feeds.
        let mut cols: BTreeMap<usize, Vec<String>> = BTreeMap::new();
        for id in &ids {
            cols.entry(depth[id]).or_default().push(id.clone());
        }
        for col in cols.values_mut() {
            col.sort_by(|a, b| self.pos(a)[1].total_cmp(&self.pos(b)[1]));
        }
        let mut place = Vec::new();
        let mut x = origin[0];
        for col in cols.values() {
            let w = col.iter().map(|id| super::canvas::node_size(self, id).x).fold(0.0, f32::max);
            let mut y = origin[1];
            for id in col {
                place.push((id.clone(), [x, y]));
                y += super::canvas::node_size(self, id).y + GRID * 3.0;
            }
            x += w + GRID * 6.0;
        }
        let g = self.edit();
        for (id, pos) in place {
            if let Some(u) = g.ui.get_mut(&id) {
                u.pos = [(pos[0] / GRID).round() * GRID, (pos[1] / GRID).round() * GRID];
            }
        }
        self.end_edit();
    }
}

trait End {
    fn k_end(self, k: usize, size: f32) -> f32;
}

impl End for [f32; 2] {
    fn k_end(self, k: usize, size: f32) -> f32 {
        self[k] + size
    }
}

/// An id made from `stem` that `taken` does not hold.
fn fresh(stem: &str, taken: &BTreeSet<String>) -> String {
    if !taken.contains(stem) {
        return stem.to_owned();
    }
    let base = stem.trim_end_matches(|c: char| c.is_ascii_digit()).trim_end_matches('_');
    let base = if base.is_empty() { "node" } else { base };
    let mut k = 2;
    loop {
        let id = format!("{base}_{k}");
        if !taken.contains(&id) {
            return id;
        }
        k += 1;
    }
}
