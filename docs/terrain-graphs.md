# Terrain graphs

[← Back to the README](../README.md)

The **Terrain** side of Massif makes landscapes from nodes, in the spirit of Gaea and World Machine: shapes, erosion, rivers, climate, caves and villages wired into a graph, seen lit in a viewer, then written into the map. Switch to it at the top left of the window.

Its **Generator** workspace holds the **Terrain Generator** of 1.2, four panels that work together: the **Node Library**, the **Generator Viewer**, the **Generator Graph** and **Node Properties**. The first pages below are about it. A graph can also be written as text, in [MassifScript](massifscript.md).

Maps made with the generator of earlier versions keep it, as the **classic graph**: its panels (**Classic graph**, **Graph Viewport**, **Classic properties**, **Render Settings**) and its **Graph** and **Render** workspaces are offered for those maps only. Its pages come after the generator's, from [The classic graph](#the-classic-graph) on. A classic graph opens in the new generator migrated, node by node.

Every node and every setting is listed in [terrain-nodes.md](terrain-nodes.md).

## The Terrain Generator

The Terrain side opens on the **Generator** workspace, its panels marked **Experimental** while the generator finds its way. The samples and templates of the home screen open there, with their graph and its preview, and the World side only when you go to it (Apply, or the side switch).

[![Annotated generator with an alpine preview above the connected terrain nodes.](assets/workspaces/terrain-generator.webp)](assets/workspaces/terrain-generator.webp)

Follow the connections beneath the preview to see how each node contributes to the alpine terrain. [View full size](assets/workspaces/terrain-generator.webp).

The four panels are laid out as one, and like every panel of Massif each can be moved, swapped, put in tabs, floated in a window of its own or closed, and chosen again from any area's panel chooser, in its **Terrain** section:

- **Node Library**, on the left: the generator's nodes by category, to search and add. A node clicked lands in the middle of the graph's view; dragged, where it is dropped. Composites are listed under **Composites**.
- **Generator Viewer**, top middle: the selected node's terrain, or the Map Output's, lit, in 2D or 3D, with **Live** and **Apply to map** (see [The Generator Viewer](#the-generator-viewer)).
- **Generator Graph**, below it: the nodes and their wires, with the **Script** pane (MassifScript) and a status line at its foot: what is computing, how long the last picture took, the graphics card, the cache (see [The generator graph](#the-generator-graph)).
- **Node Properties**, on the right: the selected node's settings, or the graph's own when no node is selected (see [Node settings](#node-settings)).

They work together wherever they are: selecting a node in the graph shows its settings and its terrain, and undo over any of them undoes the graph's last change. The graph is saved with the world.

> 📷 **Screenshot:** the Terrain side's **Generator** workspace with the Alpine range sample open (home screen ▸ Samples ▸ Alpine range, then the Terrain side): the Node Library, the Viewer in 3D, the graph and Node Properties with the Alpine Range composite selected.

### What a graph makes

A graph ends in one **Map Output**, like Blender's
Material Output: one node with an input for each thing the map can take —
Height, Water, Albedo (the colour of the ground), Terrain, Biome,
Temperature and Humidity, Vegetation, Caves, four layers of rock (with their
colours) and any mask (`mask.snow`, `mask.flow`, `mask.<name>`). Each input is
optional and says what it stands for when left empty. Sockets have types, in
colours: heights, masks (0 to 1), colours, vectors (wind), terrains and
biomes; a height and a mask feed each other, a colour feeds a number by its
brightness, a number a colour as a grey.

### Nodes

Shapes a mountain map is made of (ranges along a drawn crest,
peaks placed by hand, alpine and rounded massifs, volcanoes, mesas, canyons,
glacial valleys, fault scarps, dunes, karst, craters, islands, archipelagos
along a chain, coastlines), adjustments (levels, curves, terraces, strata,
folds, warps, sharpening, transforms), simulations (thermal and stream
erosion, water flow), climate (temperature, wind, rainfall with rain shadows,
humidity, the game's biomes), masks of what the relief is (slope, height,
curvature, facing, sunlight, openness, snow, wetness, distance), colours
(gradients from nature, mixes) and terrains (layers by masks). Positions are
fractions of the terrain, distances and heights blocks. Every node and
setting is listed in [terrain-nodes.md](terrain-nodes.md).

### Light primitives, Transform and Combine

As in Gaea, a landscape can be built from a few light pieces rather than
one heavy node. **Mountain** makes one mountain in the middle of the
terrain from six settings: **Scale** (its width at the foot), **Height**,
**Style** (Basic, Eroded, Old, Alpine, Strata), **Bulk** (Low, a slender
peak, to High, a massif), **Reduce details** (leaves the finest relief to
erosion, and previews faster) and **Seed**. It costs about half what
Alpine mountains cost (10.5 against 23.4 ms for a tile of 256² at full
detail on one core). **Transform** then moves, turns, scales, stretches and
deforms a relief, at full detail, tile by tile (its first version worked at
the simulations' resolution and stays for graphs made with it). **Combine**
gathers any number of reliefs: connect **In 1**, and an empty **In 2**
appears, and so on; each input after the first has a **Mask** of its own,
and they combine in the order of their numbers by one mode (Blend, Add,
Subtract, Max, Min, Multiply, Average, Difference, Screen, Overlay, Power)
and **Strength**. In MassifScript the inputs are given in order,
`Mixer(a, b, c, mode: "max")`, a mask by its name in backquotes
(`` `mask.2`: m ``); in a graph's JSON they are `in.1`, `in.2`… and
`mask.2`, `mask.3`….

### The relief bench

`tools/relief_bench.sh fetch` downloads five real landforms once
(Copernicus GLO-30 elevation tiles, 168 MB, outside the repository: the
Bernese Alps, Monte Perdido, the Grand Canyon, the Appalachian plateau and
Pico in the Azores); `tools/relief_bench.sh run` measures each against
Massif's nodes on a terrain as wide (16.4 km), their heights brought to the
same range: the slope distribution, the hypsometric curve, the drainage and
ridge densities and the curvature of valley floors, with a gap from 0 (the
same) to 1 (far) for each, and a board of hillshades side by side
(`relief-bench.md`, `relief-bench.png`). The measures live in
`massif_terrain::bench`, so that work on erosion and shapes is judged by
numbers.

### Underground

A **Cave layer** node is one of the map's cave layers
made by the graph (chambers, tunnels, noodles and ravines with their
densities, sizes and heights, the roof, the surface entrances, sealed
aquifers and the lava floor), and its mask says where its caves go; a
**Cave biome** node paints lush caves, dripstone caves or the deep dark
between two heights where a mask says. **Merge cave layers** takes the
stronger of two, or, with **Both**, stacks cave biomes at their own heights
in the same columns. The composite **Cave System** puts them together over a
relief: the game's caves, lush caves under wet ground, the deep dark far
under the mountains, dripstone caves between, with its densities, depths,
water and lava and biomes as settings; open it to change its rules. They go
to the Map Output's **Caves** and **Cave biomes**: Apply adds or updates the
graph's own cave layer (marked as the graph's, removed when its node is),
leaves the layers painted by hand where the graph puts no caves, and the
caves are carved, sealed and decorated on export as painted ones are.
**Volumes** are for what cave layers cannot do: a **Tube** along a drawn
line or a river's or road's splines, a **Sphere** (a hall, or one at each
point Scatter places), a **Box**, **Cave noise** (the game's noise caves as a
volume), a **Noise volume**, combined by **Combine volumes** (union,
subtract, intersect, with a smooth seam), roughened by **Shape volume** and
kept to a band of heights, a mask or under the ground by **Limit volume**.
The Map Output's **Volume carve** cuts its volume out of the ground, **Volume
add** fills its volume with rock: Apply writes them into the map's sculpted
blocks, where only the volume reaches.

### Examples

Five graphs to start from: an alpine chain with a glacial
valley, chalk cliffs over coves, desert mesas over dunes, a volcanic
archipelago, a canyon in a high plateau.

### Graphs of earlier versions

A world whose graph was made with the generator above
opens with it migrated: each node becomes the node (or the few nodes) doing
the same, with its settings, links, place and seed. What the new generator
has no node for yet (floating islands, overhangs, tectonic plates, 1.2's
water erosion, rivers and lakes) is kept as nodes marked 1.2, which run the
old code. The noises are new, so a migrated graph keeps its shapes and
heights, not its exact patterns.

### For models and scripts

A graph is a readable JSON document (named
nodes, inputs naming what feeds them). [terrain-graph.schema.json](terrain-graph.schema.json)
is its JSON Schema and [terrain-nodes.md](terrain-nodes.md) its catalogue,
both written from the generator itself. From the command line:
`massif-cli terrain validate` names each mistake by its path and the name
probably meant; `terrain patch` applies edits a step at a time (add, set,
link, unlink, remove, rename) or a JSON Patch; `terrain render` draws a
graph; `terrain migrate` prints a world's graph, migrated; `terrain build`
builds a graph into a world.

## The generator graph

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
- **Zoom and move**: a mouse wheel or a pinch zooms around the pointer,
  from far enough out to see a big graph whole to close enough to read a
  node's every word; **⌘+** and **⌘−** zoom too, **⌘0** back to full size.
  Two fingers on a trackpad move the view, as dragging with the middle
  button does (or with Space held). **F**, the frame button, or a
  double-click on the empty ground shows the whole graph.
- **Minimap**: at the bottom left when part of the graph is out of view,
  the part in view outlined. Click in it to go there, drag the outline to
  move the view; the wheel over it still zooms the graph.
- **Undo** and **Redo** (⌘Z, ⌘⇧Z) take back the graph's changes while the
  pointer is over the area, inside composites too; a drag or a slider
  moved is one step.

Every change to what the graph makes goes through the graph's own editing
steps (add, set, link, rename, bypass, frame, group, expose…), the same a
script or a model uses: see [For models and scripts](#for-models-and-scripts).

Each node shows its result as a thumbnail, lit as the viewer lights it,
with the range of its values (in blocks for a height, from 0 to 1 for a
mask) when the graph is zoomed in.

> 📷 **Screenshot:** the Generator Graph with the add-node search open (Tab) after typing `therm`, the matching nodes listed.

### Composites

A composite is a node that holds a graph of its own, with inputs, outputs
and settings it exposes. Massif ships **Alpine Range** (a chain along a drawn crest, its rivers
cutting valleys up every flank, snow above a line), **Cave System** and
**Village Generator**; any nodes can be made into one of the world's own.

- **Make a composite**: select nodes and press **⌘G** (or the composite
  button). They go inside a new composite, one node of it in their place;
  the wires that crossed into them become its inputs, those that left them
  its outputs. **⌘⇧G** (or **Ungroup**) puts a composite's nodes back.
- **Open** a composite: double-click it, click the stack at its top right,
  or **Open** in its settings. The graph shows what it holds, a breadcrumb
  at the top (`Graph › Alpine Range`) leads back, as **Esc** does. Its inputs
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

> 📷 **Screenshot:** the Alpine Range composite opened (double-click it): the breadcrumb `Graph › Alpine Range` at the top, its inputs and outputs as columns at the graph's edges.

### Frames

Frames gather nodes under a title, a colour and a note; frames sit in
frames. Select nodes and press **⌘J** (or the frame button) to frame them,
in the frame they all sit in. Drag a frame by its title to move it with
everything it holds; a node dropped inside a frame joins it, dragged out it
leaves it; a frame dropped in another sits in it. The chevron folds a frame
to its title bar, its wires ending on it. Click its title to show its
settings (title, colour, note, folded); double-click to rename it;
right-click it for its colour, or to remove it (the nodes stay).

### Sockets and wires

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
| ◆ | a setting driven by a field (see [Settings](#node-settings)) | amber |

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

While a wire is drawn near it, the Map Output's folded groups that hold an
input taking what the wire carries open by themselves (a layer opens
Underground, for the caves), and fold again if the wire goes elsewhere; the
inputs that take it glow, on every node. Folded, a group says how many
inputs it holds, and how many are connected. A graph with no Map Output
says so at the top of the canvas, with a button that adds one, its Height
fed by the rightmost node making heights; Apply says so too.

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

## Node settings

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

> 📷 **Screenshot:** Node Properties of a Mountain range node, its groups (main, Shape, Detail) and a ◇ setting driven by a mask wired into its diamond socket.

## The Generator Viewer

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
- 3D: drag to turn around the terrain, scroll to come closer; turning
  keeps the camera's distance, only the wheel changes it.
- **Shift + drag** turns the sun, in both.
- The sliders menu at the top right: how much the relief is exaggerated,
  contour lines, the mask's opacity, the sun's height, the 3D view's
  **Background** (plain **Dark**, as in Gaea, by default, or the **Sky**),
  the **Resolution** (Auto: as many samples as the picture needs, at most
  1024 a side; or 1024, 2048, 4096 or 8192, the last two heavy in memory
  and slow, as the menu says), and whether thumbnails are in clay or in
  colours.
- The selected node's points or line (a range's spine, peaks) show over the
  2D picture: drag them, double-click to add one, right-click to remove
  one.
- **Live** shows the graph's result on the World map as it is edited,
  without writing it; **Apply to map** writes it, in one step that can be
  undone (see [Apply](#live-and-apply)).

In **3D**, the viewer draws with the same renderer as the classic Graph Viewport: the generator's output under a sky with soft light, clouds, the sea at the world's sea level and the sides of the terrain, with a camera you turn, move and zoom freely. Its bar at the foot opens the render settings (sun, clouds, haze, exposure), turns the clouds and the sea on and off, frames the relief and saves the view as an image. The **Dark** background shows a plain dark backdrop in place of the sky.

> 📷 **Screenshot:** the Generator Viewer in 3D over an alpine graph, with its bar at the foot (Render, Clouds, Sea, frame, save an image) and the settings menu open at the top right.

## Live and Apply

With Live on, the World side's map shows the graph as it is
edited, without writing it: the area in view first, sharp within a few
milliseconds on the graphics card, the rest of the map behind; simulations
run quick first and exact once the graph rests. **Apply** writes it to the
map in one undoable step, as Convert does.

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

> 📷 **Screenshot:** the World side's map with **Live** on, the graph's result drawn over the map before Apply, and the Apply progress dialog.

## MassifScript

A graph can be written as text, and text made into a graph: the **script** button at the top right of the Generator Graph (a page with `<>`) shows the **Script** pane. **Apply** builds the graph from the script (one step to undo), **From graph** writes the graph open as a script, and **Copy for AI** copies a guide to paste into any AI chat (ChatGPT, Claude, Mistral…): the language, every node, examples and the graph open now. Paste the script it answers into the pane, and Apply. See [MassifScript](massifscript.md) and [its reference](massifscript-reference.md). An MCP server, so that an AI assistant could work on the graph in Massif directly, is planned, not shipped.

> 📷 **Screenshot:** the Script pane beside the Generator Graph, a short script in it and the graph it built, with the **Copy for AI** button (the sparkles) highlighted.

## Generator keys

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
| Delete, Backspace, X | remove the selected nodes (a frame selected: the frame, its nodes staying) |
| F, Home, double-click the ground | show the whole graph |
| wheel, pinch, ⌘+ ⌘− ⌘0 | zoom (⌘0: full size) |
| ⌘Z, ⌘⇧Z | undo, redo |

## The classic graph

The generator of earlier versions stays for the maps made with it. Its panels show in the panel chooser's **Classic generator** section only for such a map (or while one of them is on screen), and its workspaces are **Graph** and **Render** on the Terrain side. The pages below describe it. A classic graph also opens in the Terrain Generator, migrated node by node (see [Graphs of earlier versions](#graphs-of-earlier-versions)).

## The Graph workspace

The classic graph makes its relief from
nothing — or starts from the map as it is, painted or imported from
WorldPainter, with the **Map** node (see below). Its **Graph** workspace is laid out
like Gaea: the **Graph Viewport** above, the **Classic graph** below, the
**Classic properties** on the right. The viewport shows the graph's output;
select a node and press **V** (or double-click it) to see the graph up to
that node — heights as a relief, masks in white over the relief they come
from, colours on it — and V again to go back to the output.

## The Render workspace

The **Render** workspace gives the viewport the stage, with the **Render
Settings** beside it, as when a Gaea terrain is rendered in Blender:

- the sun's direction, height and strength (Shift + drag in the viewport
  turns it), with soft cast shadows over the relief;
- a sky that follows it, blue by day, warm at sunset;
- volumetric clouds — cover, height, thickness, density, size, a seed to
  move them — lit by the sun and casting their shadows on the ground;
- haze with distance, as a share of the map whatever its size, and a mist
  over the lowest ground that shows between the mountains, not from above
  them; a sea at the world's sea level, the exposure and the height
  exaggeration;
- **Save image** renders the view at full quality, from 1920 × 1080 to
  7680 × 4320, as a PNG. The view is drawn from the preview: a finer
  preview (up to 2,048 samples) gives a sharper image.

The relief is shaded with smooth normals, and where it is steep with rock
drawn in the world's own space — lumps, grain, beds across the face — so
that cliffs are neither faceted nor stretched; elsewhere with the graph's
colours, mottled, grass in drier and greener patches, and a forest's crowns
where they are a forest's dark green. Each point sees as much of the sky as
the relief around it leaves it, worked out on the GPU. While the view moves
it is drawn at half the size, and sharp once it stops.

In the viewport, left drag turns the view, right or middle drag moves it,
the wheel or a pinch comes closer, a double-click frames the relief.

## Templates

**Templates** (in the graph's toolbar) start from a ready-made landscape,
each an ordinary graph to change, shown with a picture of what it makes on
this map:

| Template | What it makes |
|---|---|
| Alpine range | Eroded peaks over green hills, snow on the summits (the starter graph) |
| Archipelago | Tropical islands in a shallow sea, sandy beaches, green and rocky peaks |
| Karst towers | A forest of stone towers, each of its own height, with sheer walls fluted from top to foot and broken by ledges, rounded shoulders and broad green tops, overgrown scree at their feet and mossy walls, over a jungle crossed by rivers |
| Floating mountains | Mountains of rock floating over such towers, each of several masses with their undersides hanging to points: a level of rock over the void, above the mist |
| River valley | A wide valley between snowy ridged mountains, a river along its floor |
| Desert canyon | A winding, stepped canyon in a red plateau, buttes, a river at the bottom |
| Volcanic island | A volcano rising from the sea, black beaches, green lower slopes |
| Dune sea | Long dunes shaped by the wind, ripples over them |
| Rolling hills | Meadows, streams and ponds |
| Continents | Continents and oceans from tectonic plates, ranges where they collide, rivers to the sea, and climates from tundra to jungle written as the game's biomes |
| Paint this map | The map as it is, painted from its relief: grass, worn ground on the steeper slopes, rock on the cliffs, gravel where water runs, sand on the shores, snow on the heights |
| Rivers on this map | The map as it is, with rivers traced in its valleys and lakes in its hollows |

Templates follow the world: their sea is the world's sea level, and their
features grow with the map. Each also sets the sun, the clouds and the sea
of the viewport to suit it. "Back to the previous graph" undoes the choice.

## Editing a graph

Right-click the graph or press Tab to open the node library and type to
search: a node is found by its name in your language, in English or in any
language Massif speaks, by its first letters or by its letters in order
(`thrml` finds Thermal erosion). Results are grouped by family; ↑ and ↓
choose one, whose description shows under the list, Enter adds it and Esc
closes. With nothing typed, the nodes you added last come first. Drop a
wire on empty space to add a node already wired. Ports and
wires are coloured by what they carry: heights, masks or colours.

- Click a node to select it; Shift or ⌘ adds nodes to the selection, and a
  box dragged on the background selects all it touches. Dragging a node
  moves the whole selection.
- ⌘D duplicates the selection, ⌘C / ⌘X / ⌘V copy, cut and paste it with its
  wires (between worlds too), Delete removes it, ⌘A selects everything, Esc
  nothing; right-click (or Alt-click) a wire to cut it.
- Right-drag, middle-drag or Space + drag moves around; a mouse wheel or a
  pinch zooms, two fingers on a trackpad move; F frames the selection (or
  everything).
- ⌘Z / ⇧⌘Z on the Terrain side undo and redo the changes to the graph —
  and the conversions, in the order they were made.
- A line along the bottom of the graph says what can be done with what is
  selected.

Previews are evaluated on another thread with a cache, so changing a
setting computes only that node and what follows it.

## Nodes

| Family | Nodes |
|---|---|
| Landforms | Tectonic plates (height, ranges, faults), Mountain (alpine, rounded, eroded, spiky), Mountain range, Volcano, Crater, Crater field, Dunes, Canyon, Mesas, Island, Rocks, Floating islands (top, bottom, mask) |
| Primitives | Noise (rolling, ridged, billow), Cells (cones, ridges, or one height per cell), Radial and linear gradients, Shape, Constant |
| Adjust | Levels, Normalize, Curve, Terraces, Stratify, Clamp, Blur, Sharpen, Warp, Transform, Swirl, Fold, Invert height, Overhang (the rims of cliffs over the void) |
| Simulate | Erosion (heights, flow, deposits, wear), Thermal erosion (heights, talus), Snow, Rivers, Lakes, Sea (water and shore masks) |
| Combine | Combine (add, subtract, multiply, max, min, average, lerp, screen, overlay, difference), Blend through a mask |
| Masks | Select height, Select slope, Curvature (ridges or hollows), Facing, Occlusion, Flow, Invert |
| Climate | Temperature, Humidity, Biomes (the colours of the climate) |
| Colour | Auto colour (eight kinds of land), Gradient (twenty gradients from nature), Solid colour, Mix colours, Tint, Adjust colour |
| Output | Height output, Terrain output, Colour output, Water output, Layer output (rock between two heights, over the ground), Biome output, Noise paint (blocks mixed by a noise in order, the Paint tool's noise with its presets, where a mask says) |

## Rock over the void

A height map cannot hold an overhang or an island in the air; the map
can, its levels being spans of rock stacked in each column. **Layer
output** is the way from one to the other: it takes a top and a bottom —
from **Floating islands**, from **Overhang**, or from any two heights —
and optionally the colour of the top. The viewport draws up to two layers
over the relief, lit, casting their shadows on the ground and on each
other; viewing one of these nodes (V) shows it over the graph's ground.
Converted, each layer becomes a Solid level of the map named after what
makes it ("Floating islands (graph 1)"), filled again by later
conversions and removed when its output goes: columns of ground, air,
then rock, exported to Minecraft as they are.

## Working on a map that exists

The **Map** node gives the map as it
shows from the sky — its ground with the rock its levels add (towers,
floating islands, overhangs), the pits they carve and what was sculpted in
3D — with where its water lies and the colours of its terrains, read again
whenever the map changes (the levels the graph writes itself are left out).
Wire it where a landform would go: into Rivers, Lakes, Erosion, masks of
slope or height… The template **Paint this map** paints it from its relief
without moving a block of it — the tops of its towers and the floors of its
pits included — and **Rivers on this map** traces rivers in its valleys.

## Rivers

The Rivers node finds the way the water of the whole relief
goes: flooded up from the sea and the map's edges, every place drains to
its lowest way out, across hollows by their bottom, across flats as water
would wander. Where enough of the land drains (**Fewer**), a river runs: a
main stem from each mouth up to its farthest source, its tributaries off
it. Each is a smooth line laid in the bottom of its valley on the relief
itself, wider and deeper as water joins it (**Width** and **Depth** are the
largest river's); its water only ever goes down, stays under its banks and
meets the sea. Beds and banks (**Banks**: how far they slope down to the
water) are dug at every sample of the build — a 16k build gets rivers drawn
at the block, in under a second at 8k. The hollows a river runs through hold
lakes up to where they spill (**Lakes**: which ones; through the others the
river cuts a gorge). Its outputs: the dug relief, the beds as a mask (gravel
in them), and the water's level for the **Water output**.

Wire the Map's **Water level** into the Rivers' second input (the template
does) and the water already on the map — rivers and lakes painted in
WorldPainter — is kept: the rivers traced keep to its beds and join them,
nothing is dug where it lies, and it stays in the water given back. Rivers
over a coastal plain barely above the sea keep their water down to the
coast.

## Climate

**Temperature** is warm on one side of the map and cold on the
other, colder with height; **Humidity** comes from the sea, rides the wind
inland and rains out on the slopes facing it, leaving the land behind the
mountains dry. Both are masks (temperature from −30 °C to +45 °C).
**Biomes** gives the colours of the climate they make, and **Biome output**
writes it to the map: every column gets the biome of the game for its
climate (snowy plains, taiga, plains, forest, swamp, savanna, desert,
badlands, jungle, beaches, stony or frozen peaks, the oceans by their
warmth, rivers where the water runs) and, painted, its ground to suit
(snow, grass, sand, red sand, rock, gravel).

## Tectonic plates

A landform from plates, continental (land) or oceanic
(under the sea), each moving its own way: plates running into each other
raise ranges (the highest between two continents, a coastal range over a
trench where an ocean dives under a continent, an arc of islands between two
oceans), plates pulling apart open rifts or ridges under the sea, plates
sliding past each other leave faults; coasts wind, with capes, bays and
islands. In its settings, the plates are on a map of their own: drag one to
move it, the tip of its arrow to change how it moves, right-click it to make
it a continent or an ocean or to remove it; add plates, or scatter them
anew. The template **Continents** puts it all together.

## Over the map, not into it

With "over the map" checked (the default
for a graph that starts from the map), converting leaves the map's ground
as it is: what the graph digs goes to a Carve level, what it raises to a
Solid level, with its water, both written again at every conversion.
Change the graph, or paint the map, and convert again: nothing is baked
in. Deleting the two levels gives the map back; unchecked, the graph
replaces the ground.

## Size

**Size** says how much of the world the terrain covers: the map as it is
(the default: it writes to the map's own tiles), or a square of 1k to 32k
blocks around its middle — larger than the map, converting adds the tiles
it lacks, so a graph started on a 1k map makes a 16k world when set to 16k.
The viewport says the size it shows. Nodes measure in blocks, so a larger
terrain holds more mountains of the same size, not larger ones (templates
fit their features to it). Converting always works at one sample per block,
up to 16,384 a side, a little coarser beyond.

## Convert to voxels

**Convert to voxels** builds the graph over all of its size on another
thread, showing which node it is at, then writes it to the map as one
undoable step: the heights; the colour output turned
into real blocks of the game (from natural ground, rock, desert, snow and
ice, or every block), each column taking the block whose colour is closest
— by hue before lightness, so that a dark green is moss or grass rather
than mud — two neighbouring blocks mixing where the colour lies between
them; the terrain outputs painted over it; and the layers as levels over
the ground. Written into the ground, a graph's heights replace what was
sculpted in 3D on the tiles it rewrites (in the same undoable step):
the sculpture belonged to the old ground. Distances are in blocks, so a graph looks
the same at every resolution. The graph is saved with the world. When
nothing feeds the Height output, the viewport says so, with **Start from
this map**, which links a Map node to it.

Builds are made for large maps: nodes run on every core, erosion in
parallel blocks; simulations run at 4,096 samples at most and carry what
they changed to the full resolution, so the finest details are kept. The
starter graph builds at 16,384 × 16,384 in about 25 seconds and writes to a
16k map in about 2 more, in about 5.5 GB of memory; 8,192 × 8,192 takes
about 12 seconds and 2 GB. The floating mountains, with their two layers,
take about 55 seconds and 8 GB at 16,384 × 16,384.

