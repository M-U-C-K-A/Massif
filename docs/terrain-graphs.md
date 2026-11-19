# Terrain graphs

[← Back to the README](../README.md)

The Terrain side of Massif: a node-based terrain generator in the manner of
Gaea, its viewport and renderer, its templates, and how a graph becomes
blocks.

- [The Graph workspace](#the-graph-workspace)
- [The Render workspace](#the-render-workspace)
- [Templates](#templates)
- [Editing a graph](#editing-a-graph)
- [Nodes](#nodes)
- [Rock over the void](#rock-over-the-void)
- [Working on a map that exists](#working-on-a-map-that-exists)
- [Rivers](#rivers)
- [Climate](#climate)
- [Tectonic plates](#tectonic-plates)
- [Over the map, not into it](#over-the-map-not-into-it)
- [Size](#size)
- [Convert to voxels](#convert-to-voxels)
- [The new terrain generator (beta)](#the-new-terrain-generator-beta)

## The Graph workspace

The Terrain side holds the generator. A graph makes its relief from
nothing — or starts from the map as it is, painted or imported from
WorldPainter, with the **Map** node (see below). Its **Graph** workspace is laid out
like Gaea: the **Graph Viewport** above, the **Terrain Graph** below, the
**Node Properties** on the right. The viewport shows the graph's output;
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

## The new terrain generator (beta)

Massif 1.3 brings a new terrain generator, in beta beside the one above,
which stays as it is: turn it on in **Settings ▸ General ▸ New terrain
generator (beta)**, and the Terrain side opens its graph instead. A world
keeps both graphs; an older Massif opens it with its own.

**What it makes.** A graph ends in one **Map Output**, like Blender's
Material Output: one node with an input for each thing the map can take —
Height, Water, Albedo (the colour of the ground), Terrain, Biome,
Temperature and Humidity, Vegetation, Caves, four layers of rock (with their
colours) and any mask (`mask.snow`, `mask.flow`, `mask.<name>`). Each input is
optional and says what it stands for when left empty. Sockets have types, in
colours: heights, masks (0 to 1), colours, vectors (wind), terrains and
biomes; a height and a mask feed each other, a colour feeds a number by its
brightness, a number a colour as a grey.

**Nodes.** Shapes a mountain map is made of (ranges along a drawn crest,
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

**Live.** With Live on, the World side's map shows the graph as it is
edited, without writing it: the area in view first, sharp within a few
milliseconds on the graphics card, the rest of the map behind; simulations
run quick first and exact once the graph rests. **Apply** writes it to the
map in one undoable step, as Convert does.

**Graphs of 1.2.** A world whose graph was made with the generator above
opens with it migrated: each node becomes the node (or the few nodes) doing
the same, with its settings, links, place and seed. What the new generator
has no node for yet (floating islands, overhangs, tectonic plates, 1.2's
water erosion, rivers and lakes) is kept as nodes marked 1.2, which run the
old code. The noises are new, so a migrated graph keeps its shapes and
heights, not its exact patterns.

**Examples.** Five graphs to start from: an alpine chain with a glacial
valley, chalk cliffs over coves, desert mesas over dunes, a volcanic
archipelago, a canyon in a high plateau.

**For models and scripts.** A graph is a readable JSON document (named
nodes, inputs naming what feeds them). [terrain-graph.schema.json](terrain-graph.schema.json)
is its JSON Schema and [terrain-nodes.md](terrain-nodes.md) its catalogue,
both written from the generator itself. From the command line:
`massif-cli terrain validate` names each mistake by its path and the name
probably meant; `terrain patch` applies edits a step at a time (add, set,
link, unlink, remove, rename) or a JSON Patch; `terrain render` draws a
graph; `terrain migrate` prints a world's graph, migrated; `terrain build`
builds a graph into a world.

