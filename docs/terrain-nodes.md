# Terrain graph nodes

<!-- Written by massif-terrain (format::catalog_markdown); do not edit by hand: `MASSIF_WRITE_DOCS=1 cargo test -p massif-terrain --test format` writes it again. -->

Every type of node a terrain graph of Massif can use, with its inputs, outputs, settings and an example. The JSON Schema of graphs is `terrain-graph.schema.json`, beside this file.

## How a graph is built

- A graph is a JSON object: `{ "format": "massif.terrain-graph", "version": 3, "terrain": { … }, "nodes": { … } }`. Only `nodes` is needed; `terrain` may give the terrain's `size` (blocks a side, the map's own when left out), its `seed` and its `sea_level`.
- Each node has an id of its own (letters, digits, `_` and `-`), by which the others refer to it and which never changes, and a `type` from this catalogue. `name` is what the editor shows, free to change. Their order does not matter.
- An input names what feeds it: a node's name (its first output), or `name.output` for another of its outputs. An optional input left out stands for what the catalogue says; a required one left out breaks the node, and what follows it.
- Settings left out take their defaults; a number outside a setting's range is brought back into it, with a warning.
- Positions (points, curves) are fractions of the terrain's side, from 0 (west, north) to 1 (east, south). Distances, widths and heights are in blocks, a block being a metre. Heights are absolute: the sea lies at the world's sea level (62 unless the map says otherwise), and Minecraft builds from y −64 to 320. A map of 16k is 16384 blocks a side.
- One `output.map` node, the Map Output, says what the map gets: its Height, and if wanted its Water, Albedo, Terrain, Biome, Temperature, Humidity, Vegetation, Caves, layers and any mask (`mask.<name>`). Each input left empty keeps what the map has, as the catalogue says.
- Sockets have types: height (blocks), mask (0 to 1), colour, vector, terrain and biome. A height and a mask feed each other as they are; a colour feeds a number by its brightness, a number feeds a colour as a grey; a terrain or a biome feeds only its own type.
- A setting marked drivable can follow a field: connect a height or a mask to the input `@<setting>`, and each place takes the value the field has there.
- Simulations (erosion, rainfall, humidity…) work over the whole terrain at the simulations' resolution; finer views of them carry the change they made.
- A seed left out comes from the node's id, so that two nodes never repeat each other; `terrain.seed` changes them all.
- A composite is a node that holds a graph: `massif:<name>` (shipped with Massif, such as `massif:mountain`) or `project:<name>` (defined under the document's `components`, each with its `label`, `inputs`, `outputs`, `exposed` settings and `nodes`; inside, `in.<name>` names its input). Its `params` are the settings it exposes. `"bypass": true` leaves a node out, what it feeds taking its first input.
- `frames` gather nodes under a title and a colour (`#rrggbb`), and nest (`parent`); they never change the terrain.
- `massif-cli terrain validate graph.json` says what is wrong, each mistake with its path (`nodes.hills.params.hieght`) and the name it probably meant. `massif-cli terrain patch` changes a graph a step at a time: `add`, `set`, `link`, `unlink`, `remove`, `rename` (the name shown), `bypass`, `frame`, `unframe`, `group` (nodes into a new composite), `ungroup`, `expose`, `unexpose`, `inside` (steps inside a composite), `port`, `unport`, `output`, `unoutput` (a composite's inputs and outputs), `fork` (a copy of a shipped composite to edit), or a JSON Patch.

## Sockets

| Type | Carries | Feeds |
|---|---|---|
| `height` | Heights of the ground, in blocks. | `height`, `mask`, `color` |
| `mask` | A value from 0 to 1 at each place: where something is, and how much. | `height`, `mask`, `color` |
| `color` | A colour at each place. | `height`, `mask`, `color` |
| `vector` | A direction and its strength at each place: wind, the way water flows. | `vector` |
| `terrain` | A terrain (the block material) at each place. | `terrain` |
| `biome` | A biome of the game at each place. | `biome` |
| `layer` | A painted layer of the world at each place (ground cover, objects, pockets, caves, annotations), and how strongly. | `layer` |
| `points` | Where things stand: points with a turn, a scale, a kind and numbers of their own. | `points` |
| `splines` | Lines through the terrain: roads, rivers, walls, each with a width and a kind. | `splines` |
| `footprints` | The ground things take: rectangles of columns, with the side they face and their entrance. | `footprints` |
| `instances` | What stands there: a pack's building, a structure of the game, an object, each placed and turned. | `instances` |
| `density` | A field over the volume, negative inside matter: caves, arches, overhangs. | `density` |

## Inputs

### Map — `input.map`

The map as it is: its ground, its water, its terrains and biomes, read again whenever the map changes. Start from the map you painted or imported: run erosion over it, put rivers in its valleys, colour it from its relief, keep its terrains where a mask says. Water is 1 where water lies over the ground, Level the water's surface. The levels the graph writes itself are left out.

It reads the map as it is, and follows it when it changes.

Outputs:

- `height` (height) — The ground's height.
- `water` (mask) — 1 where water lies over the ground.
- `level` (height) — The water's surface where there is water, the ground's elsewhere.
- `terrain` (terrain) — The terrain of each column.
- `biome` (biome) — The biome of each column.
- `color` (color) — The terrains' colours.

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "map": { "type": "input.map" },
    "out": { "type": "output.map", "inputs": { "height": "map" } }
  }
}
```

### Heightmap — `input.heightmap`

A heightmap image turned into relief: black at Low, white at High, laid over a rectangle of the terrain. Choose the file (PNG, TIFF, JPEG, BMP or a square RAW of 16-bit samples; 16-bit greys keep their precision), then where it lies: X, Z, Width and Depth as shares of the terrain, Rotation, and how its shape fits. Outside it, the relief is Low (or the image repeats when Tile is on); the Mask output says where the image lies, fading over Edge fade, to blend several heightmaps or a heightmap into a generated relief.

It reads the files its settings name, and follows them when they change.

Outputs:

- `height` (height) — The relief the image gives.
- `mask` (mask) — 1 where the image lies, fading at its edges.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `file` | file | — | .png, .tif, .tiff, .jpg, .jpeg, .bmp, .raw, .r16 | The heightmap: an image (16-bit greys keep their precision) or a square RAW of 16-bit samples. |
| `low` | number | 40 | -2048 to 4096 blocks | The height black stands for, in blocks. Usually -64 to 320. |
| `high` | number | 200 | -2048 to 4096 blocks | The height white stands for, in blocks. Usually -64 to 320. |
| `x` | number | 0 | -4 to 4 fraction of the terrain's side | Where the image's west edge lies, as a share of the terrain. In fraction of the terrain's side. Usually -0.5 to 1.5. |
| `z` | number | 0 | -4 to 4 fraction of the terrain's side | Where the image's north edge lies, as a share of the terrain. In fraction of the terrain's side. Usually -0.5 to 1.5. |
| `width` | number | 1 | 0.001 to 16 fraction of the terrain's side | How wide the image is, as a share of the terrain. In fraction of the terrain's side. Usually 0.01 to 2. |
| `depth` | number | 1 | 0.001 to 16 fraction of the terrain's side | How deep (north to south) the image is, as a share of the terrain. In fraction of the terrain's side. Usually 0.01 to 2. |
| `rotation` | number | 0 | -180 to 180 degrees | How far the image is turned around its middle, in degrees. |
| `fit` | choice | `stretch` | `stretch`, `contain`, `cover` | How the image's shape meets the rectangle's. `stretch`: The image fills the rectangle, stretched if its shape differs. `contain`: The whole image within the rectangle, its shape kept. `cover`: The rectangle filled, the image's shape kept, its edges cut. |
| `edge` | number | 0 | 0 to 16000 blocks | How far inside its edges the image's mask fades out, in blocks. Usually 0 to 1000. |
| `tile` | boolean | false |  | Repeat the image beyond its rectangle. |
| `invert` | boolean | false |  | Read white as low and black as high. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "heightmap": { "type": "input.heightmap", "params": { "file": "heightmap.png" } },
    "out": { "type": "output.map", "inputs": { "height": "heightmap" } }
  }
}
```

### Image — `input.image`

An image read as a mask (one of its channels, over a range) and as colours: a temperature or rainfall map, a painted mask, a satellite colour map. Choose the file and where it lies, as for a heightmap. Channel picks which of its values becomes the mask, Low and High the values read as 0 and 1 (a temperature map stored from −30 to 45 °C reads with Low −30 and High 45 into a temperature mask). The Colour output gives its colours, for the Map Output's Albedo.

It reads the files its settings name, and follows them when they change.

Outputs:

- `mask` (mask) — The channel read, from 0 to 1.
- `color` (color) — The image's colours.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `file` | file | — | .png, .tif, .tiff, .jpg, .jpeg, .bmp | The image: a climate map, a painted mask, a colour map. |
| `channel` | choice | `luminance` | `luminance`, `red`, `green`, `blue`, `alpha` | Which of the image's values makes the mask. `luminance`: The image's brightness. `red`: The red channel. `green`: The green channel. `blue`: The blue channel. `alpha`: The alpha (opacity) channel. |
| `low` | number | 0 | -1000 to 1000 | The value read as 0. Usually 0 to 1. |
| `high` | number | 1 | -1000 to 1000 | The value read as 1. Usually 0 to 1. |
| `invert` | boolean | false |  | Turn the mask over. |
| `x` | number | 0 | -4 to 4 fraction of the terrain's side | Where the image's west edge lies, as a share of the terrain. In fraction of the terrain's side. Usually -0.5 to 1.5. |
| `z` | number | 0 | -4 to 4 fraction of the terrain's side | Where the image's north edge lies, as a share of the terrain. In fraction of the terrain's side. Usually -0.5 to 1.5. |
| `width` | number | 1 | 0.001 to 16 fraction of the terrain's side | How wide the image is, as a share of the terrain. In fraction of the terrain's side. Usually 0.01 to 2. |
| `depth` | number | 1 | 0.001 to 16 fraction of the terrain's side | How deep (north to south) the image is, as a share of the terrain. In fraction of the terrain's side. Usually 0.01 to 2. |
| `rotation` | number | 0 | -180 to 180 degrees | How far the image is turned around its middle, in degrees. |
| `fit` | choice | `stretch` | `stretch`, `contain`, `cover` | How the image's shape meets the rectangle's. `stretch`: The image fills the rectangle, stretched if its shape differs. `contain`: The whole image within the rectangle, its shape kept. `cover`: The rectangle filled, the image's shape kept, its edges cut. |
| `tile` | boolean | false |  | Repeat the image beyond its rectangle. |
| `outside` | number | 0 | 0 to 1 | The mask's value outside the image. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "image": { "type": "input.image", "params": { "file": "heightmap.png" } },
    "out": { "type": "output.map", "inputs": { "height": "relief", "mask.mask": "image" } },
    "relief": { "type": "noise.fbm", "params": { "height": 250, "scale": 1500 } }
  }
}
```

### World file (1.2) — `legacy.file_input`

A world brought in (WorldPainter's .world, Massif's .massif): its surface, its water and its colours. Path is the world's file; Fit stretches it over the terrain, or lays it at its own blocks. Low and High are the heights a height map's black and white stand for (Heightmap reads those better).

A simulation: it works over the whole terrain at the simulations' resolution.

It reads the files its settings name, and follows them when they change.

Outputs:

- `height` (height) — The world's surface.
- `water` (mask) — Where water lies.
- `colour` (color) — The colour of its ground.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `path` | file | — | .world, .massif, .png, .r16, .r32 | The file. |
| `low` | number | 40 | -4096 to 4096 blocks | The height black stands for. In blocks. |
| `high` | number | 220 | -4096 to 4096 blocks | The height white stands for. In blocks. |
| `fit` | boolean | true |  | Stretched over the terrain, or at its own blocks. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "file_input": { "type": "legacy.file_input", "params": { "path": "heightmap.png" } },
    "out": { "type": "output.map", "inputs": { "height": "file_input" } }
  }
}
```

## Primitives

### Constant — `pattern.constant`

The same height everywhere. A flat ground at one height: a start to build on, or a level to combine with another relief.

Outputs:

- `height` (height) — The height, everywhere.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `value` | number | 64 | -2048 to 4096 blocks | The height of the ground everywhere, in blocks. Usually -64 to 320. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "constant": { "type": "pattern.constant" },
    "out": { "type": "output.map", "inputs": { "height": "constant" } }
  }
}
```

### Noise — `noise.fbm`

Fractal noise: relief at every scale, from its largest features down to fine detail. The base of most landscapes. Scale is the size of the largest features; each octave adds detail half as large (lacunarity) and weaker (roughness). Rolling makes hills, Ridged mountain crests, Billowy rounded mounds. Height is how far the lowest and highest points lie apart, Base where the lowest lies.

Outputs:

- `height` (height) — The relief, from Base to Base + Height.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `style` | choice | `fbm` | `fbm`, `ridged`, `billow` | How the octaves add up. `fbm`: Octaves summed: smooth hills and valleys. `ridged`: Octaves folded into sharp crests, with finer ridges branching where the coarser ones are high: mountain ranges. `billow`: Octaves folded the other way: puffy, rounded mounds with creases between them. |
| `scale` | number | 1000 | 2 to 65536 blocks | Blocks across the largest features. Usually 16 to 8000. |
| `height` | number | 200 | 0 to 4096 blocks | How far the lowest and the highest points lie apart, in blocks. Usually 0 to 1000. A field connected to the input `@height` drives it place by place. |
| `base` | number | 40 | -2048 to 4096 blocks | The height of the lowest points, in blocks. Usually -64 to 320. A field connected to the input `@base` drives it place by place. |
| `octaves` | integer | 8 | 1 to 16 | Octaves summed: each adds detail twice as fine. |
| `roughness` | number | 0.5 | 0.05 to 0.95 | How strong each octave is next to the one before: higher, rougher. |
| `lacunarity` | number | 2 | 1.2 to 4 | How much finer each octave is than the one before. |
| `offset_x` | number | 0 | -1000000 to 1000000 blocks | Moves the noise east, in blocks. Usually -10000 to 10000. |
| `offset_z` | number | 0 | -1000000 to 1000000 blocks | Moves the noise south, in blocks. Usually -10000 to 10000. |
| `seed` | seed | — | 0 to 4294967295 | Another seed gives another variation of the same settings. Left out, it comes from the node's name. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "fbm": { "type": "noise.fbm" },
    "out": { "type": "output.map", "inputs": { "height": "fbm" } }
  }
}
```

### Eroded noise — `noise.eroded`

Fractal noise whose fine detail fades on slopes and gathers on heights: smooth flanks, branching valleys, the look of an eroded relief without simulating it. Each octave is weakened where the coarser ones slope (after Iñigo Quilez), so slopes stay smooth and detail stays on crests and in hollows. Erosion is how strongly. A quick base for hills and old mountains, or a mask that follows the relief's structure.

Outputs:

- `height` (height) — The relief, from Base to Base + Height.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `scale` | number | 1200 | 2 to 65536 blocks | Blocks across the largest features. Usually 16 to 8000. |
| `height` | number | 120 | 0 to 4096 blocks | How far the lowest and the highest points lie apart, in blocks. Usually 0 to 600. A field connected to the input `@height` drives it place by place. |
| `base` | number | 62 | -2048 to 4096 blocks | The height of the lowest points, in blocks. Usually -64 to 320. A field connected to the input `@base` drives it place by place. |
| `erosion` | number | 1.5 | 0 to 6 | How strongly slopes smooth the finer octaves. |
| `roughness` | number | 0.5 | 0.05 to 0.95 | How strong each octave is next to the one before. |
| `seed` | seed | — | 0 to 4294967295 | Another seed gives another variation of the same settings. Left out, it comes from the node's name. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "eroded": { "type": "noise.eroded" },
    "out": { "type": "output.map", "inputs": { "height": "eroded" } }
  }
}
```

### Cells — `noise.cells`

Cellular (Voronoi) noise: points scattered one per cell, and a value from where a sample lies among them — pits, pillows, ridges along the edges, or a mosaic. Scale is how far apart the points are, Jitter how far they stray from a grid. Distance makes round pits around the points, Edges ridges between cells (dried mud, crazed rock), Cells a mosaic of flat pieces (terraces, fields). Height and Base place the result in blocks; for a mask, 1 and 0.

Outputs:

- `height` (height) — The cells, from Base to Base + Height.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `mode` | choice | `edges` | `f1`, `f2`, `edges`, `cells` | What value a sample takes. `f1`: Distance to the nearest point: round pits around each point, high between them. `f2`: Distance to the second nearest point: soft, pillowy cells. `edges`: How far from the edge between two cells: ridges along the edges (cracks when inverted). `cells`: One value per cell: a mosaic of flat pieces. |
| `scale` | number | 300 | 2 to 65536 blocks | How far apart the points are, in blocks. Usually 8 to 4000. |
| `jitter` | number | 0.9 | 0 to 1 | How far the points stray from a grid. |
| `height` | number | 1 | -4096 to 4096 blocks | How far the lowest and the highest values lie apart (1 for a mask). In blocks. Usually 0 to 200. A field connected to the input `@height` drives it place by place. |
| `base` | number | 0 | -2048 to 4096 blocks | The lowest value (0 for a mask). In blocks. Usually -64 to 320. A field connected to the input `@base` drives it place by place. |
| `seed` | seed | — | 0 to 4294967295 | Another seed gives another variation of the same settings. Left out, it comes from the node's name. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "cells": { "type": "noise.cells" },
    "out": { "type": "output.map", "inputs": { "height": "cells" } }
  }
}
```

### Gradient — `pattern.gradient`

A value rising from one point to another, straight or around the first: tilt a relief, fade a mask towards a coast, raise a dome. Place the two ends on the viewer: the value is 0 at the first and 1 at the second (Radial: 0 at the first, 1 at the second's distance from it), shaped by the Profile; Height and Base place it in blocks. Multiply a relief by it to make it fade out, or add it to tilt land towards the sea.

Outputs:

- `value` (mask) — From Base at the first point to Base + Height at the second.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `kind` | choice | `linear` | `linear`, `radial` | Straight, or around the first point. `linear`: From the first point to the second, straight. `radial`: Out from the first point, reaching 1 at the second's distance. |
| `ends` | points | none | up to 2; each `x`, `z` | Where the value is 0, then where it is 1, placed on the viewer. |
| `profile` | profile | `[[0.0,0.0],[1.0,1.0]]` |  | How the value rises from the first point (left) to the second (right). |
| `height` | number | 1 | -4096 to 4096 blocks | The value at the second point, over Base (1 for a mask). In blocks. Usually -200 to 200. |
| `base` | number | 0 | -2048 to 4096 blocks | The value at the first point (0 for a mask). In blocks. Usually -64 to 320. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "gradient": { "type": "pattern.gradient", "params": { "ends": [{ "x": 0.35, "z": 0.4 }, { "x": 0.65, "z": 0.6 }] } },
    "out": { "type": "output.map", "inputs": { "height": "relief", "mask.value": "gradient" } },
    "relief": { "type": "noise.fbm", "params": { "height": 250, "scale": 1500 } }
  }
}
```

### Shape — `pattern.shape`

A geometric shape as a mask: a disc, a square, a polygon or a star, its edge as soft as wanted. Place its centre on the viewer; Radius is its size, Softness how far its edge fades (0 sharp), Angle turns it. Use it to keep something within an area, to raise a platform, to cut a crater-like hollow.

Outputs:

- `mask` (mask) — 1 inside the shape, 0 outside.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `shape` | choice | `circle` | `circle`, `square`, `polygon`, `star` | The shape. `circle`: A disc. `square`: A square, turned by Angle. `polygon`: A regular polygon of Sides sides. `star`: A star of Sides points. |
| `center` | points | none | up to 1; each `x`, `z` | Where the shape's centre is, placed on the viewer. |
| `radius` | number | 1000 | 1 to 65536 blocks | How far the shape reaches from its centre, in blocks. Usually 10 to 8000. |
| `softness` | number | 200 | 0 to 16000 blocks | How far the edge fades, in blocks. Usually 0 to 2000. |
| `sides` | integer | 6 | 3 to 32 | How many sides (or points) a polygon (or a star) has. |
| `angle` | number | 0 | -180 to 180 degrees | How far the shape is turned, in degrees. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "out": { "type": "output.map", "inputs": { "height": "relief", "mask.mask": "shape" } },
    "relief": { "type": "noise.fbm", "params": { "height": 250, "scale": 1500 } },
    "shape": { "type": "pattern.shape", "params": { "center": [{ "x": 0.35, "z": 0.4 }, { "x": 0.65, "z": 0.6 }] } }
  }
}
```

### Rocks (1.2) — `legacy.rocks`

Rocky detail around 0, to add to a relief. Scale is the size of the rocks, Height how high they stand, Sharpness from rounded boulders (0) to sharp crags (1). Add it to a relief with Blend.

A simulation: it works over the whole terrain at the simulations' resolution.

Outputs:

- `out` (height) — The rocks, around 0.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `scale` | number | 90 | 2 to 8000 blocks | The size of the rocks. In blocks. |
| `height` | number | 12 | 0 to 1000 blocks | How high they stand. In blocks. |
| `sharpness` | number | 0.6 | 0 to 1 | From rounded boulders (0) to sharp crags (1). |
| `seed` | integer | 10 | 0 to 4294967295 | Another seed places them otherwise. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "out": { "type": "output.map", "inputs": { "height": "rocks" } },
    "rocks": { "type": "legacy.rocks" }
  }
}
```

## Shapes

### Alpine mountains — `mountain.alpine`

Young, sharp mountains: pyramidal summits, knife-edge ridges and deep valleys, like the Alps or the Himalaya. Mountains over an area: summits about Peak spacing apart joined by ridges, valleys between them, and the relief of Detail size on their slopes. Connect a mask to Area to say where they stand (a painted area, a coastline's land), or let Coverage spread them. Height is the summits' height over Base; the Profile shapes the massif from its edges to its heart. Run erosion after them for valleys carved by water.

Inputs:

- `area` (mask, optional) — Where the mountains stand, from 0 to 1 (where Coverage says, when unconnected).

Outputs:

- `height` (height) — The mountains over Base.
- `mask` (mask) — 1 on the mountains, fading to 0 at their feet.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `height` | number | 220 | 0 to 4096 blocks | How high the summits rise over Base, in blocks (a Minecraft world stops at 320 unless a data pack raises it). Usually 0 to 600. A field connected to the input `@height` drives it place by place. |
| `base` | number | 70 | -2048 to 4096 blocks | The height of the ground at the mountains' feet, in blocks (0 to add them to another relief). Usually -64 to 320. |
| `peak_spacing` | number | 500 | 64 to 16000 blocks | How far apart the summits stand, in blocks. Usually 200 to 4000. |
| `coverage` | number | 0.55 | 0 to 1 | How much of the terrain the mountains cover when Area is unconnected. |
| `profile` | profile | `[[0.0,0.0],[0.3,0.08],[0.65,0.5],[1.0,1.0]]` |  | The shape of the flanks, from the foot (left) to the crest (right). |
| `sharpness` | number | 0.8 | 0 to 1 | 0 gives rounded crests and smooth valleys, 1 knife-edge arêtes. |
| `roughness` | number | 0.55 | 0 to 1 | How much small relief the slopes carry. |
| `detail` | number | 160 | 16 to 8000 blocks | The size of the largest relief on the slopes (ridges, gullies), in blocks. Usually 32 to 2000. |
| `warp` | number | 0.35 | 0 to 1 | How far the mountains' lines wander from straight. |
| `seed` | seed | — | 0 to 4294967295 | Another seed gives another variation of the same settings. Left out, it comes from the node's name. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "alpine": { "type": "mountain.alpine" },
    "out": { "type": "output.map", "inputs": { "height": "alpine" } }
  }
}
```

### Rounded mountains — `mountain.rounded`

Old, worn mountains: rounded summits, broad shoulders and gentle valleys, like the Appalachians or the Vosges. Mountains over an area: summits about Peak spacing apart joined by ridges, valleys between them, and the relief of Detail size on their slopes. Connect a mask to Area to say where they stand (a painted area, a coastline's land), or let Coverage spread them. Height is the summits' height over Base; the Profile shapes the massif from its edges to its heart. Run erosion after them for valleys carved by water.

Inputs:

- `area` (mask, optional) — Where the mountains stand, from 0 to 1 (where Coverage says, when unconnected).

Outputs:

- `height` (height) — The mountains over Base.
- `mask` (mask) — 1 on the mountains, fading to 0 at their feet.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `height` | number | 120 | 0 to 4096 blocks | How high the summits rise over Base, in blocks (a Minecraft world stops at 320 unless a data pack raises it). Usually 0 to 600. A field connected to the input `@height` drives it place by place. |
| `base` | number | 70 | -2048 to 4096 blocks | The height of the ground at the mountains' feet, in blocks (0 to add them to another relief). Usually -64 to 320. |
| `peak_spacing` | number | 800 | 64 to 16000 blocks | How far apart the summits stand, in blocks. Usually 200 to 6000. |
| `coverage` | number | 0.6 | 0 to 1 | How much of the terrain the mountains cover when Area is unconnected. |
| `profile` | profile | `[[0.0,0.0],[0.3,0.3],[0.7,0.82],[1.0,1.0]]` |  | The shape of the flanks, from the foot (left) to the crest (right). |
| `sharpness` | number | 0.12 | 0 to 1 | 0 gives rounded crests and smooth valleys, 1 knife-edge arêtes. |
| `roughness` | number | 0.4 | 0 to 1 | How much small relief the slopes carry. |
| `detail` | number | 260 | 16 to 8000 blocks | The size of the largest relief on the slopes (ridges, gullies), in blocks. Usually 32 to 2000. |
| `warp` | number | 0.35 | 0 to 1 | How far the mountains' lines wander from straight. |
| `seed` | seed | — | 0 to 4294967295 | Another seed gives another variation of the same settings. Left out, it comes from the node's name. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "out": { "type": "output.map", "inputs": { "height": "rounded" } },
    "rounded": { "type": "mountain.rounded" }
  }
}
```

### Mountain range — `mountain.range`

A range of mountains along a line drawn on the terrain: a crest of summits and saddles, with side ridges running down to its feet. Draw the Spine on the viewer: the crest follows it, summits about Peak spacing apart with saddles between (Saddles: how deep), and side ridges (Spurs) run down the flanks every Spur spacing, valleys between them. Width is the range's width from foot to foot; each point of the spine can make it higher or wider there. Asymmetry makes one side steeper (positive: the left of the way the spine runs). Combine with a coastline or a plain with Max, or Add with Base 0.

Outputs:

- `height` (height) — The range over Base.
- `mask` (mask) — 1 on the mountains, fading to 0 at their feet.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `spine` | curve | none | each `x`, `z`, `height`, `width` | The line the crest follows, drawn on the viewer; each point can make the range higher or wider there. |
| `height` | number | 230 | 0 to 4096 blocks | How high the summits rise over Base, in blocks (a Minecraft world stops at 320 unless a data pack raises it). Usually 0 to 600. A field connected to the input `@height` drives it place by place. |
| `base` | number | 70 | -2048 to 4096 blocks | The height of the ground at the mountains' feet, in blocks (0 to add them to another relief). Usually -64 to 320. |
| `width` | number | 1800 | 64 to 32000 blocks | How wide the range is from foot to foot, in blocks. Usually 300 to 10000. |
| `profile` | profile | `[[0.0,0.0],[0.25,0.14],[0.6,0.62],[1.0,1.0]]` |  | The shape of the flanks, from the foot (left) to the crest (right). |
| `peak_spacing` | number | 450 | 32 to 16000 blocks | How far apart the summits stand along the crest, in blocks. Usually 100 to 4000. |
| `saddles` | number | 0.45 | 0 to 1 | How deep the crest dips between summits. |
| `spurs` | number | 0.6 | 0 to 1 | How strongly side ridges run down the flanks, valleys between them. |
| `spur_spacing` | number | 260 | 32 to 8000 blocks | How far apart the side ridges are, in blocks. Usually 100 to 2000. |
| `asymmetry` | number | 0 | -1 to 1 | Makes one flank wider and gentler, the other steeper: positive widens the left of the way the spine runs. |
| `sharpness` | number | 0.75 | 0 to 1 | 0 gives rounded crests and smooth valleys, 1 knife-edge arêtes. |
| `roughness` | number | 0.5 | 0 to 1 | How much small relief the slopes carry. |
| `detail` | number | 150 | 16 to 8000 blocks | The size of the largest relief on the slopes (ridges, gullies), in blocks. Usually 32 to 2000. |
| `warp` | number | 0.35 | 0 to 1 | How far the mountains' lines wander from straight. |
| `seed` | seed | — | 0 to 4294967295 | Another seed gives another variation of the same settings. Left out, it comes from the node's name. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "out": { "type": "output.map", "inputs": { "height": "range" } },
    "range": { "type": "mountain.range", "params": { "spine": [{ "x": 0.2, "z": 0.55 }, { "x": 0.5, "z": 0.45 }, { "x": 0.8, "z": 0.5 }] } }
  }
}
```

### Peaks — `mountain.peaks`

Summits placed by hand, each with its height and reach: horns with ridges (arêtes) running down from the top. Place the peaks on the viewer; each has its own height and radius. Arêtes is how many main ridges run down from each summit (3 or 4 for a horn like the Matterhorn), Arête strength how deep the faces between them are cut. Where peaks meet, the higher one wins, with a saddle between. Add them to a range or a massif for the mountain everyone must recognise.

Outputs:

- `height` (height) — The peaks over Base.
- `mask` (mask) — 1 on the mountains, fading to 0 at their feet.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `peaks` | points | none | up to 64; each `x`, `z`, `height`, `radius` | The summits, placed on the viewer, each with its height and radius. |
| `base` | number | 70 | -2048 to 4096 blocks | The height of the ground at the mountains' feet, in blocks (0 to add them to another relief). Usually -64 to 320. |
| `profile` | profile | `[[0.0,0.0],[0.25,0.14],[0.6,0.62],[1.0,1.0]]` |  | The shape of the flanks, from the foot (left) to the crest (right). |
| `aretes` | integer | 4 | 2 to 8 | How many main ridges run down from each summit. |
| `arete_strength` | number | 0.7 | 0 to 1 | How deep the faces between the ridges are hollowed: 0 a round cone, 1 sharp ridges over deep cirques. |
| `sharpness` | number | 0.8 | 0 to 1 | 0 gives rounded crests and smooth valleys, 1 knife-edge arêtes. |
| `roughness` | number | 0.5 | 0 to 1 | How much small relief the slopes carry. |
| `detail` | number | 120 | 16 to 8000 blocks | The size of the largest relief on the slopes (ridges, gullies), in blocks. Usually 32 to 2000. |
| `warp` | number | 0.35 | 0 to 1 | How far the mountains' lines wander from straight. |
| `seed` | seed | — | 0 to 4294967295 | Another seed gives another variation of the same settings. Left out, it comes from the node's name. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "out": { "type": "output.map", "inputs": { "height": "peaks" } },
    "peaks": { "type": "mountain.peaks", "params": { "peaks": [{ "x": 0.35, "z": 0.4 }, { "x": 0.65, "z": 0.6 }] } }
  }
}
```

### Volcano — `mountain.volcano`

Volcanoes placed by hand: steep stratovolcanoes, broad shields, calderas or cinder cones, with a crater, gullies down their flanks and smaller cones on them. Place each volcano on the viewer with its height and radius. The kind sets its profile; Crater is the crater's width as a share of the radius (a caldera's is wider), Crater depth how far below the rim its floor lies. Gullies are the channels old flows cut down the flanks; Cones adds smaller vents on them. On an island, take the Max with the sea floor.

Outputs:

- `height` (height) — The volcanoes over Base.
- `mask` (mask) — 1 on the landform, 0 around it.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `volcanoes` | points | none | up to 16; each `x`, `z`, `height`, `radius` | The volcanoes, placed on the viewer, each with its height and radius. |
| `kind` | choice | `strato` | `strato`, `shield`, `caldera`, `cinder` | The kind of volcano: its profile and its crater. `strato`: Steep and concave, a small crater at the top: Fuji, Etna. `shield`: Broad and gentle, its top a wide dome with a pit: Mauna Loa. `caldera`: A volcano whose top fell in: a wide, flat-floored crater ringed by cliffs: Crater Lake. `cinder`: A small, straight-sided cone with a deep crater. |
| `base` | number | 70 | -2048 to 4096 blocks | The height of the ground around it, in blocks (0 to add it to another relief). Usually -64 to 320. |
| `crater` | number | 0.1 | 0 to 0.6 | How wide the crater is, as a share of the radius (a caldera's is at least 0.3). |
| `crater_depth` | number | 35 | 0 to 1024 blocks | How far below the rim the crater's floor lies, in blocks. Usually 0 to 200. |
| `gullies` | number | 0.5 | 0 to 1 | How deeply channels cut the flanks. |
| `gully_count` | integer | 14 | 3 to 96 | How many channels run down each volcano. |
| `cones` | integer | 3 | 0 to 16 | How many smaller cones stand on each volcano's flanks. |
| `roughness` | number | 0.4 | 0 to 1 | How much small relief it carries. |
| `seed` | seed | — | 0 to 4294967295 | Another seed gives another variation of the same settings. Left out, it comes from the node's name. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "out": { "type": "output.map", "inputs": { "height": "volcano" } },
    "volcano": { "type": "mountain.volcano", "params": { "volcanoes": [{ "x": 0.35, "z": 0.4 }, { "x": 0.65, "z": 0.6 }] } }
  }
}
```

### Mesas and buttes — `plateau.mesa`

Flat-topped tablelands cut by cliffs: mesas, buttes standing alone, stepped walls of rock layers, talus at their feet. Coverage is the share of the ground left high, Scale the size of the mesas: small ones stand alone as buttes. Cliff is how sheer their walls are, Layers how many steps the rock beds make in them, Talus how far the scree spreads at their feet. Dissection cuts canyons into the tablelands. Connect a mask to Area to say where they rise.

Inputs:

- `area` (mask, optional) — Where the tablelands rise, from 0 to 1 (Coverage spreads them when unconnected).

Outputs:

- `height` (height) — The tablelands over Base.
- `mask` (mask) — 1 on the landform, 0 around it.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `height` | number | 60 | 0 to 2048 blocks | How high the tops stand over Base, in blocks. Usually 0 to 200. A field connected to the input `@height` drives it place by place. |
| `base` | number | 70 | -2048 to 4096 blocks | The height of the ground around it, in blocks (0 to add it to another relief). Usually -64 to 320. |
| `scale` | number | 900 | 32 to 32000 blocks | The size of the mesas, in blocks. Usually 100 to 4000. |
| `coverage` | number | 0.45 | 0 to 1 | The share of the ground left high. |
| `cliff` | number | 0.85 | 0 to 1 | How sheer the walls are. |
| `layers` | integer | 3 | 0 to 16 | How many steps the rock beds make in the walls. |
| `talus` | number | 0.5 | 0 to 1 | How far scree spreads at the feet of the walls. |
| `dissection` | number | 0.35 | 0 to 1 | How much canyons cut into the tablelands. |
| `roughness` | number | 0.4 | 0 to 1 | How much small relief it carries. |
| `seed` | seed | — | 0 to 4294967295 | Another seed gives another variation of the same settings. Left out, it comes from the node's name. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "mesa": { "type": "plateau.mesa" },
    "out": { "type": "output.map", "inputs": { "height": "mesa" } }
  }
}
```

### Canyon — `valley.canyon`

A canyon cut into the relief: along a line drawn on the terrain, or a network of them winding across it, with a flat floor and stepped walls. Draw the Path on the viewer for one canyon (each point can make it deeper or wider there), or leave it empty for a network of canyons, Scale apart, winding across the terrain with side canyons (Branches). Width is the canyon's width at the rim, Floor the share of it that is flat bottom, Steps how many ledges the rock beds make in its walls. Feed it a plateau (a Mesas node, a Constant) through Height.

Inputs:

- `height` (height, optional) — The relief to cut into (a ground at Base when unconnected).

Outputs:

- `height` (height) — The relief with the canyon cut into it.
- `mask` (mask) — 1 in the canyon's floor, fading up its walls.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `path` | curve | none | each `x`, `z`, `depth`, `width` | The canyon's course, drawn on the viewer; a network when not drawn. |
| `depth` | number | 55 | 0 to 2048 blocks | How deep the canyon is, in blocks. Usually 0 to 200. A field connected to the input `@depth` drives it place by place. |
| `width` | number | 260 | 4 to 16000 blocks | How wide the canyon is at its rim, in blocks. Usually 10 to 2000. |
| `floor` | number | 0.22 | 0 to 0.95 | The share of the width that is flat floor. |
| `steps` | integer | 3 | 0 to 16 | How many ledges the walls make. |
| `meander` | number | 0.4 | 0 to 1 | How much the canyon winds. |
| `scale` | number | 1600 | 64 to 32000 blocks | How far apart the canyons of a network are, in blocks. Usually 200 to 8000. |
| `branches` | number | 0.4 | 0 to 1 | How many side canyons a network has. |
| `base` | number | 140 | -2048 to 4096 blocks | The height of the ground cut into when Height is unconnected, in blocks. Usually -64 to 320. |
| `seed` | seed | — | 0 to 4294967295 | Another seed gives another variation of the same settings. Left out, it comes from the node's name. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "canyon": { "type": "valley.canyon", "params": { "path": [{ "x": 0.2, "z": 0.55 }, { "x": 0.5, "z": 0.45 }, { "x": 0.8, "z": 0.5 }] } },
    "out": { "type": "output.map", "inputs": { "height": "canyon" } }
  }
}
```

### Glacial valley — `valley.glacial`

A U-shaped valley a glacier carved, along a line drawn on the terrain: a wide flat floor gently descending between steep walls. Draw the Path on the viewer (each point can make it wider there). Floor height is the height of its flat floor where the path starts, Drop how much lower it lies at the end; Width is the valley's width from rim to rim, Floor the share of it that is flat, Steepness how sheer its walls are. Cut it into mountains through Height for a valley like Yosemite or Lauterbrunnen; a floor under the sea level makes a fjord.

Inputs:

- `height` (height, optional) — The relief to cut into (a ground at Base when unconnected).

Outputs:

- `height` (height) — The relief with the valley carved.
- `mask` (mask) — 1 on the valley's floor, fading up its walls.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `path` | curve | none | each `x`, `z`, `width` | The valley's course, drawn on the viewer. |
| `floor_height` | number | 75 | -2048 to 4096 blocks | The height of the valley's flat floor where the path starts, in blocks. Usually -64 to 320. A field connected to the input `@floor_height` drives it place by place. |
| `drop` | number | 20 | -2048 to 2048 blocks | How much lower the floor lies at the path's end than at its start, in blocks. Usually -50 to 150. |
| `width` | number | 700 | 16 to 16000 blocks | How wide the valley is from rim to rim, in blocks. Usually 50 to 4000. |
| `floor` | number | 0.35 | 0 to 0.95 | The share of the width that is flat floor. |
| `steepness` | number | 0.75 | 0 to 1 | How sheer the walls are. |
| `meander` | number | 0.2 | 0 to 1 | How much the valley winds. |
| `base` | number | 140 | -2048 to 4096 blocks | The height of the ground cut into when Height is unconnected, in blocks. Usually -64 to 320. |
| `seed` | seed | — | 0 to 4294967295 | Another seed gives another variation of the same settings. Left out, it comes from the node's name. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "glacial": { "type": "valley.glacial", "params": { "path": [{ "x": 0.2, "z": 0.55 }, { "x": 0.5, "z": 0.45 }, { "x": 0.8, "z": 0.5 }] } },
    "out": { "type": "output.map", "inputs": { "height": "glacial" } }
  }
}
```

### Fault — `fault.scarp`

A fault along a line drawn on the terrain: the ground on one side lifted, a scarp between the two. Draw the fault's line on the viewer. Throw is how much higher the left side (of the way the line runs) stands; a negative throw lifts the right side. Width is the scarp's width, Ragged how broken its face is. Feed it the relief to break through Height.

Inputs:

- `height` (height, optional) — The relief to cut into (a ground at Base when unconnected).

Outputs:

- `height` (height) — The relief with the fault.
- `mask` (mask) — 1 on the scarp's face.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `path` | curve | none | each `x`, `z` | The fault's line, drawn on the viewer. |
| `throw` | number | 30 | -1024 to 1024 blocks | How much higher the left side stands, in blocks (negative: the right side). Usually -100 to 100. A field connected to the input `@throw` drives it place by place. |
| `width` | number | 40 | 1 to 4000 blocks | How wide the scarp is, in blocks. Usually 2 to 300. |
| `ragged` | number | 0.4 | 0 to 1 | How broken the scarp's face and line are. |
| `base` | number | 140 | -2048 to 4096 blocks | The height of the ground cut into when Height is unconnected, in blocks. Usually -64 to 320. |
| `seed` | seed | — | 0 to 4294967295 | Another seed gives another variation of the same settings. Left out, it comes from the node's name. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "out": { "type": "output.map", "inputs": { "height": "scarp" } },
    "scarp": { "type": "fault.scarp", "params": { "path": [{ "x": 0.2, "z": 0.55 }, { "x": 0.5, "z": 0.45 }, { "x": 0.8, "z": 0.5 }] } }
  }
}
```

### Dunes — `land.dunes`

Sand shaped by the wind: transverse crests, linear ridges, crescent barchans or star dunes, with ripples over them. Direction is where the wind blows to (0 east, 90 south); Wavelength how far apart the crests are; Height how tall they stand. Irregularity breaks and bends the crests. Connect a mask to Sand to say where there is sand (the dunes fade where there is less). Ripples add the wind's small waves where the samples are fine enough to hold them.

Inputs:

- `sand` (mask, optional) — Where there is sand, from 0 to 1 (everywhere when unconnected).

Outputs:

- `height` (height) — The dunes over Base.
- `mask` (mask) — 1 on the landform, 0 around it.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `kind` | choice | `transverse` | `transverse`, `linear`, `barchan`, `star` | The kind of dunes. `transverse`: Long crests across the wind, a gentle slope up and a steep one down: sand seas. `linear`: Long ridges along the wind (seifs), sharp crests between corridors. `barchan`: Crescents scattered on hard ground, their horns pointing downwind. `star`: Pyramids of sand with arms in three directions, where the wind turns. |
| `direction` | number | 20 | -180 to 180 degrees | Where the wind blows to, in degrees from east (90: south). |
| `wavelength` | number | 140 | 8 to 8000 blocks | How far apart the crests are, in blocks. Usually 20 to 800. |
| `height` | number | 14 | 0 to 512 blocks | How tall the dunes stand, in blocks. Usually 0 to 60. A field connected to the input `@height` drives it place by place. |
| `base` | number | 70 | -2048 to 4096 blocks | The height of the ground around it, in blocks (0 to add it to another relief). Usually -64 to 320. |
| `irregularity` | number | 0.5 | 0 to 1 | How much the crests bend, break and vary. |
| `ripples` | number | 0.3 | 0 to 1 | How strong the wind's small waves are. |
| `seed` | seed | — | 0 to 4294967295 | Another seed gives another variation of the same settings. Left out, it comes from the node's name. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "dunes": { "type": "land.dunes" },
    "out": { "type": "output.map", "inputs": { "height": "dunes" } }
  }
}
```

### Karst — `land.karst`

Limestone dissolved by water: steep towers on a plain, crowded cones around hollows, or a plain pitted with sinkholes. Spacing is how far apart the towers, cones or sinkholes are, Height how tall the towers (or deep the sinkholes) are. Steepness makes the towers' walls sheer. Coverage is the share of places that hold one. Put it over a coastline for a drowned karst like Ha Long Bay.

Outputs:

- `height` (height) — The karst over Base.
- `mask` (mask) — 1 on the landform, 0 around it.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `kind` | choice | `towers` | `towers`, `cockpit`, `sinkholes` | The kind of karst. `towers`: Steep pillars of limestone rising from a plain: Guilin, Ha Long Bay. `cockpit`: Crowded conical hills around star-shaped hollows: Jamaica's Cockpit Country. `sinkholes`: A plain pitted with round hollows where the rock dissolved beneath. |
| `spacing` | number | 160 | 8 to 8000 blocks | How far apart the towers, cones or sinkholes are, in blocks. Usually 30 to 1000. |
| `height` | number | 70 | 0 to 1024 blocks | How tall the towers are (how deep the sinkholes), in blocks. Usually 0 to 200. A field connected to the input `@height` drives it place by place. |
| `base` | number | 70 | -2048 to 4096 blocks | The height of the ground around it, in blocks (0 to add it to another relief). Usually -64 to 320. |
| `steepness` | number | 0.85 | 0 to 1 | How sheer the towers' walls are. |
| `coverage` | number | 0.6 | 0 to 1 | The share of places holding a tower, a cone or a sinkhole. |
| `roughness` | number | 0.4 | 0 to 1 | How much small relief it carries. |
| `seed` | seed | — | 0 to 4294967295 | Another seed gives another variation of the same settings. Left out, it comes from the node's name. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "karst": { "type": "land.karst" },
    "out": { "type": "output.map", "inputs": { "height": "karst" } }
  }
}
```

### Craters — `crater.field`

Impact craters: a bowl with a raised rim and its ejecta, a central peak in the large ones; placed by hand or scattered, many small and few large. Place craters on the viewer (each with its radius and depth) or let Count scatter them, from Smallest to Largest. Connect a relief to Height to dig them into it; otherwise they lie on a ground at Base. Large craters (over Peak size) get a central peak and a flat floor.

Inputs:

- `height` (height, optional) — The relief to dig them into (a ground at Base when unconnected).

Outputs:

- `height` (height) — The relief with its craters.
- `mask` (mask) — 1 on the landform, 0 around it.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `craters` | points | none | up to 64; each `x`, `z`, `radius`, `depth` | Craters placed by hand, each with its radius and depth. |
| `count` | integer | 30 | 0 to 2000 | How many craters are scattered. |
| `smallest` | number | 20 | 4 to 8000 blocks | The radius of the smallest scattered craters, in blocks. Usually 4 to 400. |
| `largest` | number | 400 | 4 to 16000 blocks | The radius of the largest scattered craters, in blocks. Usually 20 to 3000. |
| `depth_ratio` | number | 0.18 | 0 to 1 | How deep the scattered craters are, as a share of their radius. |
| `rim` | number | 0.35 | 0 to 1 | How high the rim stands, as a share of the depth. |
| `peak_size` | number | 250 | 8 to 16000 blocks | Craters wider than this get a central peak and a flat floor, in blocks. |
| `base` | number | 70 | -2048 to 4096 blocks | The height of the ground around it, in blocks (0 to add it to another relief). Usually -64 to 320. |
| `seed` | seed | — | 0 to 4294967295 | Another seed gives another variation of the same settings. Left out, it comes from the node's name. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "field": { "type": "crater.field", "params": { "craters": [{ "x": 0.35, "z": 0.4 }, { "x": 0.65, "z": 0.6 }] } },
    "out": { "type": "output.map", "inputs": { "height": "field" } }
  }
}
```

### Island — `coast.island`

An island rising from the sea: a ragged coast, a beach, a shallow shelf around it, land rising inland. Place its centre on the viewer. Radius is its size, Height how high its middle rises over the sea, Coast roughness how ragged the shore is. Shelf and Depth shape the sea around it. Put mountains on it with Max (or Add with their Base at 0) through its Land mask.

Outputs:

- `height` (height) — The island and the sea floor around it.
- `land` (mask) — 1 on land, 0 at sea, soft across the shore.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `center` | points | none | up to 1; each `x`, `z` | Where the island's middle is, placed on the viewer. |
| `radius` | number | 1400 | 16 to 32000 blocks | How far the island reaches from its middle, in blocks. Usually 100 to 8000. |
| `height` | number | 70 | 0 to 4096 blocks | How high the island's middle rises over the sea, in blocks. Usually 0 to 600. A field connected to the input `@height` drives it place by place. |
| `elongation` | number | 0.3 | 0 to 0.9 | How much longer than wide the island is. |
| `angle` | number | 30 | -180 to 180 degrees | Which way its length runs, in degrees from east. |
| `profile` | profile | `[[0.0,0.0],[0.35,0.18],[0.75,0.7],[1.0,1.0]]` |  | The shape of the land from the shore (left) to the middle (right). |
| `beach` | number | 40 | 0 to 2000 blocks | How wide the low, flat shore is, in blocks. Usually 0 to 300. |
| `shelf` | number | 400 | 0 to 16000 blocks | How far the shallow sea reaches from the shore, in blocks. Usually 0 to 3000. |
| `depth` | number | 40 | 0 to 1024 blocks | How deep the sea is at the shelf's edge, in blocks (it keeps sinking beyond). Usually 0 to 200. |
| `roughness` | number | 0.5 | 0 to 1 | How ragged the coastline is: capes and bays, small and large. |
| `outline_scale` | number | 600 | 16 to 16000 blocks | The size of the largest capes and bays, in blocks. Usually 50 to 4000. |
| `hills` | number | 0.35 | 0 to 1 | How much rolling relief the land carries. |
| `seed` | seed | — | 0 to 4294967295 | Another seed gives another variation of the same settings. Left out, it comes from the node's name. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "island": { "type": "coast.island", "params": { "center": [{ "x": 0.35, "z": 0.4 }, { "x": 0.65, "z": 0.6 }] } },
    "out": { "type": "output.map", "inputs": { "height": "island" } }
  }
}
```

### Archipelago — `coast.archipelago`

A scatter of islands in the sea, many small and a few large, over the terrain or along a line. Count islands are scattered over the terrain, or along the Chain when one is drawn (an arc of islands), their sizes between Smallest and Largest (most of them small). Place islands by hand as Islands to set each one yourself; they come in addition. Shelf and Depth shape the sea around them, so that close islands share their shallows.

Outputs:

- `height` (height) — The islands and the sea floor around them.
- `land` (mask) — 1 on land, 0 at sea, soft across the shore.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `count` | integer | 14 | 0 to 400 | How many islands are scattered. |
| `smallest` | number | 70 | 16 to 16000 blocks | The radius of the smallest islands, in blocks. Usually 32 to 2000. |
| `largest` | number | 700 | 16 to 32000 blocks | The radius of the largest islands, in blocks. Usually 100 to 6000. |
| `height` | number | 60 | 0 to 4096 blocks | How high the largest islands rise over the sea, in blocks (smaller ones less). Usually 0 to 600. A field connected to the input `@height` drives it place by place. |
| `chain` | curve | none | each `x`, `z` | A line the islands gather along (an arc of islands); over the whole terrain when not drawn. |
| `spread` | number | 1500 | 0 to 32000 blocks | How far islands stray from the Chain, in blocks. Usually 0 to 6000. |
| `islands` | points | none | up to 128; each `x`, `z`, `radius`, `height` | Islands placed by hand, in addition to the scattered ones. |
| `profile` | profile | `[[0.0,0.0],[0.35,0.18],[0.75,0.7],[1.0,1.0]]` |  | The shape of the land from the shore (left) to the middle (right). |
| `beach` | number | 40 | 0 to 2000 blocks | How wide the low, flat shore is, in blocks. Usually 0 to 300. |
| `shelf` | number | 400 | 0 to 16000 blocks | How far the shallow sea reaches from the shore, in blocks. Usually 0 to 3000. |
| `depth` | number | 40 | 0 to 1024 blocks | How deep the sea is at the shelf's edge, in blocks (it keeps sinking beyond). Usually 0 to 200. |
| `roughness` | number | 0.5 | 0 to 1 | How ragged the coastline is: capes and bays, small and large. |
| `hills` | number | 0.35 | 0 to 1 | How much rolling relief the land carries. |
| `seed` | seed | — | 0 to 4294967295 | Another seed gives another variation of the same settings. Left out, it comes from the node's name. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "archipelago": { "type": "coast.archipelago", "params": { "chain": [{ "x": 0.2, "z": 0.55 }, { "x": 0.5, "z": 0.45 }, { "x": 0.8, "z": 0.5 }], "islands": [{ "x": 0.35, "z": 0.4 }, { "x": 0.65, "z": 0.6 }] } },
    "out": { "type": "output.map", "inputs": { "height": "archipelago" } }
  }
}
```

### Coastline — `coast.coastline`

Land and sea across the terrain: continents and seas, a ragged coast with capes and bays, beaches, a shelf, fjords if wanted. Land is the share of the terrain above the sea, Scale the size of the landmasses. Draw an Outline to say where the coast runs (a closed line: land inside), or connect a mask to Shape (an imported or painted land map); otherwise noise decides. Inland is how far the land takes to rise to Height. Fjords cut long, deep inlets into the coast. The Land output is the mask to put mountains, forests and beaches through.

Inputs:

- `shape` (mask, optional) — Where the land is, from 0 to 1 (noise or the Outline decide, when unconnected).

Outputs:

- `height` (height) — The land and the sea floor.
- `land` (mask) — 1 on land, 0 at sea, soft across the shore.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `land` | number | 0.5 | 0 to 1 | The share of the terrain that is land, when noise decides. |
| `scale` | number | 5000 | 64 to 65536 blocks | The size of the landmasses, in blocks. Usually 500 to 20000. |
| `height` | number | 80 | 0 to 4096 blocks | How high the land rises inland, in blocks over the sea. Usually 0 to 600. A field connected to the input `@height` drives it place by place. |
| `inland` | number | 1500 | 16 to 32000 blocks | How far from the shore the land reaches its height, in blocks. Usually 100 to 8000. |
| `outline` | area | none | each `x`, `z` | The coast, drawn on the viewer as a closed line: land inside. |
| `profile` | profile | `[[0.0,0.0],[0.35,0.18],[0.75,0.7],[1.0,1.0]]` |  | The shape of the land from the shore (left) to Inland (right). |
| `beach` | number | 40 | 0 to 2000 blocks | How wide the low, flat shore is, in blocks. Usually 0 to 300. |
| `shelf` | number | 400 | 0 to 16000 blocks | How far the shallow sea reaches from the shore, in blocks. Usually 0 to 3000. |
| `depth` | number | 40 | 0 to 1024 blocks | How deep the sea is at the shelf's edge, in blocks (it keeps sinking beyond). Usually 0 to 200. |
| `fjords` | number | 0 | 0 to 1 | How many long, deep inlets are cut into the coast. |
| `fjord_size` | number | 350 | 16 to 8000 blocks | How wide apart the fjords run, in blocks. Usually 50 to 2000. |
| `roughness` | number | 0.5 | 0 to 1 | How ragged the coastline is: capes and bays, small and large. |
| `hills` | number | 0.35 | 0 to 1 | How much rolling relief the land carries. |
| `seed` | seed | — | 0 to 4294967295 | Another seed gives another variation of the same settings. Left out, it comes from the node's name. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "coastline": { "type": "coast.coastline", "params": { "outline": [{ "x": 0.5, "z": 0.25 }, { "x": 0.74, "z": 0.42 }, { "x": 0.65, "z": 0.7 }, { "x": 0.35, "z": 0.7 }, { "x": 0.26, "z": 0.42 }] } },
    "out": { "type": "output.map", "inputs": { "height": "coastline" } }
  }
}
```

### Floating islands (1.2) — `legacy.floating_islands`

Islands of rock floating in the air: a rolling top on sheer sides, over an underside hanging down to a point. Altitude is the height of their middles and Spread how much it varies; Size the radius of the largest; Coverage how many of the places one could float have one; Depth how far the underside hangs, as a multiple of the radius. Top and Bottom feed a layer of the Map Output.

A simulation: it works over the whole terrain at the simulations' resolution.

Outputs:

- `top` (height) — The islands' top (the bottom where there is none).
- `bottom` (height) — Their underside.
- `islands` (mask) — Where they are.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `altitude` | number | 230 | -64 to 4096 blocks | The height of their middles. In blocks. |
| `spread` | number | 25 | 0 to 1000 blocks | How much their altitude varies. In blocks. |
| `size` | number | 70 | 4 to 4000 blocks | The radius of the largest. In blocks. |
| `coverage` | number | 0.5 | 0 to 1 | How many of the places one could float have one. |
| `depth` | number | 1.3 | 0 to 10 | How far the underside hangs, as a multiple of the radius. |
| `seed` | integer | 11 | 0 to 4294967295 | Another seed places them otherwise. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "floating_islands": { "type": "legacy.floating_islands" },
    "out": { "type": "output.map", "inputs": { "height": "floating_islands" } }
  }
}
```

### Tectonic plates (1.2) — `legacy.tectonic`

Relief from tectonic plates: continents and oceans, mountains where plates collide, rifts where they pull apart. Plates are 1.2's plates, as JSON. Relief is the height of the highest ranges, Width how wide the belts along the boundaries are, Roughness how much the boundaries wind, Sea the sea level the oceans lie under.

A simulation: it works over the whole terrain at the simulations' resolution.

Outputs:

- `height` (height) — The relief.
- `ranges` (mask) — Where plates collide.
- `faults` (mask) — Where they pull apart.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `plates` | text | `[]` |  | The plates, as 1.2 wrote them (JSON). |
| `relief` | number | 170 | 0 to 4096 blocks | The height of the highest ranges. In blocks. |
| `width` | number | 260 | 1 to 20000 blocks | How wide the belts along the boundaries are. In blocks. |
| `roughness` | number | 0.6 | 0 to 1 | How much the boundaries wind. |
| `sea` | number | 63 | -64 to 320 blocks | The sea level the oceans lie under. In blocks. |
| `seed` | integer | 7 | 0 to 4294967295 | Another seed varies the boundaries. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "out": { "type": "output.map", "inputs": { "height": "tectonic" } },
    "tectonic": { "type": "legacy.tectonic" }
  }
}
```

## Adjust

### Levels — `adjust.levels`

Maps one range of heights onto another, along a curve. What lies between In low and In high is spread between Out low and Out high; what lies outside is held at the ends. Gamma bends the way between them: above 1 lifts the low ground (rounded tops, wide valleys), below 1 flattens it (plains below sharp peaks). Works on masks too, with values from 0 to 1.

Inputs:

- `in` (height) — The relief (or mask) to adjust.

Outputs:

- `out` (height) — The relief adjusted.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `in_low` | number | 0 | -4096 to 4096 blocks | The height taken as the bottom of the range. In blocks. Usually -64 to 512. |
| `in_high` | number | 256 | -4096 to 4096 blocks | The height taken as the top of the range. In blocks. Usually -64 to 512. |
| `out_low` | number | 0 | -4096 to 4096 blocks | Where the bottom of the range goes. In blocks. Usually -64 to 512. |
| `out_high` | number | 256 | -4096 to 4096 blocks | Where the top of the range goes. In blocks. Usually -64 to 512. |
| `gamma` | number | 1 | 0.05 to 20 | Above 1 lifts the middle of the range, below 1 lowers it. Usually 0.2 to 5. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "levels": { "type": "adjust.levels", "inputs": { "in": "relief" } },
    "out": { "type": "output.map", "inputs": { "height": "levels" } },
    "relief": { "type": "noise.fbm", "params": { "height": 250, "scale": 1500 } }
  }
}
```

### Curve — `adjust.curve`

Reshapes a relief along a curve drawn from its low to its high values: flatten the valleys, sharpen the peaks, lift a plateau. The values from In low to In high go through the Curve (left: In low, right: In high; bottom: Out low, top: Out high). A curve sagging under the diagonal flattens the low ground and steepens the summits; one bulging over it rounds the tops and lifts the plains. Works on masks too, with values from 0 to 1.

Inputs:

- `in` (height) — The relief (or mask) to adjust.

Outputs:

- `out` (height) — The relief adjusted.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `curve` | profile | `[[0.0,0.0],[0.5,0.35],[1.0,1.0]]` |  | How the values are reshaped, from In low (left) to In high (right). |
| `in_low` | number | 62 | -4096 to 4096 blocks | The value at the curve's left. In blocks. Usually -64 to 320. |
| `in_high` | number | 300 | -4096 to 4096 blocks | The value at the curve's right. In blocks. Usually -64 to 320. |
| `out_low` | number | 62 | -4096 to 4096 blocks | What the curve's bottom stands for. In blocks. Usually -64 to 320. |
| `out_high` | number | 300 | -4096 to 4096 blocks | What the curve's top stands for. In blocks. Usually -64 to 320. |
| `range` | choice | `given` | `given`, `own` | Whether the curve goes from In low to In high, or over the input's own range (its ends then stand for the input's lowest and highest points). `given`: Between the heights set. `own`: From the input's lowest point to its highest, measured over the whole terrain. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "curve": { "type": "adjust.curve", "inputs": { "in": "relief" } },
    "out": { "type": "output.map", "inputs": { "height": "curve" } },
    "relief": { "type": "noise.fbm", "params": { "height": 250, "scale": 1500 } }
  }
}
```

### Terraces — `adjust.terrace`

Steps cut into a relief between two heights: flat ledges joined by risers, as rock beds or rice terraces make them. Steps is how many ledges there are between Low and High; Sharpness how abrupt the risers are (0 a gentle ramp, 1 a wall). Outside Low and High the relief is left as it is.

Inputs:

- `in` (height) — The relief (or mask) to adjust.

Outputs:

- `out` (height) — The relief adjusted.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `steps` | integer | 8 | 1 to 256 | How many ledges between Low and High. |
| `sharpness` | number | 0.7 | 0 to 1 | How abrupt the risers are. |
| `low` | number | 62 | -4096 to 4096 blocks | Where the steps begin, in blocks. Usually -64 to 320. |
| `high` | number | 300 | -4096 to 4096 blocks | Where they end, in blocks. Usually -64 to 320. |
| `strength` | number | 1 | 0 to 1 | How much the steps replace the slope: 0 leaves the relief as it is. |
| `range` | choice | `given` | `given`, `own` | Whether the steps go from Low to High, or from the input's lowest point to its highest. `given`: Between the heights set. `own`: From the input's lowest point to its highest, measured over the whole terrain. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "out": { "type": "output.map", "inputs": { "height": "terrace" } },
    "relief": { "type": "noise.fbm", "params": { "height": 250, "scale": 1500 } },
    "terrace": { "type": "adjust.terrace", "inputs": { "in": "relief" } }
  }
}
```

### Strata — `adjust.stratify`

Layers of rock of uneven thickness cut into a relief: ledges on the slopes, as sedimentary rock weathers. Layers is how many beds the relief's range is cut into, of thicknesses that vary; Strength how flat their ledges are; Tilt leans the beds across the terrain, from west to east. Works over the input's own range.

Inputs:

- `in` (height) — The relief (or mask) to adjust.

Outputs:

- `out` (height) — The relief adjusted.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `layers` | integer | 14 | 1 to 200 | How many beds the range is cut into. |
| `strength` | number | 0.7 | 0 to 1 | How flat their ledges are. |
| `tilt` | number | 6 | -60 to 60 degrees | How much the beds lean, from west to east. In degrees. |
| `seed` | seed | — | 0 to 4294967295 | Another seed gives another variation of the same settings. Left out, it comes from the node's name. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "out": { "type": "output.map", "inputs": { "height": "stratify" } },
    "relief": { "type": "noise.fbm", "params": { "height": 250, "scale": 1500 } },
    "stratify": { "type": "adjust.stratify", "inputs": { "in": "relief" } }
  }
}
```

### Clamp — `adjust.clamp`

A relief held between two heights: nothing under Min, nothing over Max. Cuts the tops off at Max (flat plateaus, a mesa's cap) and fills the bottoms up to Min (a sea floor, a valley's flat).

Inputs:

- `in` (height) — The relief (or mask) to adjust.

Outputs:

- `out` (height) — The relief adjusted.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `min` | number | -64 | -4096 to 4096 blocks | The lowest the relief goes, in blocks. Usually -64 to 320. |
| `max` | number | 320 | -4096 to 4096 blocks | The highest it goes, in blocks. Usually -64 to 320. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "clamp": { "type": "adjust.clamp", "inputs": { "in": "relief" } },
    "out": { "type": "output.map", "inputs": { "height": "clamp" } },
    "relief": { "type": "noise.fbm", "params": { "height": 250, "scale": 1500 } }
  }
}
```

### Fold — `adjust.fold`

Mirrors what rises over a level back down: ridges folded into crests and troughs. Everything above Level is reflected under it, as if the relief were folded there: a smooth hill becomes a ring of ridges, noise becomes sharp crests. Strength mixes the fold with the relief as it was.

Inputs:

- `in` (height) — The relief (or mask) to adjust.

Outputs:

- `out` (height) — The relief adjusted.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `level` | number | 120 | -4096 to 4096 blocks | Where the relief is folded, in blocks. Usually -64 to 320. |
| `strength` | number | 1 | 0 to 1 | How much of the fold is taken. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "fold": { "type": "adjust.fold", "inputs": { "in": "relief" } },
    "out": { "type": "output.map", "inputs": { "height": "fold" } },
    "relief": { "type": "noise.fbm", "params": { "height": 250, "scale": 1500 } }
  }
}
```

### Invert — `adjust.invert`

Turns a relief upside down within its own range: its highest point where its lowest was. Valleys become ridges and peaks become pits, between the same lowest and highest heights, measured over the whole terrain. For a mask, Invert in Masks is 1 − the mask.

Inputs:

- `in` (height) — The relief (or mask) to adjust.

Outputs:

- `out` (height) — The relief adjusted.

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "invert": { "type": "adjust.invert", "inputs": { "in": "relief" } },
    "out": { "type": "output.map", "inputs": { "height": "invert" } },
    "relief": { "type": "noise.fbm", "params": { "height": 250, "scale": 1500 } }
  }
}
```

### Normalize — `adjust.normalize`

Stretches a relief so that its lowest point lies at Low and its highest at High. Measures the whole relief, then maps its lowest and highest points to Low and High: a relief of any range becomes one of known heights, or a mask from 0 to 1. Heights are measured at the simulation's resolution, so the finest peaks may touch the ends a little early; they are held there.

Inputs:

- `in` (height) — The relief (or mask) to adjust.

Outputs:

- `out` (height) — The relief adjusted.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `low` | number | 0 | -4096 to 4096 blocks | Where the lowest point goes (0 for a mask). In blocks. Usually -64 to 512. |
| `high` | number | 1 | -4096 to 4096 blocks | Where the highest point goes (1 for a mask). In blocks. Usually -64 to 512. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "normalize": { "type": "adjust.normalize", "inputs": { "in": "relief" } },
    "out": { "type": "output.map", "inputs": { "height": "normalize" } },
    "relief": { "type": "noise.fbm", "params": { "height": 250, "scale": 1500 } }
  }
}
```

### Blur — `adjust.blur`

Smooths a relief: the detail smaller than Radius goes, the larger shapes stay. A Gaussian blur Radius blocks wide. Wide blurs are made at a coarser level of detail and brought back sharp, so that a blur of a thousand blocks costs as little as one of ten. Use it to soften noise before using it as a mask, to round off terraces, or to make broad uplifts from detailed shapes.

Inputs:

- `in` (height) — The relief (or mask) to adjust.

Outputs:

- `out` (height) — The relief adjusted.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `radius` | number | 32 | 0 to 8192 blocks | How far the smoothing reaches, in blocks. Usually 1 to 512. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "blur": { "type": "adjust.blur", "inputs": { "in": "relief" } },
    "out": { "type": "output.map", "inputs": { "height": "blur" } },
    "relief": { "type": "noise.fbm", "params": { "height": 250, "scale": 1500 } }
  }
}
```

### Sharpen — `adjust.sharpen`

Brings out a relief's detail: what stands above its surroundings rises, what lies below sinks. The relief minus its blur over Radius, times Amount, is added back: crests and gullies about Radius wide stand out more. Large amounts make a relief look carved.

Inputs:

- `in` (height) — The relief (or mask) to adjust.

Outputs:

- `out` (height) — The relief adjusted.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `radius` | number | 24 | 1 to 1024 blocks | The size of the detail brought out, in blocks. Usually 2 to 256. |
| `amount` | number | 0.8 | 0 to 8 | How much it is brought out. Usually 0 to 3. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "out": { "type": "output.map", "inputs": { "height": "sharpen" } },
    "relief": { "type": "noise.fbm", "params": { "height": 250, "scale": 1500 } },
    "sharpen": { "type": "adjust.sharpen", "inputs": { "in": "relief" } }
  }
}
```

### Warp — `adjust.warp`

Pushes a relief around by a noise: straight lines wander, round shapes twist, everything looks less made. Each place takes the height found up to Amount blocks away, in a direction that changes smoothly over Scale. Small amounts take the regularity off shapes; large ones swirl them like rock under pressure.

Inputs:

- `in` (height) — The relief (or mask) to adjust.

Outputs:

- `out` (height) — The relief adjusted.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `amount` | number | 60 | 0 to 4096 blocks | How far the relief is pushed, in blocks. Usually 0 to 600. |
| `scale` | number | 400 | 4 to 65536 blocks | The size of the pushes, in blocks. Usually 32 to 4000. |
| `octaves` | integer | 3 | 1 to 8 | How much finer turns the pushes have. |
| `seed` | seed | — | 0 to 4294967295 | Another seed gives another variation of the same settings. Left out, it comes from the node's name. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "out": { "type": "output.map", "inputs": { "height": "warp" } },
    "relief": { "type": "noise.fbm", "params": { "height": 250, "scale": 1500 } },
    "warp": { "type": "adjust.warp", "inputs": { "in": "relief" } }
  }
}
```

### Transform — `adjust.transform`

Scales, turns and moves a relief over the terrain. Scale above 1 enlarges the relief around the terrain's middle; Rotate turns it, in degrees clockwise; Offset moves it, as fractions of the terrain. Worked over the whole terrain at the simulations' resolution.

A simulation: it works over the whole terrain at the simulations' resolution.

Inputs:

- `in` (height) — The relief (or mask) to adjust.

Outputs:

- `out` (height) — The relief adjusted.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `scale` | number | 1 | 0.01 to 100 | How much larger the relief is drawn. Usually 0.25 to 4. |
| `rotate` | number | 0 | -360 to 360 degrees | How far it is turned, clockwise. In degrees. |
| `offset_x` | number | 0 | -1 to 1 fraction of the terrain's side | How far it is moved east, as a fraction of the terrain. |
| `offset_z` | number | 0 | -1 to 1 fraction of the terrain's side | How far it is moved south, as a fraction of the terrain. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "out": { "type": "output.map", "inputs": { "height": "transform" } },
    "relief": { "type": "noise.fbm", "params": { "height": 250, "scale": 1500 } },
    "transform": { "type": "adjust.transform", "inputs": { "in": "relief" } }
  }
}
```

### Swirl — `adjust.swirl`

Twists a relief around a point, most at the middle, not at all at Radius. Angle is how far the middle turns, in degrees; Radius how far the twist reaches, as a fraction of the terrain; X and Z where its middle is. Worked over the whole terrain at the simulations' resolution.

A simulation: it works over the whole terrain at the simulations' resolution.

Inputs:

- `in` (height) — The relief (or mask) to adjust.

Outputs:

- `out` (height) — The relief adjusted.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `angle` | number | 120 | -1080 to 1080 degrees | How far the middle turns. In degrees. |
| `radius` | number | 0.5 | 0.01 to 2 fraction of the terrain's side | How far the twist reaches, as a fraction of the terrain. |
| `x` | number | 0.5 | -1 to 2 fraction of the terrain's side | Where its middle is, west to east. In fraction of the terrain's side. |
| `z` | number | 0.5 | -1 to 2 fraction of the terrain's side | Where its middle is, north to south. In fraction of the terrain's side. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "out": { "type": "output.map", "inputs": { "height": "swirl" } },
    "relief": { "type": "noise.fbm", "params": { "height": 250, "scale": 1500 } },
    "swirl": { "type": "adjust.swirl", "inputs": { "in": "relief" } }
  }
}
```

### Overhang (1.2) — `legacy.overhang`

Rims jutting out from the top of cliffs over the void below: a slab of rock, as its top and bottom. Reach is how far the rims jut out, Thickness how thick they are where they leave the cliff, Drop the smallest drop that makes a cliff, Variation how unevenly they jut out along it. Top and Bottom feed a layer of the Map Output.

A simulation: it works over the whole terrain at the simulations' resolution.

Inputs:

- `in` (height) — The relief it works on.

Outputs:

- `top` (height) — The rims' top (the bottom where there is none).
- `bottom` (height) — Their underside.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `reach` | number | 10 | 0 to 200 blocks | How far the rims jut out. In blocks. |
| `thickness` | number | 8 | 1 to 200 blocks | How thick a rim is where it leaves the cliff. In blocks. |
| `drop` | number | 24 | 1 to 400 blocks | The smallest drop that makes a cliff. In blocks. |
| `variation` | number | 0.6 | 0 to 1 | How unevenly the rims jut out along the cliffs. |
| `seed` | integer | 12 | 0 to 4294967295 | Another seed varies them otherwise. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "out": { "type": "output.map", "inputs": { "height": "overhang" } },
    "overhang": { "type": "legacy.overhang", "inputs": { "in": "relief" } },
    "relief": { "type": "noise.fbm", "params": { "height": 250, "scale": 1500 } }
  }
}
```

## Simulate

### Stream erosion — `erosion.stream`

Rivers cut valleys that branch up into every slope, leaving sharp ridges between them, and lay what they carry down where they slow. The erosion that makes mountains look like mountains: water gathers into streams, the more of it the deeper they cut (stream power), their walls fall back to the talus angle, and the sediment settles at the feet of slopes, on valley floors and at the sea. Use it after shapes and noise to turn lumps into ranges drained by real valleys; follow it with Particle erosion for small gullies and Thermal erosion for scree. Strength is how far the rivers cut back into the land; Hardness keeps some rock standing; Rain gives more water where it falls.

A simulation: it works over the whole terrain at the simulations' resolution.

Inputs:

- `height` (height) — The relief to erode.
- `hardness` (mask, optional) — Rock that resists: 1 wears twenty times slower than 0. Left empty: soft rock everywhere.
- `rainfall` (mask, optional) — How much rain falls where: a rainfall map, a rain shadow. Left empty: the same rain everywhere.
- `uplift` (mask, optional) — Where the land rises while the rivers cut, as much as Rebuild says. Left empty: the relief itself: the higher, the more.

Outputs:

- `height` (height) — The relief eroded.
- `flow` (mask) — Where water runs: 1 in the largest rivers, fading up the smallest rills.
- `wear` (mask) — Where the water wore the ground away, the most where it cut the most: river beds, gullies.
- `deposition` (mask) — Where what was worn away came to rest: fans, valley floors, deltas.
- `sediment` (mask) — How much sediment the water carries at the end: muddy rivers, gravel beds.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `strength` | number | 1 | 0 to 20 | How hard the rivers cut: higher, deeper valleys reaching further up into the land. Usually 0 to 4. |
| `rebuild` | number | 1 | 0 to 1 | How much of the relief rises while the rivers cut, then is brought back to its height: 1 turns any shape into mountains carved by their rivers; 0 only cuts valleys into it, down to the sea and the low ground. |
| `iterations` | integer | 200 | 10 to 2000 | Steps at each level of the simulation: more, finer valleys reaching further, slower. |
| `talus` | number | 55 | 10 to 80 degrees | The steepest the valleys' walls stand: lower, wider valleys and rounder ridges. In degrees. |
| `deposition` | number | 0.5 | 0 to 1 | How readily the water drops what it carries where it slows: fans, filled valleys, deltas. |
| `channel` | number | 24 | 0 to 512 blocks | The side of the smallest area that drains into a stream: under it the ground only creeps and slides. In blocks. |
| `diffusion` | number | 0 | 0 to 1 | How much soil creeps downhill and rounds the land between the streams. |
| `m` | number | 0.5 | 0.2 to 0.8 | How much more large rivers cut than small ones (m of K·A^m·S^n). |
| `n` | number | 1 | 0.5 to 2 | How much more steep streams cut than gentle ones (n of K·A^m·S^n). |
| `seed` | seed | — | 0 to 4294967295 | Another seed gives another variation of the same settings. Left out, it comes from the node's name. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "out": { "type": "output.map", "inputs": { "height": "stream" } },
    "relief": { "type": "noise.fbm", "params": { "height": 250, "scale": 1500 } },
    "stream": { "type": "erosion.stream", "inputs": { "height": "relief" } }
  }
}
```

### Particle erosion — `erosion.particle`

Raindrops run down the slopes, wearing gullies and rills where they speed up and laying soil where they slow down. The fine detail of erosion: each drop of rain runs downhill, picks up soil where it gathers speed and drops it where it slows, in hollows and at the feet of slopes. Use it after Stream erosion, or on its own on hills and cliffs, for gullies, rills, scree fans and soft valley floors. More Rain cuts deeper; Capacity is how much a drop carries; Radius how wide it wears.

A simulation: it works over the whole terrain at the simulations' resolution.

Inputs:

- `height` (height) — The relief to erode.
- `hardness` (mask, optional) — Rock that resists: 1 wears twenty times slower than 0. Left empty: soft rock everywhere.
- `rainfall` (mask, optional) — How much rain falls where: a rainfall map, a rain shadow. Left empty: the same rain everywhere.

Outputs:

- `height` (height) — The relief eroded.
- `flow` (mask) — Where water runs: 1 in the largest rivers, fading up the smallest rills.
- `wear` (mask) — Where the water wore the ground away, the most where it cut the most: river beds, gullies.
- `deposition` (mask) — Where what was worn away came to rest: fans, valley floors, deltas.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `rain` | number | 1 | 0.05 to 16 | Raindrops for each sample of the simulation: more, deeper gullies, slower. Usually 0.1 to 4. |
| `erosion` | number | 0.3 | 0 to 1 | How fast a drop picks up soil where it can carry more. |
| `deposition` | number | 0.3 | 0 to 1 | How fast a drop lays down what it carries beyond what it can. |
| `capacity` | number | 4 | 0 to 32 | How much soil a fast drop on a steep slope can carry. Usually 0.5 to 16. |
| `evaporation` | number | 0.02 | 0 to 0.5 | How much of its water a drop loses at each step: higher, shorter gullies. |
| `radius` | number | 6 | 1 to 64 blocks | How wide a drop wears the ground around it. In blocks. |
| `inertia` | number | 0.05 | 0 to 0.95 | How much a drop keeps its way rather than turning downhill: higher, straighter and longer gullies. |
| `seed` | seed | — | 0 to 4294967295 | Another seed gives another variation of the same settings. Left out, it comes from the node's name. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "out": { "type": "output.map", "inputs": { "height": "particle" } },
    "particle": { "type": "erosion.particle", "inputs": { "height": "relief" } },
    "relief": { "type": "noise.fbm", "params": { "height": 250, "scale": 1500 } }
  }
}
```

### Thermal erosion — `erosion.thermal`

Loose rock slides down slopes steeper than its angle of repose, and gathers at their feet as scree. Wears cliffs back and fills the feet of slopes with talus, the way frost and gravity do: what is steeper than Talus angle sheds material to the lower neighbours until it is stable. More iterations go further towards that angle; Strength is how much moves at each; Hardness keeps cliffs of hard rock standing while soft beds crumble. The Talus output is where material came to rest: scree, gravel, rubble.

A simulation: it works over the whole terrain at the simulations' resolution.

Inputs:

- `height` (height) — The relief to wear.
- `hardness` (mask, optional) — Rock that holds: 1 never crumbles, 0 crumbles as Strength says. Left empty: the same rock everywhere.

Outputs:

- `height` (height) — The relief worn.
- `talus` (mask) — Where material came to rest, by how much (1 for 2 blocks and more).

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `angle` | number | 35 | 5 to 80 degrees | The steepest slope loose material holds on. In degrees. |
| `iterations` | integer | 50 | 1 to 1000 | How many times material slides: more, closer to the talus angle. |
| `strength` | number | 0.5 | 0.01 to 1 | How much of what is too steep moves at each iteration. |
| `strata` | number | 0 | 0 to 128 blocks | The thickness of the beds of rock: hard beds stand as cliffs while the soft ones between crumble back into benches, a staircase of terraces. 0: one rock throughout. In blocks. Usually 0 to 32. |
| `strata_hardness` | number | 0.85 | 0 to 1 | How much the hard beds resist: 1 never crumbles. |
| `strata_share` | number | 0.4 | 0.05 to 0.95 | How much of each pair of beds is hard rock: more, taller cliffs and narrower benches. |
| `strata_warp` | number | 0.3 | 0 to 2 | How far the beds wave up and down across the land, in beds. |
| `seed` | seed | — | 0 to 4294967295 | Another seed gives another variation of the same settings. Left out, it comes from the node's name. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "out": { "type": "output.map", "inputs": { "height": "thermal" } },
    "relief": { "type": "noise.fbm", "params": { "height": 250, "scale": 1500 } },
    "thermal": { "type": "erosion.thermal", "inputs": { "height": "relief" } }
  }
}
```

### Sediment — `sediment.deposit`

Loose rock and soil shed by steep slopes run down with the water and settle where it slows: fans at the feet of mountains, filled valleys and basins, deltas at the sea. Lays down what the mountains shed, without cutting anything: the steep slopes of Source give Amount blocks of material over the run, the water carries it down its ways and drops it where they flatten, spreading alluvial fans at the mouths of valleys, flat floors in the valleys and basins, and deltas where rivers meet the sea. Use it after Stream erosion, or on any range, to soften its feet into the plains.

A simulation: it works over the whole terrain at the simulations' resolution.

Inputs:

- `height` (height) — The relief that sheds and gathers.
- `source` (mask, optional) — Where loose material comes from (still only from its steep slopes). Left empty: every steep slope.

Outputs:

- `height` (height) — The relief with its sediment laid down.
- `deposition` (mask) — Where what was worn away came to rest: fans, valley floors, deltas.
- `wear` (mask) — Where the material came from.
- `sediment` (mask) — How much sediment the water carries at the end: muddy rivers, gravel beds.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `amount` | number | 6 | 0 to 200 blocks | Blocks of material the steepest slopes shed over the run. Usually 0 to 40. |
| `deposition` | number | 0.7 | 0 to 1 | How readily the water drops what it carries: higher, shorter and steeper fans; lower, material carried far down the valleys and into the sea. |
| `iterations` | integer | 150 | 10 to 2000 | Steps at each level of the simulation: more, fans spreading further. |
| `creep` | number | 0.05 | 0 to 1 | How much the loose ground creeps and smooths. |
| `seed` | seed | — | 0 to 4294967295 | Another seed gives another variation of the same settings. Left out, it comes from the node's name. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "deposit": { "type": "sediment.deposit", "inputs": { "height": "relief" } },
    "out": { "type": "output.map", "inputs": { "height": "deposit" } },
    "relief": { "type": "noise.fbm", "params": { "height": 250, "scale": 1500 } }
  }
}
```

### Glaciers — `ice.glacier`

Ice gathers above the snow line, flows down the valleys and scours them into U-shapes, with cirques at their heads, fjords below the sea and moraines at their ends. Snow above the equilibrium line (ELA) turns to ice, which flows downhill and melts below the line. Where it slides it wears its bed: V-shaped river valleys become broad U-shaped troughs, their heads armchair cirques, side valleys left hanging, and valleys the ice carried below the sea become fjords. Moraines are laid along its edges and where it ends. Lower the line or raise Accumulation for larger glaciers; Strength is how deep they cut.

A simulation: it works over the whole terrain at the simulations' resolution.

Inputs:

- `height` (height) — The relief the ice covers.
- `snow` (mask, optional) — Where snow falls (a snow or rainfall map): 0 no ice grows. Left empty: the same snow everywhere.

Outputs:

- `height` (height) — The relief scoured by the ice, its moraines laid.
- `ice` (mask) — Where the ice lies at the end, by how thick (1 for 30 blocks and more).
- `wear` (mask) — Where the water wore the ground away, the most where it cut the most: river beds, gullies.
- `deposition` (mask) — Where the ice laid its moraines, along its edges and where it ends.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `ela` | number | 170 | -64 to 320 blocks | The equilibrium line: above it snow gathers into ice, below it the ice melts. In blocks. Usually 60 to 300. |
| `accumulation` | number | 1 | 0 to 10 | How fast ice gathers above the line, and melts below: higher, thicker glaciers reaching further down. Usually 0 to 4. |
| `strength` | number | 1 | 0 to 10 | How deep the ice scours its bed. Usually 0 to 4. |
| `iterations` | integer | 800 | 10 to 10000 | Steps of the simulation: more, an older ice age, deeper troughs. |
| `moraines` | number | 3 | 0 to 32 blocks | How high the ridges of rubble along the ice's edges stand. In blocks. |
| `flow` | number | 1 | 0 to 10 | How readily the ice flows and slides: higher, thinner and faster glaciers. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "glacier": { "type": "ice.glacier", "inputs": { "height": "relief" } },
    "out": { "type": "output.map", "inputs": { "height": "glacier" } },
    "relief": { "type": "noise.fbm", "params": { "height": 250, "scale": 1500 } }
  }
}
```

### Wind — `wind.erosion`

The wind blows loose sand into dunes, ridged where sand is plenty and crescents where it is scarce, and wears bare rock that faces it. Spreads Sand blocks of loose sand (where the Sand mask says) and lets the wind work it: it lifts sand in the open, carries it a hop downwind, and drops it in the lee of every rise, so that dunes grow with gentle backs to the wind and steep slip faces away from it. Plenty of sand makes transverse ridges; little makes barchans. Abrasion wears rock facing the wind into streamlined ridges (yardangs). Connect a wind vector (Climate's wind) to make the wind turn over the land.

A simulation: it works over the whole terrain at the simulations' resolution.

Inputs:

- `height` (height) — The ground under the sand.
- `sand` (mask, optional) — Where loose sand lies at first. Left empty: sand everywhere.
- `wind` (vector, optional) — The wind's way at each place. Left empty: the Direction setting everywhere.

Outputs:

- `height` (height) — The ground with its dunes.
- `sand` (mask) — How deep the sand lies (1 for 8 blocks and more).
- `wear` (mask) — Where the water wore the ground away, the most where it cut the most: river beds, gullies.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `direction` | number | 0 | 0 to 360 degrees | Where the wind blows towards: 0° east, 90° south. In degrees. |
| `sand_depth` | number | 6 | 0 to 64 blocks | Blocks of loose sand to blow around: plenty, transverse ridges; little, crescents. Usually 0 to 24. |
| `hop` | number | 12 | 1 to 128 blocks | How far a grain flies in one hop: longer, larger dunes further apart. In blocks. Usually 4 to 48. |
| `iterations` | integer | 400 | 10 to 10000 | Steps of the simulation: more, dunes grown larger and more regular. |
| `strength` | number | 1 | 0 to 5 | How much sand the wind lifts at each step. |
| `abrasion` | number | 0 | 0 to 5 | How fast blown sand wears the bare rock that faces the wind. |
| `seed` | seed | — | 0 to 4294967295 | Another seed gives another variation of the same settings. Left out, it comes from the node's name. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "erosion": { "type": "wind.erosion", "inputs": { "height": "relief" } },
    "out": { "type": "output.map", "inputs": { "height": "erosion" } },
    "relief": { "type": "noise.fbm", "params": { "height": 250, "scale": 1500 } }
  }
}
```

### Flow — `water.flow`

Where the rain runs off, as a mask: bright in the rivers, fading up every rill. Follows the rain down the relief, across hollows and flats to the sea or the terrain's edge, and measures how much land drains through each place. Use it to paint river beds, wet valley floors and gullies, or to drive vegetation. Small streams shows more of the rills next to the rivers.

A simulation: it works over the whole terrain at the simulations' resolution.

Inputs:

- `height` (height) — The relief the water runs over.
- `rainfall` (mask, optional) — How much rain falls where: a rainfall map, a rain shadow. Left empty: the same rain everywhere.

Outputs:

- `flow` (mask) — Where water runs: 1 in the largest rivers, fading up the smallest rills.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `spread` | number | 0.5 | 0 to 1 | How much the small streams show next to the large rivers. |
| `seed` | seed | — | 0 to 4294967295 | Another seed gives another variation of the same settings. Left out, it comes from the node's name. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "flow": { "type": "water.flow", "inputs": { "height": "relief" } },
    "out": { "type": "output.map", "inputs": { "height": "relief", "mask.flow": "flow" } },
    "relief": { "type": "noise.fbm", "params": { "height": 250, "scale": 1500 } }
  }
}
```

### Rivers — `water.rivers`

Rivers where enough of the land drains, their beds and banks dug into the relief, wider and deeper downstream, holding lakes in deep hollows. Follows the rain to the sea or the terrain's edge and lays a river wherever enough land drains: a thread at its source, wider and deeper as water joins it. Its water only ever goes down, cutting through the rims of shallow hollows and filling deep ones with lakes. Threshold sets how many rivers there are; Width and Depth those of the largest; Banks how gently the land slopes down to them. Water already on the map (wired to Water) is kept, and rivers end where they reach it.

A simulation: it works over the whole terrain at the simulations' resolution.

Inputs:

- `height` (height) — The relief the rivers run over.
- `water` (height, optional) — Water already there (a map's rivers and lakes): kept, and rivers end in it. Left empty: no water yet.

Outputs:

- `height` (height) — The relief with the rivers' beds and banks dug in.
- `rivers` (mask) — 1 in the rivers' beds, fading out over their banks.
- `water` (height) — The surface of the water where there is some, in blocks; well under the ground elsewhere.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `threshold` | number | 0.55 | 0 to 1 | How much land must drain to a place for a river to run there: higher, fewer and larger rivers. |
| `width` | number | 14 | 1 to 256 blocks | The width of the largest river; the smallest are a quarter of it. In blocks. Usually 2 to 64. |
| `depth` | number | 5 | 0.5 to 64 blocks | The depth of the largest river. In blocks. Usually 1 to 16. |
| `banks` | number | 1 | 0 to 8 | How far the banks slope down to the water, in river widths. Usually 0 to 4. |
| `meander` | number | 0.5 | 0 to 3 | How far the rivers wind about the water's way, in river widths: 0 follows it straight. Usually 0 to 1.5. |
| `lakes` | number | 0.5 | 0 to 1 | Which hollows hold lakes rather than being cut through: 0 none, 1 every hollow a block deep. |
| `sea` | number | 62 | -64 to 320 blocks | Rivers end where the ground goes under it. In blocks. |
| `seed` | seed | — | 0 to 4294967295 | Another seed gives another variation of the same settings. Left out, it comes from the node's name. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "out": { "type": "output.map", "inputs": { "height": "rivers" } },
    "relief": { "type": "noise.fbm", "params": { "height": 250, "scale": 1500 } },
    "rivers": { "type": "water.rivers", "inputs": { "height": "relief" } }
  }
}
```

### Lakes — `water.lakes`

Every hollow fills up to where it spills: with water, or with ground. Finds every hollow of the relief, where rain would gather, and fills it to the level where it overflows. Amount 0 makes them lakes; 1 fills them with ground, as sediment would, smoothing the land; between, both. What lies under the sea level is the sea's, not a lake.

A simulation: it works over the whole terrain at the simulations' resolution.

Inputs:

- `height` (height) — The relief whose hollows fill.

Outputs:

- `height` (height) — The relief, its hollows filled with ground as much as Amount says.
- `lakes` (mask) — Where the lakes are, by how deep (1 for a block and a half and more).
- `water` (height) — The surface of the water where there is some, in blocks; well under the ground elsewhere.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `amount` | number | 0 | 0 to 1 | How much of each hollow fills with ground rather than water: 0, lakes; 1, flat ground. |
| `sea` | number | 62 | -64 to 320 blocks | Under it the hollows are the sea's: hollows spill into it. In blocks. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "lakes": { "type": "water.lakes", "inputs": { "height": "relief" } },
    "out": { "type": "output.map", "inputs": { "height": "lakes" } },
    "relief": { "type": "noise.fbm", "params": { "height": 250, "scale": 1500 } }
  }
}
```

### Sea — `water.sea`

The sea up to a level, where the terrain's edge lets it in, and the shore around it. Floods the relief up to Level from the terrain's edge: low ground behind higher ground stays dry unless Connected is off. Its Shore mask follows the coast, a band of height around the level, for beaches and cliffs' feet.

A simulation: it works over the whole terrain at the simulations' resolution.

Inputs:

- `height` (height) — The relief the sea floods.

Outputs:

- `height` (height) — The relief, as it is.
- `water` (height) — The surface of the water where there is some, in blocks; well under the ground elsewhere.
- `shore` (mask) — A band along the coast, Shore blocks of height either side of the level.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `level` | number | 62 | -64 to 320 blocks | The sea's surface. In blocks. |
| `shore` | number | 4 | 0.5 to 64 blocks | How many blocks of height either side of the level the shore takes. Usually 0.5 to 16. |
| `connected` | boolean | true |  | Only where the sea comes in from the terrain's edge; off, every place under the level is sea. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "out": { "type": "output.map", "inputs": { "height": "sea" } },
    "relief": { "type": "noise.fbm", "params": { "height": 250, "scale": 1500 } },
    "sea": { "type": "water.sea", "inputs": { "height": "relief" } }
  }
}
```

### Erosion (1.2) — `legacy.erosion`

Water running down the slopes carries soil from the heights to the valleys: gullies, smoothed slopes, alluvial fans. Rain is how many drops fall on each sample; Erosion and Deposition how much soil a drop takes and leaves; Capacity how much it can carry; Evaporation how fast it dries; Radius the brush a drop wears with, in blocks.

A simulation: it works over the whole terrain at the simulations' resolution.

Inputs:

- `in` (height) — The relief it works on.

Outputs:

- `height` (height) — The relief worn.
- `flow` (mask) — Where water ran.
- `deposits` (mask) — Where soil settled.
- `wear` (mask) — Where soil was taken.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `rain` | number | 1 | 0 to 20 | Drops on each sample. |
| `erosion` | number | 0.3 | 0 to 1 | How much soil a drop takes. |
| `deposition` | number | 0.3 | 0 to 1 | How much soil a drop leaves. |
| `capacity` | number | 4 | 0 to 64 | How much soil a drop can carry. |
| `evaporation` | number | 0.02 | 0 to 0.5 | How fast a drop dries. |
| `radius` | number | 6 | 1 to 64 blocks | The brush a drop wears with. In blocks. |
| `seed` | integer | 5 | 0 to 4294967295 | Another seed drops the rain otherwise. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "erosion": { "type": "legacy.erosion", "inputs": { "in": "relief" } },
    "out": { "type": "output.map", "inputs": { "height": "erosion" } },
    "relief": { "type": "noise.fbm", "params": { "height": 250, "scale": 1500 } }
  }
}
```

### Rivers (1.2) — `legacy.rivers`

Rivers traced where the water of the whole terrain runs, wider and deeper downstream, dug into their valleys, with lakes in the hollows they cross. Threshold is how much land must drain to a place for a river to run there (higher: fewer, larger rivers); Depth and Width those of the largest; Sea the level where rivers end; Banks how far the banks slope down, in river widths; Lakes which hollows hold lakes.

A simulation: it works over the whole terrain at the simulations' resolution.

Inputs:

- `in` (height) — The relief it works on.
- `water_level` (height, optional) — Water already there (rivers of another node).

Outputs:

- `height` (height) — The relief with the rivers dug.
- `rivers` (mask) — Where they run.
- `water` (height) — The water's level.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `threshold` | number | 0.55 | 0 to 1 | How much land must drain to a place for a river to run there. |
| `depth` | number | 5 | 0 to 100 blocks | The depth of the largest river. In blocks. |
| `width` | number | 14 | 1 to 400 blocks | The width of the largest river. In blocks. |
| `sea` | number | 63 | -10000 to 320 blocks | The sea level, where rivers end (−10000: no sea). In blocks. |
| `banks` | number | 1 | 0 to 10 | How far the banks slope down to the water, in river widths. |
| `lakes` | number | 0.5 | 0 to 1 | Which hollows the rivers cross hold lakes. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "out": { "type": "output.map", "inputs": { "height": "rivers" } },
    "relief": { "type": "noise.fbm", "params": { "height": 250, "scale": 1500 } },
    "rivers": { "type": "legacy.rivers", "inputs": { "in": "relief" } }
  }
}
```

### Lakes (1.2) — `legacy.lakes`

The hollows water would fill, filled: lakes and flat valley floors. Amount goes from nothing (0) to every hollow full (1); Sea is the level under which a hollow is the sea's, not a lake.

A simulation: it works over the whole terrain at the simulations' resolution.

Inputs:

- `in` (height) — The relief it works on.

Outputs:

- `height` (height) — The relief.
- `lakes` (mask) — Where the lakes are.
- `water` (height) — The lakes' level.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `amount` | number | 1 | 0 to 1 | From nothing to every hollow full. |
| `sea` | number | 63 | -10000 to 320 blocks | The sea level (−10000: no sea). In blocks. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "lakes": { "type": "legacy.lakes", "inputs": { "in": "relief" } },
    "out": { "type": "output.map", "inputs": { "height": "lakes" } },
    "relief": { "type": "noise.fbm", "params": { "height": 250, "scale": 1500 } }
  }
}
```

### Sea (1.2) — `legacy.sea`

A sea level: where the ground is under water, and its shore. Level is the sea's height; Shore how wide the shore is, in blocks of height around it.

A simulation: it works over the whole terrain at the simulations' resolution.

Inputs:

- `in` (height) — The relief it works on.

Outputs:

- `height` (height) — The relief.
- `water` (mask) — Where the sea is.
- `shore` (mask) — Its shore.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `level` | number | 62 | -64 to 320 blocks | The sea's height. In blocks. |
| `shore` | number | 4 | 0 to 100 blocks | How wide the shore is, in blocks of height. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "out": { "type": "output.map", "inputs": { "height": "sea" } },
    "relief": { "type": "noise.fbm", "params": { "height": 250, "scale": 1500 } },
    "sea": { "type": "legacy.sea", "inputs": { "in": "relief" } }
  }
}
```

### Flow (1.2) — `legacy.flow`

Where the water of the terrain runs and gathers. Spread is how much small rills show beside the big rivers.

A simulation: it works over the whole terrain at the simulations' resolution.

Inputs:

- `in` (height) — The relief it works on.

Outputs:

- `mask` (mask) — Where water runs, 1 in the largest rivers.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `spread` | number | 0.5 | 0 to 1 | How much small rills show beside the big rivers. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "flow": { "type": "legacy.flow", "inputs": { "in": "relief" } },
    "out": { "type": "output.map", "inputs": { "height": "relief", "mask.mask": "flow" } },
    "relief": { "type": "noise.fbm", "params": { "height": 250, "scale": 1500 } }
  }
}
```

## Climate

### Temperature — `climate.temperature`

How warm each place is: warmer toward one side of the terrain, colder with height, milder by the sea. Toward is the warm side (90 for the south, as in the northern hemisphere); the temperature goes from Cold on the far side to Warm on that one. Lapse takes degrees off for every 100 blocks over the sea, so that summits freeze; Sea moderation pulls the sea toward a mild 12 °C. Variation adds slow warmer and colder patches. Feed Biomes with it, and the Map Output's Temperature.

Inputs:

- `height` (height, optional) — The relief (flat at the sea level when unconnected).

Outputs:

- `temperature` (mask) — 0 for −30 °C, 1 for +45 °C.
- `celsius` (height) — The temperature in °C.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `toward` | number | 90 | -360 to 360 degrees | The warm side, in degrees (0 east, 90 south). |
| `warm` | number | 28 | -30 to 45 °C | The temperature at the warm edge, in °C. A field connected to the input `@warm` drives it place by place. |
| `cold` | number | -8 | -30 to 45 °C | The temperature at the cold edge, in °C. A field connected to the input `@cold` drives it place by place. |
| `lapse` | number | 5 | 0 to 30 °C | How many degrees colder for every 100 blocks over the sea. In °C. |
| `sea_moderation` | number | 0.3 | 0 to 1 | How much the sea pulls toward a mild 12 °C. |
| `variation` | number | 2 | 0 to 20 °C | How much slow warmer and colder patches vary it, in °C. |
| `variation_scale` | number | 3000 | 32 to 65536 blocks | The size of those patches, in blocks. Usually 200 to 10000. |
| `seed` | seed | — | 0 to 4294967295 | Another seed gives another variation of the same settings. Left out, it comes from the node's name. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "out": { "type": "output.map", "inputs": { "height": "relief", "temperature": "temperature" } },
    "relief": { "type": "noise.fbm", "params": { "height": 250, "scale": 1500 } },
    "temperature": { "type": "climate.temperature" }
  }
}
```

### Wind — `climate.wind`

The prevailing wind over the terrain, bent around high ground. Direction is where the wind blows to (0 east, 90 south): the westerlies of Europe blow to the east. Deflection is how much the relief turns it aside instead of letting it climb. The output is a direction and a strength at each place, for dunes, rain and wind erosion.

Inputs:

- `height` (height, optional) — The relief (flat at the sea level when unconnected).

Outputs:

- `wind` (vector) — Where the wind blows, and how strongly.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `direction` | number | 0 | -360 to 360 degrees | Where the wind blows to, in degrees (0 east, 90 south). |
| `strength` | number | 1 | 0 to 10 | How strong it is. |
| `deflection` | number | 0.5 | 0 to 1 | How much the relief turns it aside. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "out": { "type": "output.map", "inputs": { "height": "relief" } },
    "relief": { "type": "noise.fbm", "params": { "height": 250, "scale": 1500 } },
    "wind": { "type": "climate.wind" }
  }
}
```

### Rainfall — `climate.rainfall`

Where rain falls: air carries moisture from the sea downwind and loses it as it rises, so that slopes facing the wind are wet and the land behind mountains dry. Direction is where the wind blows to. Moisture is how wet the air comes in; Orographic how much rain rising air sheds (high: deep rain shadows behind ranges); Refill how quickly the air takes moisture back over the sea; Drying how quickly it rains out over flat land. Worked over the whole terrain at the simulations' resolution; the result is from 0 (driest) to 1 (wettest).

A simulation: it works over the whole terrain at the simulations' resolution.

Inputs:

- `height` (height) — The relief the air crosses.

Outputs:

- `rain` (mask) — How much rain falls, from 0 to 1.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `direction` | number | 0 | -360 to 360 degrees | Where the wind blows to, in degrees (0 east, 90 south). |
| `moisture` | number | 1 | 0 to 4 | How wet the air comes in. |
| `orographic` | number | 1 | 0 to 10 | How much rain rising air sheds. |
| `refill` | number | 0.5 | 0 to 10 | How quickly the air takes moisture back over the sea. |
| `drying` | number | 0.5 | 0 to 10 | How quickly it rains out over flat land. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "out": { "type": "output.map", "inputs": { "height": "relief", "mask.rain": "rainfall" } },
    "rainfall": { "type": "climate.rainfall", "inputs": { "height": "relief" } },
    "relief": { "type": "noise.fbm", "params": { "height": 250, "scale": 1500 } }
  }
}
```

### Humidity — `climate.humidity`

How humid each place is: damp by the sea, drying inland, wetter where rain falls. Inland is how far from the sea the air stays damp. Connect Rainfall for rain shadows and wet windward slopes; Rain weight is how much it counts against the sea's nearness. Variation adds slow patches; Irregularity lets the damp band by the coast widen and narrow along it, so that forests do not follow the shore at one width. Feed Biomes with it, and the Map Output's Humidity.

A simulation: it works over the whole terrain at the simulations' resolution.

Inputs:

- `height` (height) — The relief: where it is under the sea level is sea.
- `rainfall` (mask, optional) — How much rain falls (the sea's nearness alone, when unconnected).

Outputs:

- `humidity` (mask) — From 0 (dry) to 1 (humid).

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `inland` | number | 1500 | 1 to 65536 blocks | How far from the sea the air stays damp, in blocks. Usually 100 to 8000. |
| `rain_weight` | number | 0.6 | 0 to 1 | How much rainfall counts against the sea's nearness. |
| `variation` | number | 0.1 | 0 to 1 | How much slow patches vary it. |
| `irregularity` | number | 0.5 | 0 to 1 | How much the damp coastal band widens and narrows along the coast instead of keeping one width. |
| `seed` | seed | — | 0 to 4294967295 | Another seed gives another variation of the same settings. Left out, it comes from the node's name. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "humidity": { "type": "climate.humidity", "inputs": { "height": "relief" } },
    "out": { "type": "output.map", "inputs": { "height": "relief", "humidity": "humidity" } },
    "relief": { "type": "noise.fbm", "params": { "height": 250, "scale": 1500 } }
  }
}
```

### Biomes — `climate.biomes`

The game's biomes from the climate: oceans by their warmth, beaches, rivers, peaks and meadows by height, and on land a table of temperature against humidity. Connect Temperature and Humidity (and Height, Water for oceans, beaches, rivers and peaks). Above Peaks blocks over the sea lie frozen, jagged or stony peaks, meadows and groves just under them; within Beach blocks of the sea level, beaches. On land: snowy plains and taiga in the cold, plains and forests in the mild, savanna and swamp in the warm, desert, badlands and jungle in the hot. The Biome output goes to the Map Output; the Colour output shows them as maps do.

Inputs:

- `temperature` (mask) — The temperature, as a Temperature node gives it.
- `humidity` (mask) — The humidity, from 0 to 1.
- `height` (height, optional) — The relief, for oceans, beaches and peaks (land everywhere when unconnected).
- `water` (height, optional) — The water's surface, for rivers and lakes.

Outputs:

- `biome` (biome) — The biome of each place.
- `color` (color) — Each biome's colour, as maps show it.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `peaks` | number | 170 | 0 to 4096 blocks | How high over the sea the peaks begin, in blocks. Usually 0 to 300. |
| `beach` | number | 3 | 0 to 64 blocks | How high over the sea beaches reach, in blocks. |
| `wiggle` | number | 0.5 | 0 to 1 | How much the borders between biomes wander instead of following the climate's smooth lines. |
| `wiggle_scale` | number | 300 | 16 to 16000 blocks | The size of the borders' wanderings, in blocks. Usually 50 to 2000. |
| `seed` | seed | — | 0 to 4294967295 | Another seed gives another variation of the same settings. Left out, it comes from the node's name. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "biomes": { "type": "climate.biomes", "inputs": { "humidity": "slopes", "temperature": "slopes" } },
    "out": { "type": "output.map", "inputs": { "biome": "biomes", "height": "relief" } },
    "relief": { "type": "noise.fbm", "params": { "height": 250, "scale": 1500 } },
    "slopes": { "type": "mask.slope", "inputs": { "height": "relief" } }
  }
}
```

## Masks

### Height — `mask.height`

Where the ground lies between two heights: 1 from From to To, fading over Softness on either side. Picks bands of altitude: the snowy summits (From high, To at the top), the low ground by the sea, a band of forest. Heights are in blocks, as the relief's; Softness fades the edges so that what is painted through it mixes.

Inputs:

- `height` (height) — The relief to measure.

Outputs:

- `mask` (mask) — 1 between From and To.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `from` | number | 90 | -2048 to 4096 blocks | The bottom of the band, in blocks. Usually -64 to 320. |
| `to` | number | 200 | -2048 to 4096 blocks | The top of the band, in blocks. Usually -64 to 320. |
| `softness` | number | 10 | 0 to 1024 blocks | How far the edges fade, in blocks. Usually 0 to 60. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "height": { "type": "mask.height", "inputs": { "height": "relief" } },
    "out": { "type": "output.map", "inputs": { "height": "relief", "mask.mask": "height" } },
    "relief": { "type": "noise.fbm", "params": { "height": 250, "scale": 1500 } }
  }
}
```

### Slope — `mask.slope`

Where the ground is steep: 0 below From degrees, 1 above To, a smooth step between (and back to 0 above Upper, when set). Picks slopes by their steepness: cliffs for rock, gentle ground for grass and snow (inverted). Upper keeps only a band of slopes (scree between 30° and 40°, say). The slope is measured between neighbouring samples, so a coarse preview shows slopes a little gentler than the final build.

Inputs:

- `height` (height) — The relief to measure.

Outputs:

- `mask` (mask) — 1 where it is steep.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `from` | number | 25 | 0 to 90 degrees | Slopes gentler than this are 0. In degrees. |
| `to` | number | 45 | 0 to 90 degrees | Slopes steeper than this are 1. In degrees. |
| `upper` | number | 90 | 0 to 90 degrees | Slopes steeper than this are 0 again (90: none). In degrees. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "out": { "type": "output.map", "inputs": { "height": "relief", "mask.mask": "slope" } },
    "relief": { "type": "noise.fbm", "params": { "height": 250, "scale": 1500 } },
    "slope": { "type": "mask.slope", "inputs": { "height": "relief" } }
  }
}
```

### Curvature — `mask.curvature`

Where the ground bulges into ridges or hollows into valleys, at the scale asked for. Compares the relief with its own smoothing over Scale blocks: higher than its surroundings is a ridge, lower a hollow. A small scale finds gullies and crests, a large one valleys and spurs. Sensitivity says how slight a bulge or hollow reaches 1. Ridges take rock and wind, hollows soil, water and snow drifts.

Inputs:

- `height` (height) — The relief to measure.

Outputs:

- `mask` (mask) — 1 on ridges or in hollows, as the mode says.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `mode` | choice | `convex` | `convex`, `concave`, `both` | Ridges, hollows, or both. `convex`: Where the ground bulges: crests, ridges, the edges of cliffs. `concave`: Where the ground hollows: valleys, gullies, the feet of slopes. `both`: 0.5 on even ground, toward 1 in hollows and toward 0 on ridges. |
| `scale` | number | 32 | 2 to 8192 blocks | The size of the ridges and hollows looked for, in blocks. Usually 4 to 512. |
| `sensitivity` | number | 0.5 | 0 to 1 | How slight a bulge or a hollow is enough to reach 1. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "curvature": { "type": "mask.curvature", "inputs": { "height": "relief" } },
    "out": { "type": "output.map", "inputs": { "height": "relief", "mask.mask": "curvature" } },
    "relief": { "type": "noise.fbm", "params": { "height": 250, "scale": 1500 } }
  }
}
```

### Facing — `mask.facing`

Where slopes face a direction: 1 on slopes turned toward it, fading within Spread degrees, 0 on flat ground. Direction is where the slopes look to (0 east, 90 south, 180 west, 270 north). North-facing slopes keep snow and moss in the northern hemisphere, south-facing ones are dry and sunny; slopes facing the prevailing wind take its rain. Ground flatter than Min slope counts as facing nowhere.

Inputs:

- `height` (height) — The relief to measure.

Outputs:

- `mask` (mask) — 1 on slopes facing Direction.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `direction` | number | 270 | -360 to 360 degrees | Where the slopes look to, in degrees (0 east, 90 south, 270 north). |
| `spread` | number | 90 | 1 to 180 degrees | How far from Direction a slope still counts, in degrees. |
| `min_slope` | number | 5 | 0 to 45 degrees | Ground flatter than this faces nowhere. In degrees. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "facing": { "type": "mask.facing", "inputs": { "height": "relief" } },
    "out": { "type": "output.map", "inputs": { "height": "relief", "mask.mask": "facing" } },
    "relief": { "type": "noise.fbm", "params": { "height": 250, "scale": 1500 } }
  }
}
```

### Sunlight — `mask.exposure`

How much sunlight the ground takes from a sun at a given direction and height: 1 facing it, 0 turned away. Azimuth is where the sun stands (0 east, 90 south, 180 west), Elevation how high above the horizon. Flat ground takes the sine of the elevation. Sunlit slopes dry out and lose their snow first; shaded ones keep moss, ice and forests that like the cold.

Inputs:

- `height` (height) — The relief to measure.

Outputs:

- `mask` (mask) — The share of full sunlight.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `azimuth` | number | 90 | -360 to 360 degrees | Where the sun stands, in degrees (0 east, 90 south). |
| `elevation` | number | 35 | 0 to 90 degrees | How high the sun stands above the horizon, in degrees. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "exposure": { "type": "mask.exposure", "inputs": { "height": "relief" } },
    "out": { "type": "output.map", "inputs": { "height": "relief", "mask.mask": "exposure" } },
    "relief": { "type": "noise.fbm", "params": { "height": 250, "scale": 1500 } }
  }
}
```

### Openness — `mask.occlusion`

How open to the sky the ground is: 1 on summits and plains, lower in gorges, gullies and at the feet of cliffs. Looks around each place in Directions directions up to Radius blocks, and takes how high the horizon rises. Narrow valleys, crevices and the feet of walls are dark; the dark is where moss, shade plants and old snow keep. Wide radii are worked at a coarser level.

Inputs:

- `height` (height) — The relief to measure.

Outputs:

- `mask` (mask) — 1 open to the sky, lower where the horizon rises.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `radius` | number | 64 | 2 to 4096 blocks | How far around each place the horizon is looked for, in blocks. Usually 4 to 512. |
| `directions` | integer | 8 | 4 to 32 | How many directions are looked in. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "occlusion": { "type": "mask.occlusion", "inputs": { "height": "relief" } },
    "out": { "type": "output.map", "inputs": { "height": "relief", "mask.mask": "occlusion" } },
    "relief": { "type": "noise.fbm", "params": { "height": 250, "scale": 1500 } }
  }
}
```

### Snow — `mask.snow`

Where snow lies: above a snow line, lower on slopes facing north and in hollows where it drifts, never on slopes too steep to hold it. Line is the height above which snow lies on open ground, Softness how gradually it fades in. North bias lowers the line on north-facing slopes (the shady side), Drift fills hollows, Breakup frays the edge into patches. Max slope keeps cliffs bare. Depth, when above 0, also raises the Height output by the snow's depth.

Inputs:

- `height` (height) — The relief to measure.

Outputs:

- `mask` (mask) — 1 under snow.
- `height` (height) — The relief with the snow's depth on it.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `line` | number | 180 | -2048 to 4096 blocks | The height above which snow lies on open ground, in blocks. Usually -64 to 320. A field connected to the input `@line` drives it place by place. |
| `softness` | number | 20 | 0 to 512 blocks | How gradually the snow fades in around the line, in blocks. Usually 0 to 100. |
| `max_slope` | number | 40 | 0 to 90 degrees | Slopes steeper than this stay bare. In degrees. |
| `north_bias` | number | 20 | 0 to 512 blocks | How much lower the line comes on north-facing slopes, in blocks. Usually 0 to 100. |
| `drift` | number | 0.5 | 0 to 1 | How much snow gathers in hollows. |
| `breakup` | number | 0.3 | 0 to 1 | How much the snow's edge frays into patches. |
| `depth` | number | 0 | 0 to 64 blocks | How deep the snow is on the Height output, in blocks. |
| `seed` | seed | — | 0 to 4294967295 | Another seed gives another variation of the same settings. Left out, it comes from the node's name. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "out": { "type": "output.map", "inputs": { "height": "relief", "mask.mask": "snow" } },
    "relief": { "type": "noise.fbm", "params": { "height": 250, "scale": 1500 } },
    "snow": { "type": "mask.snow", "inputs": { "height": "relief" } }
  }
}
```

### Wetness — `mask.wetness`

Where the ground stays wet: where much water gathers and it is flat (with a Flow mask), or in flat hollows. With a Flow mask from erosion connected, this is the topographic wetness index: the more water flows through a place and the flatter it is, the wetter. Without one, flat hollows of Scale blocks count as wet. Wet ground takes moss, mud, reeds, swamps and dense forest.

Inputs:

- `height` (height) — The relief to measure.
- `flow` (mask, optional) — How much water flows through each place, from erosion (hollows decide when unconnected).

Outputs:

- `mask` (mask) — 1 where the ground stays wet.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `scale` | number | 32 | 2 to 512 blocks | The size of the hollows that count, in blocks (without Flow). Usually 4 to 256. |
| `sensitivity` | number | 0.5 | 0 to 1 | How little water or hollowing is enough to count as wet. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "out": { "type": "output.map", "inputs": { "height": "relief", "mask.mask": "wetness" } },
    "relief": { "type": "noise.fbm", "params": { "height": 250, "scale": 1500 } },
    "wetness": { "type": "mask.wetness", "inputs": { "height": "relief" } }
  }
}
```

### Distance — `mask.distance`

How far each place lies from where a mask is: 1 on the mask, fading to 0 Reach blocks away, and the distance itself in blocks. Feed it where something is — the sea, rivers, a road, a painted area — and get a band around it: beaches and reeds by the water, a margin along a cliff, forests thinning away from rivers. Threshold says where the mask counts as there. Worked over the whole terrain at the simulations' resolution.

A simulation: it works over the whole terrain at the simulations' resolution.

Inputs:

- `mask` (mask) — The mask to work on.

Outputs:

- `mask` (mask) — 1 on the mask, 0 Reach blocks from it.
- `distance` (height) — The distance to the mask, in blocks.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `threshold` | number | 0.5 | 0 to 1 | Where the mask counts as there. |
| `reach` | number | 200 | 1 to 65536 blocks | How far from the mask the first output fades to 0, in blocks. Usually 4 to 2000. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "distance": { "type": "mask.distance", "inputs": { "mask": "slopes" } },
    "out": { "type": "output.map", "inputs": { "height": "relief", "mask.mask": "distance" } },
    "relief": { "type": "noise.fbm", "params": { "height": 250, "scale": 1500 } },
    "slopes": { "type": "mask.slope", "inputs": { "height": "relief" } }
  }
}
```

### Threshold — `mask.threshold`

A mask made sharp: 1 above At, 0 below, over Softness. Turns a soft mask (or any field) into a crisp one. Softness 0 gives a hard edge; a little softness keeps it from stair-stepping.

Inputs:

- `mask` (mask) — The mask to work on.

Outputs:

- `mask` (mask) — 1 above At.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `at` | number | 0.5 | -4096 to 4096 | The value where the mask turns from 0 to 1. Usually 0 to 1. |
| `softness` | number | 0.05 | 0 to 4096 | How wide the turn is. Usually 0 to 0.5. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "out": { "type": "output.map", "inputs": { "height": "relief", "mask.mask": "threshold" } },
    "relief": { "type": "noise.fbm", "params": { "height": 250, "scale": 1500 } },
    "slopes": { "type": "mask.slope", "inputs": { "height": "relief" } },
    "threshold": { "type": "mask.threshold", "inputs": { "mask": "slopes" } }
  }
}
```

### Invert — `mask.invert`

A mask turned over: 1 where it was 0, 0 where it was 1. Everywhere the mask is not: gentle ground from a slope mask, open ground from a forest mask, land from a sea mask.

Inputs:

- `mask` (mask) — The mask to work on.

Outputs:

- `mask` (mask) — 1 − the mask.

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "invert": { "type": "mask.invert", "inputs": { "mask": "slopes" } },
    "out": { "type": "output.map", "inputs": { "height": "relief", "mask.mask": "invert" } },
    "relief": { "type": "noise.fbm", "params": { "height": 250, "scale": 1500 } },
    "slopes": { "type": "mask.slope", "inputs": { "height": "relief" } }
  }
}
```

### Grow or shrink — `mask.expand`

A mask grown outward by Radius blocks, or shrunk inward with a negative radius. Grow a river mask into its banks, a road into its verges; shrink a land mask away from the shore, a forest away from its edge. Wide radii are worked at a coarser level and come back smooth.

Inputs:

- `mask` (mask) — The mask to work on.

Outputs:

- `mask` (mask) — The mask grown or shrunk.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `radius` | number | 8 | -4096 to 4096 blocks | How far the mask grows, in blocks (negative: shrinks). Usually -128 to 128. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "expand": { "type": "mask.expand", "inputs": { "mask": "slopes" } },
    "out": { "type": "output.map", "inputs": { "height": "relief", "mask.mask": "expand" } },
    "relief": { "type": "noise.fbm", "params": { "height": 250, "scale": 1500 } },
    "slopes": { "type": "mask.slope", "inputs": { "height": "relief" } }
  }
}
```

## Colours

### Colour ramp — `color.ramp`

Colours the ground by its height, from one colour at Low to another at High. The simplest colouring: a colour for the low ground and one for the high ground, blended between. Feed the Map Output's Albedo with it, or mix several through masks.

Inputs:

- `height` (height) — The relief to colour.

Outputs:

- `color` (color) — The colours.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `low` | number | 60 | -2048 to 4096 blocks | The height that takes the low colour. In blocks. Usually -64 to 320. |
| `high` | number | 250 | -2048 to 4096 blocks | The height that takes the high colour. In blocks. Usually -64 to 320. |
| `color_low` | colour | `#5c7a3a` |  | The colour of the low ground. |
| `color_high` | colour | `#e8e8ea` |  | The colour of the high ground. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "out": { "type": "output.map", "inputs": { "albedo": "ramp", "height": "relief" } },
    "ramp": { "type": "color.ramp", "inputs": { "height": "relief" } },
    "relief": { "type": "noise.fbm", "params": { "height": 250, "scale": 1500 } }
  }
}
```

### Gradient — `color.gradient`

Colours from a value — a height, a mask — mapped from Low to High onto a gradient: one from nature, or one of your own. The input from Low to High picks a colour along the gradient (Auto takes the input's own lowest and highest values). Presets give gradients from nature; Custom uses the stops of Gradient. Feed it a height for bands of altitude, a slope mask for rock on steep ground, a strata noise for badlands. Variation jitters the colours a little so that large areas do not look flat.

Inputs:

- `in` (height) — The value that picks the colour: a height or a mask.

Outputs:

- `color` (color) — The colours.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `preset` | choice | `alpine` | `custom`, `alpine`, `desert`, `tropical`, `tundra`, `volcanic`, `badlands` | A gradient from nature, or Custom. `custom`: The stops of Gradient. `alpine`: Meadow, forest, brown and grey rock, snow. `desert`: Pale sand to red rock. `tropical`: White sand, jungle greens, dark rock. `tundra`: Olive moss, grey stone, snow. `volcanic`: Black ash and basalt, rusty slopes. `badlands`: Bands of red, orange and cream rock. |
| `gradient` | gradient | `[{"at":0.0,"color":"#3d5a2a"},{"at":0.5,"color":"#7d7259"},{"at":1.0,"color":"#f2f5f7"}]` |  | The stops, when Preset is Custom. |
| `low` | number | 62 | -4096 to 4096 blocks | The value that takes the gradient's first colour. In blocks. Usually -64 to 320. |
| `high` | number | 300 | -4096 to 4096 blocks | The value that takes its last colour. In blocks. Usually -64 to 320. |
| `auto` | boolean | false |  | Take the input's own lowest and highest values as Low and High. |
| `reverse` | boolean | false |  | Run the gradient the other way. |
| `variation` | number | 0.05 | 0 to 1 | How much the colours are jittered. |
| `seed` | seed | — | 0 to 4294967295 | Another seed gives another variation of the same settings. Left out, it comes from the node's name. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "gradient": { "type": "color.gradient", "inputs": { "in": "relief" } },
    "out": { "type": "output.map", "inputs": { "albedo": "gradient", "height": "relief" } },
    "relief": { "type": "noise.fbm", "params": { "height": 250, "scale": 1500 } }
  }
}
```

### Solid colour — `color.solid`

One colour everywhere. A colour to mix in through a mask: snow white where the snow mask says, grey rock on cliffs.

Outputs:

- `color` (color) — The colours.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `color` | colour | `#7f7f7f` |  | The colour. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "out": { "type": "output.map", "inputs": { "albedo": "solid", "height": "relief" } },
    "relief": { "type": "noise.fbm", "params": { "height": 250, "scale": 1500 } },
    "solid": { "type": "color.solid" }
  }
}
```

### Mix colours — `color.mix`

Two colourings combined through a mask: B laid over A where the mask is 1, by a mode. The way to put rock colour on cliffs (a slope mask), snow on summits (a snow mask), sand on beaches (a distance mask from the sea). Strength fades the whole mix.

Inputs:

- `a` (color) — The colours mixed into.
- `b` (color) — The colours mixed in.
- `mask` (mask, optional) — Where B is mixed in (everywhere when unconnected).

Outputs:

- `color` (color) — The colours.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `mode` | choice | `mix` | `mix`, `multiply`, `screen`, `overlay` | How B combines with A. `mix`: B over A through the mask. `multiply`: A darkened by B: shadows, wet ground. `screen`: A lightened by B: dust, frost. `overlay`: B deepening A's contrast: tinting while keeping detail. |
| `strength` | number | 1 | 0 to 1 | How much of the mix is taken: 0 leaves A. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "colours": { "type": "color.gradient", "inputs": { "in": "relief" } },
    "mix": { "type": "color.mix", "inputs": { "a": "colours", "b": "colours" } },
    "out": { "type": "output.map", "inputs": { "albedo": "mix", "height": "relief" } },
    "relief": { "type": "noise.fbm", "params": { "height": 250, "scale": 1500 } }
  }
}
```

### Adjust colours — `color.adjust`

Colours turned in hue, made more or less vivid, lighter or darker, with more or less contrast. Hue turns every colour around the wheel; Saturation 0 makes greys, 2 doubles the vividness; Brightness and Contrast as in a photo editor.

Inputs:

- `color` (color) — The colours to adjust.

Outputs:

- `color` (color) — The colours.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `hue` | number | 0 | -180 to 180 degrees | How far every colour turns around the wheel. In degrees. |
| `saturation` | number | 1 | 0 to 3 | How vivid: 0 grey, 1 as it is. |
| `brightness` | number | 1 | 0 to 3 | How light: 1 as it is. |
| `contrast` | number | 1 | 0 to 3 | How far colours spread from mid-grey: 1 as it is. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "adjust": { "type": "color.adjust", "inputs": { "color": "colours" } },
    "colours": { "type": "color.gradient", "inputs": { "in": "relief" } },
    "out": { "type": "output.map", "inputs": { "albedo": "adjust", "height": "relief" } },
    "relief": { "type": "noise.fbm", "params": { "height": 250, "scale": 1500 } }
  }
}
```

### Terrain layers — `terrain.layers`

The terrain (block material) of each place, from masks: a base terrain, and up to six layers laid over it where their masks say. Base is the terrain everywhere (or Under, where connected); each layer lays its terrain where its mask passes Threshold, later layers over earlier ones (rock on slopes, then gravel on scree, then sand on beaches, then snow). Dither frays the edges so that terrains mix block by block. Terrains are named as in the world's terrain list (Grass, Rock, Gravel, Sand, Snow…). Feed the Map Output's Terrain with it.

Inputs:

- `mask_1` (mask, optional) — Where this layer's terrain lies.
- `mask_2` (mask, optional) — Where this layer's terrain lies.
- `mask_3` (mask, optional) — Where this layer's terrain lies.
- `mask_4` (mask, optional) — Where this layer's terrain lies.
- `mask_5` (mask, optional) — Where this layer's terrain lies.
- `mask_6` (mask, optional) — Where this layer's terrain lies.
- `under` (terrain, optional) — The terrains where no layer is laid (the map's own, through a Map input). Left empty: Base everywhere.

Outputs:

- `terrain` (terrain) — The terrain of each place.
- `color` (color) — Each terrain's colour, for previews.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `base` | text | `Grass` |  | The terrain everywhere no layer covers. |
| `terrain_1` | text | `Rock` |  | The terrain this layer lays, by its name. |
| `terrain_2` | text | `Gravel` |  | The terrain this layer lays, by its name. |
| `terrain_3` | text | `Sand` |  | The terrain this layer lays, by its name. |
| `terrain_4` | text | `Snow` |  | The terrain this layer lays, by its name. |
| `terrain_5` | text | `` |  | The terrain this layer lays, by its name. |
| `terrain_6` | text | `` |  | The terrain this layer lays, by its name. |
| `threshold` | number | 0.5 | 0 to 1 | Where a layer's mask lays its terrain. |
| `dither` | number | 0.3 | 0 to 1 | How much the edges fray, block by block. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "layers": { "type": "terrain.layers" },
    "out": { "type": "output.map", "inputs": { "height": "relief", "terrain": "layers" } },
    "relief": { "type": "noise.fbm", "params": { "height": 250, "scale": 1500 } }
  }
}
```

### Select terrain — `terrain.select`

A mask of where a terrain lies: 1 where the terrain is the one named. Picks a terrain out of a terrain map (from Terrain layers, or the map's own through a Map input), to put plants on grass only, to raise snow where there is snow.

Inputs:

- `terrain` (terrain) — The terrains.

Outputs:

- `mask` (mask) — 1 where the terrain is Name.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `name` | text | `Grass` |  | The terrain picked, by its name. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "out": { "type": "output.map", "inputs": { "height": "relief", "mask.mask": "select" } },
    "relief": { "type": "noise.fbm", "params": { "height": 250, "scale": 1500 } },
    "select": { "type": "terrain.select", "inputs": { "terrain": "terrains" } },
    "terrains": { "type": "terrain.layers" }
  }
}
```

### Auto colour (1.2) — `legacy.auto_color`

Natural colours from the relief alone: valleys, meadows, rock on the steep slopes, snow on the peaks. Palette is the land's colours; Snow line where snow starts, in blocks; Rock slope the slope where rock shows, in degrees.

A simulation: it works over the whole terrain at the simulations' resolution.

Inputs:

- `height` (height) — The relief to colour.

Outputs:

- `colour` (color) — Its colours.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `palette` | choice | `temperate` | `temperate`, `alpine`, `desert`, `arctic`, `volcanic`, `tropical`, `lunar`, `martian` | The land's colours. `temperate`: Meadows, forests, grey rock, snow. `alpine`: High meadows, scree, snow. `desert`: Sand and red rock. `arctic`: Tundra, ice, snow. `volcanic`: Ash and basalt. `tropical`: Jungle and dark rock. `lunar`: Greys of the moon. `martian`: Rusty dust and rock. |
| `snow_line` | number | 200 | -64 to 4096 blocks | Where snow starts. In blocks. |
| `rock_slope` | number | 38 | 0 to 90 degrees | The slope where rock shows. In degrees. |
| `seed` | integer | 1 | 0 to 4294967295 | Another seed varies the colours. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "auto_color": { "type": "legacy.auto_color", "inputs": { "height": "relief" } },
    "out": { "type": "output.map", "inputs": { "albedo": "auto_color", "height": "relief" } },
    "relief": { "type": "noise.fbm", "params": { "height": 250, "scale": 1500 } }
  }
}
```

## Combine

### Blend — `combine.blend`

Combines two reliefs, A and B, by a mode, through a mask where one is given. The result is A where the mask is 0 (or Strength is 0), and A combined with B by the mode where it is 1. Add raises mountains on a plain, Max sets landforms side by side, Replace with a mask puts one landscape inside another.

Inputs:

- `a` (height) — The relief combined into.
- `b` (height) — The relief combined in.
- `mask` (mask, optional) — Where B is combined (everywhere when unconnected).

Outputs:

- `height` (height) — A combined with B.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `mode` | choice | `add` | `add`, `subtract`, `multiply`, `max`, `min`, `average`, `difference`, `screen`, `overlay`, `replace` | How A and B combine. `add`: A + B: B raised on top of A. `subtract`: A − B: B dug out of A. `multiply`: A × B: with a mask, A where the mask is 1 and 0 where it is 0. `max`: The higher of A and B: two landforms side by side. `min`: The lower of A and B: one relief cut down by another. `average`: Halfway between A and B. `difference`: How far A and B lie apart. `screen`: 1 − (1 − A)(1 − B), for masks: either one. `overlay`: For masks: B deepens A's contrast. `replace`: B: with a mask, B where the mask is 1 and A elsewhere. |
| `strength` | number | 1 | 0 to 1 | How much of the combination is taken: 0 leaves A. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "blend": { "type": "combine.blend", "inputs": { "a": "relief", "b": "relief" } },
    "out": { "type": "output.map", "inputs": { "height": "blend" } },
    "relief": { "type": "noise.fbm", "params": { "height": 250, "scale": 1500 } }
  }
}
```

## Geometry

### Scatter — `points.scatter`

Points spread over the terrain, as many as a density mask asks, never closer than a spacing. One point at most in each square of Spacing blocks, at a place of its own inside it: kept where a random draw falls under the density there. Trees, rocks, ruins or the places villages may start: give it the mask of where they may stand, and the relief for the points' heights.

It makes geometry over the whole terrain at once, reading reliefs and masks at the simulations' resolution.

Inputs:

- `density` (mask, optional) — How likely a point is at each place, from 0 to 1. Left empty: 1 everywhere.
- `height` (height, optional) — The relief the points stand on: their height. Left empty: the sea level.

Outputs:

- `points` (points) — The points, each standing on the relief, turned at random.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `spacing` | number | 64 | 2 to 16384 blocks | The size of the squares the points are drawn in: no two points are much closer than half of it. In blocks. Usually 4 to 1024. |
| `jitter` | number | 0.8 | 0 to 1 | How far a point wanders inside its square: 0 lines them up on a grid. |
| `seed` | seed | — | 0 to 4294967295 | Another seed gives another variation of the same settings. Left out, it comes from the node's name. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "out": { "type": "output.map", "inputs": { "height": "relief" } },
    "relief": { "type": "noise.fbm", "params": { "height": 250, "scale": 1500 } },
    "scatter": { "type": "points.scatter" }
  }
}
```

### Points to mask — `points.mask`

A mask of discs round points: 1 within Radius of one, fading to 0 over Softness. Where points stand, as a mask: keep the forest off the places a village takes, raise a mound under each ruin, paint the ground round each tree.

Inputs:

- `points` (points) — The points.

Outputs:

- `mask` (mask) — 1 near the points.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `radius` | number | 8 | 0 to 4096 blocks | How far round each point the mask is 1, in blocks (times the point's scale). Usually 0 to 128. |
| `softness` | number | 4 | 0 to 4096 blocks | How far beyond Radius it fades to 0, in blocks. Usually 0 to 64. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "mask": { "type": "points.mask", "inputs": { "points": "places" } },
    "out": { "type": "output.map", "inputs": { "height": "relief", "mask.mask": "mask" } },
    "places": { "type": "points.scatter", "inputs": { "height": "relief" }, "params": { "spacing": 256 } },
    "relief": { "type": "noise.fbm", "params": { "height": 250, "scale": 1500 } }
  }
}
```

### Village site — `village.site`

Where a village's squares go: on flat dry land of the area, one for each district. The first stage of a village. The squares stand where the land is flattest and driest near the middle of the area, as much as Flat ground asks; a village of more than forty houses gets more districts, each round its own square. Houses, Density and Spread set how far the village reaches, which the roads follow.

It makes geometry over the whole terrain at once, reading reliefs and masks at the simulations' resolution.

Inputs:

- `height` (height, optional) — The relief the village stands on. Left empty: the sea level.
- `area` (mask, optional) — Where the village may go: over a half. At most 4096 × 4096 blocks. Left empty: the whole terrain.

Outputs:

- `squares` (points) — The squares, the village's first.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `houses` | integer | 18 | 3 to 120 | How many houses (not counting wells, stalls, mills and towers): the village reaches as far as it needs to hold them. |
| `density` | number | 0.6 | 0.2 to 1 | From airy (0.2) to tight (1): how close the plots follow one another along the roads. |
| `spread` | number | 0.5 | 0 to 1 | From a compact hamlet (0) to a village strung out along its roads (1). |
| `flat` | number | 0.7 | 0 to 1 | How much the squares prefer flat land over the area's middle. |
| `seed` | seed | — | 0 to 4294967295 | Another seed gives another variation of the same settings. Left out, it comes from the node's name. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "out": { "type": "output.map", "inputs": { "height": "relief" } },
    "relief": { "type": "noise.fbm", "params": { "height": 250, "scale": 1500 } },
    "site": { "type": "village.site" }
  }
}
```

### Village roads — `village.roads`

A village's roads: main roads that follow the land between its squares and out of it, side roads off them. Main roads find their way between the squares and toward the area's edge, choosing gentle slopes and dry land; side roads branch off them every few dozen blocks, longer as Spread grows. Type is the blocks they are laid in when the map is applied; Width the main roads' width (0: the pack's).

It makes geometry over the whole terrain at once, reading reliefs and masks at the simulations' resolution.

It reads the files its settings name, and follows them when they change.

Inputs:

- `height` (height, optional) — The relief the village stands on. Left empty: the sea level.
- `area` (mask, optional) — Where the village may go: over a half. At most 4096 × 4096 blocks. Left empty: the whole terrain.
- `squares` (points) — The squares, from Village site.

Outputs:

- `roads` (splines) — The roads: main, side, and the squares.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `pack` | text | `` |  | The folder of the village pack: its schematics by type (house/, church/, farm/…) and its village.json. |
| `road` | choice | `theme` | `theme`, `dirt`, `premium`, `stone`, `planks` | The blocks the roads are laid in. `theme`: The blocks the pack's village.json gives its roads. `dirt`: Dirt path and a little coarse dirt. `premium`: Dirt path and gravel, coarse dirt on the edges. `stone`: Cobblestone, andesite and stone bricks, slabs where they climb. `planks`: Spruce and oak planks. |
| `spread` | number | 0.5 | 0 to 1 | From a compact hamlet (0) to a village strung out along its roads (1). |
| `width` | integer | 0 | 0 to 9 | The main roads' width in blocks; 0 keeps the pack's. |
| `theme` | text | `` |  | Which theme of the pack, when it has several: the first one when left empty. |
| `seed` | seed | — | 0 to 4294967295 | Another seed gives another variation of the same settings. Left out, it comes from the node's name. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "out": { "type": "output.map", "inputs": { "height": "relief", "roads": "roads" } },
    "places": { "type": "points.scatter", "inputs": { "height": "relief" }, "params": { "spacing": 256 } },
    "relief": { "type": "noise.fbm", "params": { "height": 250, "scale": 1500 } },
    "roads": { "type": "village.roads", "inputs": { "squares": "places" } }
  }
}
```

### Village walls — `village.walls`

What runs round a village, three blocks inside its area: a fence, a hedge, a low wall or ramparts with towers, a gate where a main road crosses. The wall's line follows the area's outline; side roads stop short of it, main roads pass through its gates (Roads out is the roads cut so). With Walls off, it draws no wall and lets the roads through as they are.

It makes geometry over the whole terrain at once, reading reliefs and masks at the simulations' resolution.

It reads the files its settings name, and follows them when they change.

Inputs:

- `height` (height, optional) — The relief the village stands on. Left empty: the sea level.
- `area` (mask, optional) — Where the village may go: over a half. At most 4096 × 4096 blocks. Left empty: the whole terrain.
- `roads` (splines) — The roads, from Village roads (or Village walls).
- `squares` (points, optional) — The squares, from Village site: the walls go round the village's reach of them. Left empty: round the whole area.
- `buildings` (instances, optional) — The buildings, from Village buildings: the walls go round them, eight blocks out.
- `fields` (footprints, optional) — The fields, kept within the walls too.

Outputs:

- `walls` (splines) — The wall's line, closed.
- `towers` (points) — Its corners, towers and gates.
- `roads` (splines) — The roads, side roads cut at the wall, and the wall's lines: what the Map Output's roads take.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `on` | boolean | true |  | Whether the village has walls. |
| `style` | choice | `wooden-fence` | `wooden-fence`, `spruce-fence`, `post-and-rail`, `hedge`, `flowering-hedge`, `low-stone-wall`, `dry-stone-wall`, `stone-and-fence`, `palisade`, `ramparts` | What runs round it. `wooden-fence`: An oak fence, a post of oak now and then, some with a lantern. `spruce-fence`: A spruce fence on spruce posts. `post-and-rail`: Rails of birch between stripped posts. `hedge`: A hedge of oak leaves two blocks high, an arch over its gates. `flowering-hedge`: A hedge of azalea, flowering here and there. `low-stone-wall`: A cobblestone wall a block high, mossy in places. `dry-stone-wall`: Stones piled one or two high. `stone-and-fence`: A course of cobblestone with a fence on it. `palisade`: Spruce logs three or four high, a beam over its gates. `ramparts`: The theme's ramparts: battlements, towers at the corners and along them. |
| `pack` | text | `` |  | The folder of the village pack: its schematics by type (house/, church/, farm/…) and its village.json. |
| `theme` | text | `` |  | Which theme of the pack, when it has several: the first one when left empty. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "out": { "type": "output.map", "inputs": { "height": "relief" } },
    "places": { "type": "points.scatter", "inputs": { "height": "relief" }, "params": { "spacing": 256 } },
    "relief": { "type": "noise.fbm", "params": { "height": 250, "scale": 1500 } },
    "streets": { "type": "village.roads", "inputs": { "height": "relief", "squares": "places" } },
    "walls": { "type": "village.walls", "inputs": { "roads": "streets" } }
  }
}
```

### Village plots — `village.plots`

Plots along both sides of a village's roads, each the size of building it would like and its door to the road. A plot every 9 to 15 blocks along each road, fewer as Density falls; nearer the squares first. Their sizes lean small or large as Small ↔ large asks, and a little larger away from the squares.

It makes geometry over the whole terrain at once, reading reliefs and masks at the simulations' resolution.

It reads the files its settings name, and follows them when they change.

Inputs:

- `squares` (points) — The squares, from Village site.
- `roads` (splines) — The roads, from Village roads (or Village walls).

Outputs:

- `plots` (footprints) — The plots: each its door, the way it faces, its size class.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `density` | number | 0.6 | 0.2 to 1 | From airy (0.2) to tight (1): how close the plots follow one another along the roads. |
| `size_bias` | number | 0 | -1 to 1 | From small houses (−1) to large houses and manors (+1). |
| `pack` | text | `` |  | The folder of the village pack: its schematics by type (house/, church/, farm/…) and its village.json. |
| `theme` | text | `` |  | Which theme of the pack, when it has several: the first one when left empty. |
| `seed` | seed | — | 0 to 4294967295 | Another seed gives another variation of the same settings. Left out, it comes from the node's name. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "out": { "type": "output.map", "inputs": { "height": "relief" } },
    "places": { "type": "points.scatter", "inputs": { "height": "relief" }, "params": { "spacing": 256 } },
    "plots": { "type": "village.plots", "inputs": { "roads": "streets", "squares": "places" } },
    "relief": { "type": "noise.fbm", "params": { "height": 250, "scale": 1500 } },
    "streets": { "type": "village.roads", "inputs": { "height": "relief", "squares": "places" } }
  }
}
```

### Village buildings — `village.buildings`

A building of the pack on each plot with room for it, its door to the road: houses, the church by the square, farms with their fields toward the edge. Each plot draws a type by the pack's shares and rules (the church and wells near the square, forges and taverns near the market, farms and towers toward the edge, manors on the heights), then a schematic of the size the plot likes, turned to face the road; a smaller one when it has not the room. Farms keep a field behind them, as often as Fields asks. It stops at Houses houses.

It makes geometry over the whole terrain at once, reading reliefs and masks at the simulations' resolution.

It reads the files its settings name, and follows them when they change.

Inputs:

- `height` (height, optional) — The relief the village stands on. Left empty: the sea level.
- `area` (mask, optional) — Where the village may go: over a half. At most 4096 × 4096 blocks. Left empty: the whole terrain.
- `squares` (points) — The squares, from Village site.
- `roads` (splines) — The roads, from Village roads (or Village walls).
- `walls` (splines, optional) — The walls' line, from Village walls: kept clear.
- `plots` (footprints) — The plots, from Village plots.

Outputs:

- `buildings` (instances) — The buildings: the pack's schematics placed and turned.
- `fields` (footprints) — The farms' fields, each its crop.
- `roads` (splines) — The roads, side roads cut after their last building (no door opens on the rest).

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `pack` | text | `` |  | The folder of the village pack: its schematics by type (house/, church/, farm/…) and its village.json. |
| `houses` | integer | 18 | 3 to 120 | How many houses (not counting wells, stalls, mills and towers): the village reaches as far as it needs to hold them. |
| `size_bias` | number | 0 | -1 to 1 | From small houses (−1) to large houses and manors (+1). |
| `fields` | number | 1 | 0 to 1 | The share of farms that keep a field. |
| `mill` | choice | `auto` | `auto`, `wind`, `water`, `none` | Which mill the village has. `auto`: A watermill when water is near and the pack has one, else a windmill. `wind`: A windmill, toward the edge. `water`: A watermill by the water. `none`: No mill. |
| `max_slope` | integer | 10 | 1 to 32 | The most a plot's land may rise across it, in blocks. |
| `margin` | integer | 1 | 0 to 8 | Blocks kept free round each building, at least. |
| `theme` | text | `` |  | Which theme of the pack, when it has several: the first one when left empty. |
| `seed` | seed | — | 0 to 4294967295 | Another seed gives another variation of the same settings. Left out, it comes from the node's name. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "buildings": { "type": "village.buildings", "inputs": { "plots": "lots", "roads": "streets", "squares": "places" } },
    "lots": { "type": "village.plots", "inputs": { "roads": "streets", "squares": "places" } },
    "out": { "type": "output.map", "inputs": { "height": "relief", "structures": "buildings" } },
    "places": { "type": "points.scatter", "inputs": { "height": "relief" }, "params": { "spacing": 256 } },
    "relief": { "type": "noise.fbm", "params": { "height": 250, "scale": 1500 } },
    "streets": { "type": "village.roads", "inputs": { "height": "relief", "squares": "places" } }
  }
}
```

### Village props — `village.props`

A village's props: market stalls round the square, lampposts and benches along the roads, hay and carts by the farms, fences round the fields. The pack's own props (its props/ and market/ folders) where it has them, Massif's simple ones otherwise; Props sets how many, the market's stalls standing whatever it says. Everything is the buildings and the props together, for the map.

It makes geometry over the whole terrain at once, reading reliefs and masks at the simulations' resolution.

It reads the files its settings name, and follows them when they change.

Inputs:

- `height` (height, optional) — The relief the village stands on. Left empty: the sea level.
- `area` (mask, optional) — Where the village may go: over a half. At most 4096 × 4096 blocks. Left empty: the whole terrain.
- `squares` (points) — The squares, from Village site.
- `roads` (splines) — The roads, from Village roads (or Village walls).
- `walls` (splines, optional) — The walls' line, from Village walls: kept clear.
- `buildings` (instances) — The buildings, from Village buildings.
- `fields` (footprints, optional) — The fields, from Village buildings.

Outputs:

- `props` (instances) — The props.
- `all` (instances) — The buildings, the props and the fields: what the Map Output's structures take.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `pack` | text | `` |  | The folder of the village pack: its schematics by type (house/, church/, farm/…) and its village.json. |
| `props` | number | 0.5 | 0 to 1 | How many props: none (0) to many (1). |
| `theme` | text | `` |  | Which theme of the pack, when it has several: the first one when left empty. |
| `seed` | seed | — | 0 to 4294967295 | Another seed gives another variation of the same settings. Left out, it comes from the node's name. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "houses": { "type": "village.buildings", "inputs": { "height": "relief", "plots": "lots", "roads": "streets", "squares": "places" } },
    "lots": { "type": "village.plots", "inputs": { "roads": "streets", "squares": "places" } },
    "out": { "type": "output.map", "inputs": { "height": "relief", "structures": "props" } },
    "places": { "type": "points.scatter", "inputs": { "height": "relief" }, "params": { "spacing": 256 } },
    "props": { "type": "village.props", "inputs": { "buildings": "houses", "roads": "streets", "squares": "places" } },
    "relief": { "type": "noise.fbm", "params": { "height": 250, "scale": 1500 } },
    "streets": { "type": "village.roads", "inputs": { "height": "relief", "squares": "places" } }
  }
}
```

### Village ground — `village.ground`

The relief levelled under a village's buildings and fields, and a mask of what the village takes. Each building's ground is cut or raised to its floor, sloping back to the land over four blocks; each field to its own. The mask is 1 under the buildings, the fields and the roads: keep the forest and the ores' veins out of the village with it.

Inputs:

- `height` (height, optional) — The relief. Left empty: the sea level.
- `buildings` (instances) — The buildings (and props), from the village.
- `fields` (footprints, optional) — The fields.
- `roads` (splines, optional) — The roads.

Outputs:

- `height` (height) — The relief levelled under the village.
- `mask` (mask) — 1 where the village stands.

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "ground": { "type": "village.ground", "inputs": { "buildings": "houses" } },
    "houses": { "type": "village.buildings", "inputs": { "height": "relief", "plots": "lots", "roads": "streets", "squares": "places" } },
    "lots": { "type": "village.plots", "inputs": { "roads": "streets", "squares": "places" } },
    "out": { "type": "output.map", "inputs": { "height": "ground" } },
    "places": { "type": "points.scatter", "inputs": { "height": "relief" }, "params": { "spacing": 256 } },
    "relief": { "type": "noise.fbm", "params": { "height": 250, "scale": 1500 } },
    "streets": { "type": "village.roads", "inputs": { "height": "relief", "squares": "places" } }
  }
}
```

## Volumes

### Relief to volume — `volume.from_height`

The ground of a relief as a volume: matter under its heights, air above. Where volumes start: caves are cut out of it, arches and overhangs added to it, and Volume to relief reads its top back. Its value is the height above the ground in blocks, negative under it.

Inputs:

- `height` (height) — The relief.

Outputs:

- `volume` (density) — Negative under the ground, the height above it elsewhere.

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "from_height": { "type": "volume.from_height", "inputs": { "height": "relief" } },
    "out": { "type": "output.map", "inputs": { "height": "relief", "volume.carve": "from_height" } },
    "relief": { "type": "noise.fbm", "params": { "height": 250, "scale": 1500 } }
  }
}
```

### Sphere — `volume.sphere`

A ball, or one round each point: a hall to carve, a boulder to add. A ball of Radius blocks at a place of the terrain and a height; given points, one at each instead (its radius times the point's scale, Depth blocks under it): chambers where Scatter put them. Its value is the distance to its surface, negative inside.

Inputs:

- `points` (points, optional) — Where the balls are. Left empty: one ball, at the settings' place.

Outputs:

- `volume` (density) — Negative inside the balls.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `x` | number | 0.5 | -1 to 2 fraction of the terrain's side | Where it is, west to east across the terrain. In fraction of the terrain's side. Usually 0 to 1. |
| `z` | number | 0.5 | -1 to 2 fraction of the terrain's side | Where it is, north to south across the terrain. In fraction of the terrain's side. Usually 0 to 1. |
| `y` | number | 20 | -2048 to 4096 blocks | The height of its middle, in blocks. Usually -64 to 320. |
| `radius` | number | 16 | 0 to 4096 blocks | In blocks. Usually 0 to 128. |
| `squash` | number | 1 | 0.1 to 4 | Its height against its width: under 1, a flat hall. |
| `depth` | number | 0 | 0 to 4096 blocks | Given points, how far under them the balls' middles lie. In blocks. Usually 0 to 256. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "out": { "type": "output.map", "inputs": { "height": "relief", "volume.carve": "sphere" } },
    "relief": { "type": "noise.fbm", "params": { "height": 250, "scale": 1500 } },
    "sphere": { "type": "volume.sphere" }
  }
}
```

### Box — `volume.box`

A box with rounded edges: a room, a gallery, a block of rock. A box of Width × Height × Length blocks at a place of the terrain and a height, turned about the vertical and rounded at its edges. Its value is the distance to its surface, negative inside.

Outputs:

- `volume` (density) — Negative inside the box.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `x` | number | 0.5 | -1 to 2 fraction of the terrain's side | Where its middle is, west to east across the terrain. In fraction of the terrain's side. Usually 0 to 1. |
| `z` | number | 0.5 | -1 to 2 fraction of the terrain's side | Where its middle is, north to south across the terrain. In fraction of the terrain's side. Usually 0 to 1. |
| `y` | number | 20 | -2048 to 4096 blocks | The height of its middle, in blocks. Usually -64 to 320. |
| `width` | number | 32 | 0 to 4096 blocks | West to east, in blocks. Usually 0 to 256. |
| `height` | number | 16 | 0 to 4096 blocks | Bottom to top, in blocks. Usually 0 to 256. |
| `length` | number | 32 | 0 to 4096 blocks | North to south, in blocks. Usually 0 to 256. |
| `turn` | number | 0 | -180 to 180 degrees | Turned about the vertical. In degrees. |
| `rounding` | number | 2 | 0 to 4096 blocks | The radius of its rounded edges. In blocks. Usually 0 to 32. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "box": { "type": "volume.box" },
    "out": { "type": "output.map", "inputs": { "height": "relief", "volume.carve": "box" } },
    "relief": { "type": "noise.fbm", "params": { "height": 250, "scale": 1500 } }
  }
}
```

### Tube — `volume.tube`

A tunnel along lines: a cave that follows a drawn path, a lava tube, a mine. A round tunnel of Radius blocks along each line (its width, when Radius is 0), Depth blocks under the line's points, wider and narrower along its length by Variation. Draw the path, or give the splines of a river or a road to dig under. Its value is the distance to the tunnel's wall, negative inside.

Inputs:

- `splines` (splines, optional) — The lines to follow. Left empty: the line drawn in Path.

Outputs:

- `volume` (density) — Negative inside the tunnels.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `path` | curve | none | each `x`, `z` | The line the tunnel follows, when no splines are given (x and z across the terrain). |
| `y` | number | 20 | -2048 to 4096 blocks | The height of the drawn path. In blocks. Usually -64 to 320. |
| `radius` | number | 4 | 0 to 4096 blocks | In blocks; 0: half each line's width. Usually 0 to 32. |
| `depth` | number | 0 | 0 to 4096 blocks | How far under the lines' points the tunnel runs. In blocks. Usually 0 to 256. |
| `variation` | number | 0.3 | 0 to 1 | How much wider and narrower it gets along its length. |
| `seed` | seed | — | 0 to 4294967295 | Another seed gives another variation of the same settings. Left out, it comes from the node's name. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "out": { "type": "output.map", "inputs": { "height": "relief", "volume.carve": "tube" } },
    "relief": { "type": "noise.fbm", "params": { "height": 250, "scale": 1500 } },
    "tube": { "type": "volume.tube", "params": { "path": [{ "x": 0.2, "z": 0.55 }, { "x": 0.5, "z": 0.45 }, { "x": 0.8, "z": 0.5 }] } }
  }
}
```

### Noise volume — `volume.noise`

Blobs of 3D noise taking a share of space: pockets, sponge-like rock, floating islands. 3D gradient noise of three octaves, cut so that the volume takes Share of space, in blobs about Scale blocks across (flatter as Squash falls). Limit it to a band of heights or a mask with Limit volume, carve it from the ground or add it above.

Outputs:

- `volume` (density) — Negative inside the blobs, about in blocks.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `share` | number | 0.2 | 0 to 1 | The share of space the blobs take. |
| `scale` | number | 48 | 0 to 4096 blocks | About how large a blob is, in blocks. Usually 0 to 256. |
| `squash` | number | 0.6 | 0.1 to 4 | Their height against their width. |
| `seed` | seed | — | 0 to 4294967295 | Another seed gives another variation of the same settings. Left out, it comes from the node's name. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "noise": { "type": "volume.noise" },
    "out": { "type": "output.map", "inputs": { "height": "relief", "volume.carve": "noise" } },
    "relief": { "type": "noise.fbm", "params": { "height": 250, "scale": 1500 } }
  }
}
```

### Cave noise — `volume.caves`

The game's noise caves as a volume — chambers with pillars, winding tunnels, noodles — between two heights, where a mask says. The noise the game carves its caves from since 1.18, as a volume to shape further: cheese chambers with pillars, spaghetti tunnels and noodles, each with its density and size, fading out toward Lowest and Highest and where the mask falls. Unlike a Cave layer, it has no roof, aquifers or decoration: combine it with other volumes (keep it under a relief with Combine ▸ Intersect, carve it along a band of strata with a mask), then carve it through the Map Output's Volume carve.

Inputs:

- `mask` (mask, optional) — Where the caves go, from 0 to 1. Left empty: 1 everywhere.

Outputs:

- `volume` (density) — Negative in the caves, about in blocks.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `cheese` | number | 0.5 | 0 to 1 | How much of the rock the chambers take. |
| `spaghetti` | number | 0.5 | 0 to 1 | How many winding tunnels. |
| `noodles` | number | 0.3 | 0 to 1 | How many thin tunnels. |
| `low` | number | -56 | -2048 to 4096 blocks | The lowest height the caves reach. In blocks. Usually -64 to 320. |
| `high` | number | 48 | -2048 to 4096 blocks | The highest height they reach. In blocks. Usually -64 to 320. |
| `size` | number | 1 | 0.5 to 2 | How large they are (1: as the game carves them). |
| `seed` | seed | — | 0 to 4294967295 | Another seed gives another variation of the same settings. Left out, it comes from the node's name. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "caves": { "type": "volume.caves" },
    "out": { "type": "output.map", "inputs": { "height": "relief", "volume.carve": "caves" } },
    "relief": { "type": "noise.fbm", "params": { "height": 250, "scale": 1500 } }
  }
}
```

### Combine volumes — `volume.combine`

Two volumes as one: their union, A without B, or what they share, with a smooth seam if asked. The booleans of solids. Union joins caves into one network; Subtract cuts B out of A (a cave out of the ground, a doorway out of a wall); Intersect keeps the part of A inside B (cave noise kept under a relief). Smoothness rounds where they meet, over that many blocks.

Inputs:

- `a` (density, optional) — A volume. Left empty: nothing.
- `b` (density, optional) — Another. Left empty: nothing.

Outputs:

- `volume` (density) — The two combined.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `op` | choice | `union` | `union`, `subtract`, `intersect` | How they combine. `union`: Both: what is inside either. `subtract`: A without B: B cut out of A. `intersect`: Only what is inside both. |
| `smooth` | number | 0 | 0 to 4096 blocks | Over how many blocks the seam is rounded. Usually 0 to 32. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "combine": { "type": "volume.combine" },
    "out": { "type": "output.map", "inputs": { "height": "relief", "volume.carve": "combine" } },
    "relief": { "type": "noise.fbm", "params": { "height": 250, "scale": 1500 } }
  }
}
```

### Shape volume — `volume.shape`

A volume grown or shrunk, and roughened: rock walls that are not smooth. Grow pushes the surface out by that many blocks (negative: in); Roughness moves it in and out by up to that many blocks with 3D noise of Roughness scale, so that carved halls and tunnels get the uneven walls of real caves.

Inputs:

- `volume` (density) — The volume.

Outputs:

- `volume` (density) — Grown and roughened.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `grow` | number | 0 | -64 to 64 blocks | Blocks the surface moves out (in when negative). |
| `roughness` | number | 2 | 0 to 4096 blocks | How far the noise moves the surface, in blocks. Usually 0 to 16. |
| `scale` | number | 8 | 0 to 4096 blocks | How large the bumps are, in blocks. Usually 0 to 64. |
| `seed` | seed | — | 0 to 4294967295 | Another seed gives another variation of the same settings. Left out, it comes from the node's name. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "ground": { "type": "volume.from_height", "inputs": { "height": "relief" } },
    "out": { "type": "output.map", "inputs": { "height": "relief", "volume.carve": "shape" } },
    "relief": { "type": "noise.fbm", "params": { "height": 250, "scale": 1500 } },
    "shape": { "type": "volume.shape", "inputs": { "volume": "ground" } }
  }
}
```

### Limit volume — `volume.limit`

A volume kept between two heights, or under the ground, and where a mask says. Keeps a volume between Lowest and Highest and where the mask is over one half, fading it out over Fade blocks; with Under the ground and a relief, also at least Cover blocks under its surface, so that caves never break through it.

Inputs:

- `volume` (density) — The volume.
- `mask` (mask, optional) — Where it is kept. Left empty: everywhere.
- `height` (height, optional) — The relief, for Under the ground. Left empty: none.

Outputs:

- `volume` (density) — The volume, kept where asked.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `low` | number | -64 | -2048 to 4096 blocks | The lowest height it reaches. In blocks. Usually -64 to 320. |
| `high` | number | 320 | -2048 to 4096 blocks | The highest height it reaches. In blocks. Usually -64 to 320. |
| `fade` | number | 4 | 0 to 4096 blocks | Over how many blocks it thins out at its limits. Usually 0 to 32. |
| `under` | boolean | true |  | Keeps it under the relief given. |
| `cover` | number | 4 | 0 to 4096 blocks | Blocks of ground kept over it, under the relief. Usually 0 to 64. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "ground": { "type": "volume.from_height", "inputs": { "height": "relief" } },
    "limit": { "type": "volume.limit", "inputs": { "volume": "ground" } },
    "out": { "type": "output.map", "inputs": { "height": "relief", "volume.carve": "limit" } },
    "relief": { "type": "noise.fbm", "params": { "height": 250, "scale": 1500 } }
  }
}
```

### Cave layer — `caves.layer`

Caves as the game carves them since 1.18 — chambers, tunnels, noodles, ravines, water and lava — where a mask says. One of the map's cave layers, made by the graph: its settings are the Caves tool's, and the mask says where its caves go (they thin out where it falls). Connect it to the Map Output's Caves; Apply adds or updates the layer, marked as the graph's, and the map carves it as it carves painted caves: sealed aquifers, lava under the lava floor, cave biomes and decoration on export. Several layers go through Merge cave layers.

Inputs:

- `mask` (mask, optional) — Where the caves go, from 0 to 1. Left empty: 1 everywhere.

Outputs:

- `layer` (layer) — The cave layer, and how much of each place it takes.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `amount` | number | 1 | 0 to 1 | How much of the masked area takes the layer: lower, its caves are fewer and patchier. A field connected to the input `@amount` drives it place by place. |
| `cheese` | boolean | true |  | Large chambers with pillars of rock standing in them (the game's cheese caves). |
| `cheese_density` | number | 0.5 | 0 to 1 | How much of the rock the chambers take. |
| `spaghetti` | boolean | true |  | Long winding tunnels (the game's spaghetti caves). |
| `spaghetti_density` | number | 0.5 | 0 to 1 | How many tunnels. |
| `noodles` | boolean | true |  | Thin tunnels, many of them. |
| `noodles_density` | number | 0.5 | 0 to 1 | How many noodles. |
| `ravines` | boolean | true |  | Long, deep, narrow cuts. |
| `ravines_density` | number | 0.3 | 0 to 1 | How many ravines. |
| `low` | number | -56 | -2048 to 4096 blocks | The lowest height the caves reach, in blocks. Usually -64 to 320. |
| `high` | number | 319 | -2048 to 4096 blocks | The highest height tunnels and noodles reach; the roof keeps them under the ground. In blocks. Usually -64 to 320. |
| `chambers_high` | number | 54 | -2048 to 4096 blocks | The highest height chambers and ravines reach: under the sea level, as in the game. In blocks. Usually -64 to 320. |
| `roof` | integer | 6 | 0 to 64 | Blocks of rock kept over the caves under the ground's surface, save at the entrances. |
| `openings` | number | 0.15 | 0 to 1 | The share of the land where caves may open to the sky. |
| `cheese_size` | number | 1 | 0.5 to 2 | How large the chambers are (1: as the game carves them). |
| `spaghetti_size` | number | 1 | 0.5 to 2 | How wide and long the tunnels are. |
| `noodles_size` | number | 1 | 0.5 to 2 | How wide the noodles are. |
| `ravines_size` | number | 1 | 0.5 to 2 | How long and deep the ravines are. |
| `aquifers` | boolean | true |  | Water and lava lakes at their own levels, sealed from each other by walls of rock as the game seals them. |
| `aquifer_density` | number | 0.35 | 0 to 1 | The share of the rock's zones holding water or lava. |
| `aquifer_size` | number | 1 | 0.5 to 2 | How large a zone of one level is (1: 48 blocks across, 24 high). |
| `water_high` | number | 56 | -2048 to 4096 blocks | The highest level an aquifer fills to. In blocks. Usually -64 to 320. |
| `lava` | number | 0.35 | 0 to 1 | The share of the flooded zones under Lava under holding lava rather than water. |
| `lava_under` | number | 0 | -2048 to 4096 blocks | Lakes of lava only below this height. In blocks. Usually -64 to 320. |
| `lava_floor` | boolean | true |  | Caves under Lava floor height are full of lava, whatever their zone (the game's lava under y −54). |
| `lava_floor_y` | number | -54 | -2048 to 4096 blocks | The top of the lava floor. In blocks. Usually -64 to 320. |
| `seed` | seed | — | 0 to 4294967295 | Another seed gives another variation of the same settings. Left out, it comes from the node's name. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "layer": { "type": "caves.layer" },
    "out": { "type": "output.map", "inputs": { "caves": "layer", "height": "relief" } },
    "relief": { "type": "noise.fbm", "params": { "height": 250, "scale": 1500 } }
  }
}
```

### Cave biome — `caves.biome`

A cave biome — lush, dripstone or deep dark — between two heights, where a mask says. Paints a cave biome in the rock between Lowest and Highest where the mask says, as the Cave biome tool paints it: the caves there take its decoration on export (moss and vines, dripstone, sculk). Connect it to the Map Output's Cave biomes; several go through Merge cave layers. Apply adds the biome where the graph gives it and leaves the rest of the map's cave biomes as they are.

Inputs:

- `mask` (mask, optional) — Where the biome goes, from 0 to 1. Left empty: 1 everywhere.

Outputs:

- `layer` (layer) — The biome, and how much of each place it takes.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `biome` | choice | `lush` | `lush`, `dripstone`, `deep_dark` | The cave biome. `lush`: Moss, azaleas, cave vines with glow berries, dripleaves over clay pools, spore blossoms. `dripstone`: Dripstone blocks, stalactites and stalagmites. `deep_dark`: Sculk, sensors, shriekers and catalysts (Minecraft 1.19 and later). |
| `low` | number | -64 | -2048 to 4096 blocks | The lowest height it fills, in blocks. Usually -64 to 320. |
| `high` | number | 40 | -2048 to 4096 blocks | The highest height it fills, in blocks. Usually -64 to 320. |
| `amount` | number | 1 | 0 to 1 | How much of the masked area takes it. A field connected to the input `@amount` drives it place by place. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "biome": { "type": "caves.biome" },
    "out": { "type": "output.map", "inputs": { "cave_biomes": "biome", "height": "relief" } },
    "relief": { "type": "noise.fbm", "params": { "height": 250, "scale": 1500 } }
  }
}
```

### Merge cave layers — `caves.merge`

Two cave layers (or cave biomes) as one: each place takes the stronger, or, for cave biomes, both at their own heights. For a map with several kinds of caves (shallow tunnels everywhere, deep chambers under the mountains) or several cave biomes: chain as many as needed into the Map Output's Caves or Cave biomes. A column holds one cave layer; with Both, cave biomes at different heights stack in a column.

Inputs:

- `a` (layer, optional) — A cave layer or cave biome.
- `b` (layer, optional) — Another, of the same kind.

Outputs:

- `layer` (layer) — The stronger of the two at each place.

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `mode` | choice | `stronger` | `stronger`, `both` | How two layers given at the same place merge. `stronger`: Each place takes the stronger of the two, B where they are equal: for cave layers, and for cave biomes at the same heights. `both`: Cave biomes only: where both are given, both are painted, each between its own heights (deep dark low down, lush caves above it). |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "merge": { "type": "caves.merge" },
    "out": { "type": "output.map", "inputs": { "caves": "merge", "height": "relief" } },
    "relief": { "type": "noise.fbm", "params": { "height": 250, "scale": 1500 } }
  }
}
```

## Outputs

### Map output — `output.map`

What the graph gives the map: its ground, water, surface, climate, life, underground and masks, each input optional. The graph's final node, like a material output in Blender: one node with an input for each thing the map can take. Connect only what the graph makes; an input left empty keeps what the map has (or takes the default it shows). A graph can keep several Map Outputs to compare variants: the one marked Active is used.

Inputs:

- `height` (height, optional) — The ground's height, in blocks. Left empty: the map's ground as it is.
- `water` (height, optional) — The surface of rivers and lakes, in blocks: where it lies above the ground, water fills up to it. Left empty: none: the world's sea only.
- `albedo` (color, optional) — The colour of the ground: the blocks nearest to it are chosen when Terrain is left empty. Left empty: none.
- `terrain` (terrain, optional) — The terrain (the block material) of each column. Left empty: from Albedo when it is given, else the map's terrains.
- `biome` (biome, optional) — The game's biome of each column. Left empty: from Temperature and Humidity when given, else the map's biomes.
- `temperature` (mask, optional) — The climate's temperature: 0 is −30 °C, 1 is +45 °C. Left empty: none.
- `humidity` (mask, optional) — How wet the climate is, from 0 to 1. Left empty: none.
- `level.1.top` (height, optional) — The top of a level of rock over the ground (a Solid level of its own): floating islands, overhangs, arches. Left empty: none.
- `level.1.bottom` (height, optional) — The underside of that level: where it is above the ground, the void lies between. Left empty: none.
- `level.1.color` (color, optional) — The colour of that level's rock: the blocks nearest to it are chosen, from the Albedo palette. Left empty: the colour of the rock.
- `level.2.top` (height, optional) — The top of a level of rock over the ground (a Solid level of its own): floating islands, overhangs, arches. Left empty: none.
- `level.2.bottom` (height, optional) — The underside of that level: where it is above the ground, the void lies between. Left empty: none.
- `level.2.color` (color, optional) — The colour of that level's rock: the blocks nearest to it are chosen, from the Albedo palette. Left empty: the colour of the rock.
- `level.3.top` (height, optional) — The top of a level of rock over the ground (a Solid level of its own): floating islands, overhangs, arches. Left empty: none.
- `level.3.bottom` (height, optional) — The underside of that level: where it is above the ground, the void lies between. Left empty: none.
- `level.3.color` (color, optional) — The colour of that level's rock: the blocks nearest to it are chosen, from the Albedo palette. Left empty: the colour of the rock.
- `level.4.top` (height, optional) — The top of a level of rock over the ground (a Solid level of its own): floating islands, overhangs, arches. Left empty: none.
- `level.4.bottom` (height, optional) — The underside of that level: where it is above the ground, the void lies between. Left empty: none.
- `level.4.color` (color, optional) — The colour of that level's rock: the blocks nearest to it are chosen, from the Albedo palette. Left empty: the colour of the rock.
- `cover` (layer, optional) — The ground cover layer of each column (grass, flowers, moss…), by its number among the world's covers, and how strongly. Left empty: the map's.
- `objects` (layer, optional) — The object layer of each column (trees, rocks…), and how dense. Left empty: the map's.
- `understory` (layer, optional) — The understory layer of each column (bushes, ferns…), and how dense. Left empty: the map's.
- `pockets` (layer, optional) — The pocket layer of each column (ores, clay, gravel in the rock), and how strongly. Left empty: the map's.
- `caves` (layer, optional) — The caves of each column, from Cave Layer nodes: its first plane is the key of a Cave Layer node's settings (not a number among the map's layers), its second how much of the area the layer takes. Apply makes each node's layer one of the map's, marked as the graph's. Left empty: the map's.
- `cave_biomes` (layer, optional) — Cave biomes (lush, dripstone, deep dark) between their heights, from Cave Biome nodes: its first plane is the key of a Cave Biome node's settings (or of two merged with Both), its second how much of the area takes them. Left empty: the map's.
- `annotations` (layer, optional) — The annotation colour of each column. Left empty: the map's.
- `structures` (instances, optional) — Buildings, villages and the game's structures to place, each built on the generated ground; a re-Apply replaces those the graph placed, never those placed by hand. Left empty: none.
- `roads` (splines, optional) — Roads and paths laid along their lines, with their width and kind. Left empty: none.
- `volume.carve` (density, optional) — A volume cut out of the ground: where its value (about the distance to its surface, in blocks) is negative, air, written into the map's sculpted blocks. Left empty: none.
- `volume.add` (density, optional) — A volume of rock added: where its value (about the distance to its surface, in blocks) is negative, rock of the column's terrain (arches, overhangs, floating rock), written into the map's sculpted blocks. Left empty: none.
- `mask.flow` (mask, optional) — Where water runs, written to the map as the mask “flow”. Left empty: none.
- `mask.wear` (mask, optional) — Where the ground was worn away, written as the mask “wear”. Left empty: none.
- `mask.deposition` (mask, optional) — Where sediment settled, written as the mask “deposition”. Left empty: none.
- `mask.snow` (mask, optional) — Where snow lies, written as the mask “snow”. Left empty: none.
- `mask.<name>` (mask, optional) — Any other mask, written to the map under its name (mask.moss writes “moss”).

| Setting | Kind | Default | Range | What it does |
|---|---|---|---|---|
| `active` | boolean | true |  | Whether the map takes this output, when the graph has several. |
| `albedo_palette` | choice | `natural` | `natural`, `rock`, `desert`, `snowy`, `every` | The blocks Albedo and the levels' colours turn into. `natural`: Soils, stone, sand, snow: the blocks of a natural landscape. `rock`: Stones only. `desert`: Sands, sandstones, terracottas. `snowy`: Snow, ice, pale stone. `every`: Every full block of the game. |
| `paint_biomes` | boolean | false |  | Paints the ground to suit the biomes: sand in the deserts, snow in the tundra. |

```json
{
  "terrain": { "seed": 0, "size": 4096 },
  "nodes": {
    "colours": { "type": "color.gradient", "inputs": { "in": "relief" } },
    "out": { "type": "output.map", "inputs": { "albedo": "colours", "height": "relief", "mask.rock": "slopes" } },
    "relief": { "type": "noise.fbm", "params": { "height": 250, "scale": 1500 } },
    "slopes": { "type": "mask.slope", "inputs": { "height": "relief" } }
  }
}
```
