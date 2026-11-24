# The terrain generator's panel (1.3)

[← Back to the README](../README.md) · [Terrain graphs](terrain-graphs.md) · [Every node](terrain-nodes.md)

The **Terrain Generator** area holds the generator of 1.3 in one place:
the **Library** on the left, the **Viewer** over the **Graph** in the
middle, the selected node's **settings** on the right, and a status line
at the foot (what is computing, how long the last picture took, the
graphics card, the cache). Choose it from any area's header, in the
Terrain section. The graph is saved with the world.

- [The graph](#the-graph)
- [Composites](#composites)
- [Frames](#frames)
- [Sockets and wires](#sockets-and-wires)
- [The Map Output](#the-map-output)
- [Settings](#settings)
- [The viewer](#the-viewer)
- [Apply](#apply)
- [Keys](#keys)

## The graph

- **Add a node**: right-click the graph, press **Tab** (or Shift A), or use
  the **+** at the graph's top right, and type a few letters of its name:
  the start of a word (`therm` finds Thermal erosion), or its letters in
  order (`mtnrange` finds Mountain range). Arrows choose, Enter adds. A
  node can also be clicked or dragged from the Library.
- **Select** a node by clicking it (Shift or ⌘ adds to the selection), or
  drag a box around nodes on the empty graph. Drag a node to move it with
  the rest of the selection; let go, it snaps to the grid (the magnet at
  the graph's top right places nodes freely instead).
- **View** a node in the viewer: press **V**, or double-click it (a
  composite opens instead). The eye on a node says it is the one viewed.
  With none viewed, the viewer shows the active Map Output.
- **Name** a node: its settings' **Name** gives it a name to show. Its id,
  by which wires and scripts name it, never changes.
- **Bypass** a node (**B**, or its menu): it is left out, and what it feeds
  takes its first input instead. It shows dimmed, struck through, its
  thumbnail that of what passes through.
- **Fold** a node to its title with the chevron at its left, or **H**.
- **Copy and paste** nodes (⌘C, ⌘X, ⌘V): they go through the clipboard as
  JSON, with the wires among them, their frames and the composites they are
  made of, so that they paste into another world too. **⌘D** duplicates
  beside them.
- **Arrange**: the grid button lays the selected nodes (or all) out in
  columns, from their sources to their output; a node's menu also aligns
  the selected nodes' left edges or tops, and spaces them evenly.
- **Move around**: scroll to pan (or drag with the middle button, or with
  Space held), pinch or ⌘-scroll to zoom. **F** shows the whole graph. A
  minimap shows at the bottom right when part of the graph is out of
  view: click or drag in it to go there.
- **Undo** and **Redo** (⌘Z, ⌘⇧Z) take back the graph's changes while the
  pointer is over the area, inside composites too; a drag or a slider
  moved is one step.

Every change to what the graph makes goes through the graph's own editing
steps (add, set, link, rename, bypass, frame, group, expose…), the same a
script or a model uses: see [Terrain graphs](terrain-graphs.md).

Each node shows its result as a thumbnail, lit as the viewer lights it,
with the range of its values (in blocks for a height, from 0 to 1 for a
mask) when the graph is zoomed in.

## Composites

A composite is a node that holds a graph of its own, with inputs, outputs
and settings it exposes. **Mountain** ships with Massif; any nodes can be
made into one of the world's own.

- **Make a composite**: select nodes and press **⌘G** (or the composite
  button). They go inside a new composite, one node of it in their place;
  the wires that crossed into them become its inputs, those that left them
  its outputs. **⌘⇧G** (or **Ungroup**) puts a composite's nodes back.
- **Open** a composite: double-click it, click the stack at its top right,
  or **Open** in its settings. The graph shows what it holds, a breadcrumb
  at the top (`Graph › Mountain`) leads back, as **Esc** does. Its inputs
  and outputs are two columns at the graph's edges: wire from the inputs'
  column, and into the outputs' column to say what each output gives.
- **Massif's composites** open read only. **Edit a copy** makes the world's
  own copy, under a new name, which the node then is; its settings are
  kept.
- **Expose a setting**: inside a composite of the world, the **⤴** beside a
  node's setting shows it on the composite, under a label, in a group,
  with its range, folded away as advanced if you like. Every node of the
  composite then shows it in its settings, as a node's own.
- With nothing selected inside, the settings show the composite itself:
  its name, its group in the library, its inputs and outputs (add or
  remove them) and its exposed settings.

Composites are listed in the library under **Composites** and found by the
search like any node. They are saved with the world.

## Frames

Frames gather nodes under a title, a colour and a note; frames sit in
frames. Select nodes and press **⌘J** (or the frame button) to frame them,
in the frame they all sit in. Drag a frame by its title to move it with
everything it holds; a node dropped inside a frame joins it, dragged out it
leaves it; a frame dropped in another sits in it. The chevron folds a frame
to its title bar, its wires ending on it. Click its title to show its
settings (title, colour, note, folded); double-click to rename it;
right-click it for its colour, or to remove it (the nodes stay).

## Sockets and wires

What a socket carries shows in its colour and shape:

| Socket | Carries | Colour |
|---|---|---|
| ● | a height, in blocks | slate grey |
| ● | a mask, from 0 to 1 | amber |
| ● | a colour | pink |
| ● | a vector (wind, the way water flows) | indigo |
| ■ | a terrain (the block material), one per column | green |
| ■ | a biome of the game, one per column | teal |
| ■ | a painted layer (its number and strength) | olive |
| ▶ | points (where things stand) | cyan |
| ▶ | lines (roads, rivers, walls) | orange |
| ▶ | footprints (the ground things take) | tan |
| ▶ | instances (what stands there) | sky blue |
| ⬢ | a volume (caves, arches, overhangs) | violet |
| ◆ | a setting driven by a field (see [Settings](#settings)) | amber |

Circles carry values that vary smoothly, squares categories, triangles
geometry made whole, hexagons volumes, diamonds settings that vary over the
map. A filled socket is connected; an empty
ring is an input left empty, which takes the default its node says.

Drag from an output to an input to connect them; let go on an empty part
of the graph to add a node that takes what the wire carries, connected.
Drag from a connected input to take its wire elsewhere. A wire between
two different types (a height into a colour, a colour into a mask) is
converted on its way, and a dot on the wire says so; categories are never
made from numbers on a wire: a node says how. An input that cannot take
what the wire carries turns red under it, and says why and which node to
put between them (points into a mask: **Points to mask**). Right-click (or Alt-click) a
wire to cut it; double-click it to add a reroute point, which drags, and
which a right-click removes.

## The Map Output

A graph ends in one **Map Output**, as a Blender material ends in its
Material Output: one node with an input for everything the map can take,
in groups that fold (Ground and Surface open, and any group with an input
connected):

- **Ground**: Height, Water.
- **Surface**: Albedo (the ground's colour), Terrain, Biome.
- **Climate**: Temperature, Humidity. **Life**: Vegetation. **Underground**:
  Caves.
- **Rock over the void**: the top and underside of layers of rock over the
  ground (floating islands, arches).
- **Masks**: Flow, Wear, Deposition, Snow, and masks of one's own: **+ Mask**
  adds one (double-click its name on the node to rename it), written to the
  map under its name.

Every input is optional. A connected one shows a small picture of what
arrives and where from; an empty one shows in grey what stands for it
("the map's ground as it is", "none"). A graph can hold several Map
Outputs to compare variants: the one marked **Active** is the one the map
takes; click **Inactive** on another (or use its menu) to make it the
active one.

## Settings

The selected node's settings are made from the node's own description, in
groups: its main settings, **Shape**, **Detail**, and **Advanced** on demand.
Every setting has its editor: a slider with its unit for a number (the
slider covers the usual range; a value beyond it can be typed), a switch, a
choice, a colour, a seed with a die for another one, a file, points and
lines (placed on the viewer, or typed in a table), a curve from 0 to 1
(drag its points, double-click to add one, right-click to remove one), and
a gradient (drag its stops, click one to colour it, double-click the bar to
add one). A dot beside a setting changed from its default puts it back.
The foot of the panel says what the setting under the pointer does.

**◇ — settings driven by a field.** A number marked ◇ can vary over the
map: click the ◇ and the node gets a diamond socket for it; connect a mask
there and the setting takes, at each place, the mask's value there
(a mountain higher in the north, an erosion stronger on soft rock).

With no node selected, the panel shows the whole graph's settings: the
terrain's size, its seed, the sea level and the resolution simulations run
at in the previews.

## The viewer

The viewer shows the node viewed, or the active Map Output, lit as a
render: a sun from the north-west, soft shadows, light from the sky in
the hollows. **2D** shows it from above; **3D** from a camera turning
around it. **Clay** shows the shape alone, in neutral grey, as Gaea does;
**Colours** shows the ground's colours (the Albedo reaching it, or tints
by height when there is none). A mask shows in amber over the relief it
comes from; a colour, a terrain or a biome as its colours on that relief.
A picture comes coarse first, then sharp.

- 2D: drag to move, scroll to zoom around the pointer, double-click to see
  the whole terrain again.
- 3D: drag to turn around the terrain, scroll to come closer.
- **Shift + drag** turns the sun, in both.
- The sliders menu at the top right: how much the relief is exaggerated, contour lines, the mask's
  opacity, the sun's height, and whether thumbnails are in clay or in
  colours.
- The selected node's points or line (a range's spine, peaks) show over the
  2D picture: drag them, double-click to add one, right-click to remove
  one.
- **Live** shows the graph's result on the World map as it is edited,
  without writing it; **Apply to map** writes it, in one step that can be
  undone (see [Apply](#apply)).

## Apply

**Apply to map** writes everything the active Map Output carries into the
map, as one step that ⌘Z takes back whole:

- the ground (heights, water, terrains, colours as blocks, layers of rock,
  biomes of the climate), made window after window of 1024 blocks, so that
  a large map is written without ever holding all of it;
- what goes underground: the cave layers of its Cave Layer nodes (marked as
  the graph's, updated on the next Apply, never mixed with those painted by
  hand), the cave biomes, and the volumes carved out of the ground or added
  to it;
- its villages: the Map Output's structures and roads, laid on the ground
  as written. The villages an earlier Apply laid are replaced; structures
  placed by hand stay.

A dialog shows how far it is (the terrain window by window, then the
caves, then the villages); **Cancel** stops it and leaves the map as it
was. A map of 2048 blocks a side with a mountain and its erosion is
written in about a second.

Thumbnails and the viewer show every kind of output: points, lines,
footprints and instances drawn over the relief they stand on, and a volume
as its cut a little under the sea's level, laid over the relief.

## Keys

| Key | Over the graph |
|---|---|
| Tab, Shift A, right-click | add a node |
| V | view the node |
| double-click | open a composite, view another node |
| Esc | leave the composite open |
| B | bypass the selected nodes, or stop |
| H | fold or unfold the selected nodes |
| ⌘G, ⌘⇧G | make a composite, ungroup |
| ⌘J | frame the selected nodes |
| ⌘C, ⌘X, ⌘V | copy, cut, paste |
| ⌘D | duplicate the selected nodes |
| ⌘A | select every node |
| Delete, Backspace, X | remove the selected nodes |
| F, Home | show the whole graph |
| ⌘Z, ⌘⇧Z | undo, redo |
