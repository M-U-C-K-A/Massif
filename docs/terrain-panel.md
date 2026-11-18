# The terrain generator's panel (1.3)

[← Back to the README](../README.md) · [Terrain graphs](terrain-graphs.md) · [Every node](terrain-nodes.md)

The **Terrain Generator** area holds the generator of 1.3 in one place:
the **Library** on the left, the **Viewer** over the **Graph** in the
middle, the selected node's **settings** on the right, and a status line
at the foot (what is computing, how long the last picture took, the
graphics card, the cache). Choose it from any area's header, in the
Terrain section. The graph is saved with the world.

- [The graph](#the-graph)
- [Sockets and wires](#sockets-and-wires)
- [The Map Output](#the-map-output)
- [Settings](#settings)
- [The viewer](#the-viewer)
- [Keys](#keys)

## The graph

- **Add a node**: right-click the graph, press **Tab** (or Shift A), or use
  the **+** at the graph's top right, and type a few letters of its name:
  the start of a word (`therm` finds Thermal erosion), or its letters in
  order (`mtnrange` finds Mountain range). Arrows choose, Enter adds. A
  node can also be clicked or dragged from the Library.
- **Select** a node by clicking it (Shift or ⌘ adds to the selection), or
  drag a box around nodes on the empty graph. Drag a node to move it with
  the rest of the selection.
- **View** a node in the viewer: double-click it, or press **V**. The eye on
  a node says it is the one viewed. With none viewed, the viewer shows the
  active Map Output.
- **Fold** a node to its title with the chevron at its left, or **H**.
- **Frames** gather nodes under a title: select them and press **⌘J** (or
  the frame button). Drag a frame by its title to move it with its nodes,
  by its corner to size it; double-click the title to rename it,
  right-click it to remove the frame (the nodes stay).
- **Move around**: scroll to pan (or drag with the middle button, or with
  Space held), pinch or ⌘-scroll to zoom. **F** shows the whole graph. A
  minimap shows at the bottom right when part of the graph is out of
  view: click or drag in it to go there.
- **Undo** and **Redo** (⌘Z, ⌘⇧Z) take back the graph's changes while the
  pointer is over the area; a drag or a slider moved is one step.

Each node shows its result as a thumbnail, lit as the viewer lights it,
with the range of its values (in blocks for a height, from 0 to 1 for a
mask) when the graph is zoomed in.

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
| ◆ | a setting driven by a field (see [Settings](#settings)) | amber |

Circles carry values that vary smoothly, squares categories, diamonds
settings that vary over the map. A filled socket is connected; an empty
ring is an input left empty, which takes the default its node says.

Drag from an output to an input to connect them; let go on an empty part
of the graph to add a node that takes what the wire carries, connected.
Drag from a connected input to take its wire elsewhere. A wire between
two different types (a height into a colour, a colour into a mask) is
converted on its way, and a dot on the wire says so; categories are never
made from numbers on a wire: a node says how. Right-click (or Alt-click) a
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
  undone.

## Keys

| Key | Over the graph |
|---|---|
| Tab, Shift A, right-click | add a node |
| V, double-click | view the node |
| H | fold or unfold the selected nodes |
| ⌘J | frame the selected nodes |
| ⌘D | duplicate the selected nodes |
| ⌘A | select every node |
| Delete, Backspace, X | remove the selected nodes |
| F, Home | show the whole graph |
| ⌘Z, ⌘⇧Z | undo, redo |
