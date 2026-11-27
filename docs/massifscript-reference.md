<!-- Written by massif-terrain (mscript::reference); do not edit by hand: `MASSIF_WRITE_DOCS=1 cargo test -p massif-terrain --lib mscript` writes it again. -->

# MassifScript

MassifScript is the text form of Massif's terrain node graph. Running a script builds exactly the nodes, links, settings, groups (frames), composites, exposed settings and outputs the visual editor makes: the result opens as an editable graph. Loops, conditions and constants only decide what is built (they run once, when the script is applied).

## Grammar

- A script is `project "Name" { … }` holding statements. `seed = 4268;` sets the terrain's seed; `size = 8192;` its side in blocks (the map's when left out); `sea_level = 62;`.
- Comments: `// to the end of the line` and `/* between */`.
- A node: `name = NodeType(input, other_input, setting: value, …);`. Inputs come first, in the order the catalogue gives, or named (`mask: m`). Settings are always named. `name` becomes the node's id: other nodes use it as an input (`eroded`, or `eroded.flow` for its output `flow`). Settings left out take their defaults.
- Nodes can be written inside an argument: `forest = Blend(a, b, mask: InvertMask(slope));`.
- Values: numbers with arithmetic (`2400 * 1.5`, `-3`, `(a + b) / 2`, `^`, `%`), functions `min max abs sqrt floor ceil round pow clamp lerp sin cos`, `pi`; texts `"ridged"`; booleans `true`/`false`; colours `#347a52`; lists `[1, 2]`; objects `{ x: 0.3, z: 0.6 }` (for points and curves, positions are fractions of the terrain from 0 to 1).
- A setting marked * in the catalogue can follow a field: give it a node (`height: mask_node`).
- `const k = 3;` and `let x = 1;` (`x = x + 1;` changes a `let`). `if cond { … } else { … }`, `for i in 0..4 { … }`, `for v in [100, 300] { … }`. In a loop, name nodes with an index: `peak[i] = …;` makes `peak_0`, `peak_1`…, used as `peak[i - 1]`.
- `group "Title" { color = #347a52; description = "…"; … }` frames the nodes written inside; groups nest.
- A node may end with attributes: `x = Blur(a) { name: "Soft hills"; note: "…"; bypass: true; };`.
- Outputs of the map: `output height = eroded;` and likewise `water`, `albedo`, `terrain` (block materials), `biome`, `temperature`, `humidity`, `cover`, `objects`, `understory`, `pockets`, `caves`, `cave_biomes`, `annotations`, `structures`, `roads`, `volume.carve`, `volume.add`, and any mask as `mask.<name>`. They are the Map Output node's inputs.
- A component is a node made of nodes: `component Name(input: height, mask_in: mask?, size: float = 800) { … output result; }`. Parameters typed as a socket (height, mask, color, vector, terrain, biome, layer, points, splines, footprints, instances, density; `?` optional) are its inputs; those typed float, int, bool, seed, choice, string or value are settings it exposes, which its nodes take as values (`height: size`). `expose size { label: "Size"; min: 0; max: 2000; unit: "blocks"; }` says how to show one. `label = "…"; category = "…"; summary = "…";` describe it. Use it like any node: `m = Name(relief, size: 900);`, `m.result` for one of its outputs.
- `import "lib.mscript";` reads another script (its components, constants and nodes).
- A type can also be called by its id: `noise.fbm(…)`, `"massif:mountain"(…)`, and a call makes the latest version of a type, an older one with `@`: `Erosion@1(…)`, `Transform@1(…)`.

## Sockets

Each input and output carries a type. A height and a mask feed each other as they are; a colour feeds a number by its brightness, a number feeds a colour as a grey; every other type feeds only its own. Heights are in blocks (1 block = 1 m; Minecraft builds from y −64 to 320, the sea at 62); masks go from 0 to 1.

- `height` feeds height, mask, color
- `mask` feeds height, mask, color
- `color` feeds height, mask, color
- `vector` feeds vector
- `terrain` feeds terrain
- `biome` feeds biome
- `layer` feeds layer
- `points` feeds points
- `splines` feeds splines
- `footprints` feeds footprints
- `instances` feeds instances
- `density` feeds density

## Nodes

Each line: `Name(inputs) -> outputs — what it makes. Settings: name: kind range unit = default`. `?` marks an optional input; `*` a setting a field can drive.

### Inputs

- MapInput() -> height: height, water: mask, level: height, terrain: terrain, biome: biome, color: color — The map as it is: its ground, its water, its terrains and biomes, read again whenever the map changes.
- Heightmap() -> height: height, mask: mask — A heightmap image turned into relief: black at Low, white at High, laid over a rectangle of the terrain. Settings: file: file (.png, .tif, .tiff, .jpg, .jpeg, .bmp, .raw, .r16, .exr); low: float -2048..4096 blocks = 40; high: float -2048..4096 blocks = 200; x: float -4..4 of the side = 0; z: float -4..4 of the side = 0; width: float 0.001..16 of the side = 1; depth: float 0.001..16 of the side = 1; rotation: float -180..180° = 0; fit: "stretch"|"contain"|"cover" = "stretch"; edge: float 0..16000 blocks = 0; tile: bool = false; invert: bool = false.
- Image() -> mask: mask, color: color — An image read as a mask (one of its channels, over a range) and as colours: a temperature or rainfall map, a painted mask, a satellite colour map. Settings: file: file (.png, .tif, .tiff, .jpg, .jpeg, .bmp, .exr); channel: "luminance"|"red"|"green"|"blue"|"alpha" = "luminance"; low: float -1000..1000 = 0; high: float -1000..1000 = 1; invert: bool = false; x: float -4..4 of the side = 0; z: float -4..4 of the side = 0; width: float 0.001..16 of the side = 1; depth: float 0.001..16 of the side = 1; rotation: float -180..180° = 0; fit: "stretch"|"contain"|"cover" = "stretch"; tile: bool = false; outside: float 0..1 = 0.

### Primitives

- Constant() -> height: height — The same height everywhere. Settings: value: float -2048..4096 blocks = 64.
- FractalNoise() -> height: height — Fractal noise: relief at every scale, from its largest features down to fine detail. Settings: style: "fbm"|"ridged"|"billow" = "fbm"; scale: float 2..65536 blocks = 1000; height*: float 0..4096 blocks = 200; base*: float -2048..4096 blocks = 40; octaves: int 1..16 = 8; roughness: float 0.05..0.95 = 0.5; lacunarity: float 1.2..4 = 2; offset_x: float -1000000..1000000 blocks = 0; offset_z: float -1000000..1000000 blocks = 0; seed: seed (from the node's id when left out).
- ErodedNoise() -> height: height — Fractal noise whose fine detail fades on slopes and gathers on heights: smooth flanks, branching valleys, the look of an eroded relief without simulating it. Settings: scale: float 2..65536 blocks = 1200; height*: float 0..4096 blocks = 120; base*: float -2048..4096 blocks = 62; erosion: float 0..6 = 1.5; roughness: float 0.05..0.95 = 0.5; seed: seed (from the node's id when left out).
- CellNoise() -> height: height — Cellular (Voronoi) noise: points scattered one per cell, and a value from where a sample lies among them — pits, pillows, ridges along the edges, or a mosaic. Settings: mode: "f1"|"f2"|"edges"|"cells" = "edges"; scale: float 2..65536 blocks = 300; jitter: float 0..1 = 0.9; height*: float -4096..4096 blocks = 1; base*: float -2048..4096 blocks = 0; seed: seed (from the node's id when left out).
- Gradient() -> value: mask — A value rising from one point to another, straight or around the first: tilt a relief, fade a mask towards a coast, raise a dome. Settings: kind: "linear"|"radial" = "linear"; ends: points [{ x, z }, …] (up to 2; x, z from 0 to 1); profile: profile [[x, y], …] = [[0.0,0.0],[1.0,1.0]]; height: float -4096..4096 blocks = 1; base: float -2048..4096 blocks = 0.
- Shape() -> mask: mask — A geometric shape as a mask: a disc, a square, a polygon or a star, its edge as soft as wanted. Settings: shape: "circle"|"square"|"polygon"|"star" = "circle"; center: points [{ x, z }, …] (up to 1; x, z from 0 to 1); radius: float 1..65536 blocks = 1000; softness: float 0..16000 blocks = 200; sides: int 3..32 = 6; angle: float -180..180° = 0.

### Shapes

- SingleMountain() -> height: height, mask: mask — One mountain in the middle of the terrain, a summit with ridges running down its flanks: light, fast, and a base for erosion. Settings: scale: float 32..65536 blocks = 1500; height*: float 0..4096 blocks = 240; style: "basic"|"eroded"|"old"|"alpine"|"strata" = "basic"; bulk: "low"|"medium"|"high" = "medium"; reduce_details: bool = false; seed: seed (from the node's id when left out).
- AlpineMountains(area?: mask) -> height: height, mask: mask — Young, sharp mountains: pyramidal summits, knife-edge ridges and deep valleys, like the Alps or the Himalaya. Settings: height*: float 0..4096 blocks = 220; base: float -2048..4096 blocks = 70; peak_spacing: float 64..16000 blocks = 500; coverage: float 0..1 = 0.55; profile: profile [[x, y], …] = [[0.0,0.0],[0.3,0.08],[0.65,0.5],[1.0,1.0]]; sharpness: float 0..1 = 0.8; roughness: float 0..1 = 0.55; detail: float 16..8000 blocks = 160; warp: float 0..1 = 0.35; seed: seed (from the node's id when left out).
- RoundedMountains(area?: mask) -> height: height, mask: mask — Old, worn mountains: rounded summits, broad shoulders and gentle valleys, like the Appalachians or the Vosges. Settings: height*: float 0..4096 blocks = 120; base: float -2048..4096 blocks = 70; peak_spacing: float 64..16000 blocks = 800; coverage: float 0..1 = 0.6; profile: profile [[x, y], …] = [[0.0,0.0],[0.3,0.3],[0.7,0.82],[1.0,1.0]]; sharpness: float 0..1 = 0.12; roughness: float 0..1 = 0.4; detail: float 16..8000 blocks = 260; warp: float 0..1 = 0.35; seed: seed (from the node's id when left out).
- MountainRange() -> height: height, mask: mask — A range of mountains along a line drawn on the terrain: a crest of summits and saddles, with side ridges running down to its feet. Settings: spine: curve [{ x, z, height, width }, …] (x, z from 0 to 1); height*: float 0..4096 blocks = 230; base: float -2048..4096 blocks = 70; width: float 64..32000 blocks = 1800; profile: profile [[x, y], …] = [[0.0,0.0],[0.25,0.14],[0.6,0.62],[1.0,1.0]]; peak_spacing: float 32..16000 blocks = 450; saddles: float 0..1 = 0.45; spurs: float 0..1 = 0.6; spur_spacing: float 32..8000 blocks = 260; asymmetry: float -1..1 = 0; sharpness: float 0..1 = 0.75; roughness: float 0..1 = 0.5; detail: float 16..8000 blocks = 150; warp: float 0..1 = 0.35; seed: seed (from the node's id when left out).
- Peaks() -> height: height, mask: mask — Summits placed by hand, each with its height and reach: horns with ridges (arêtes) running down from the top. Settings: peaks: points [{ x, z, height, radius }, …] (up to 64; x, z from 0 to 1); base: float -2048..4096 blocks = 70; profile: profile [[x, y], …] = [[0.0,0.0],[0.25,0.14],[0.6,0.62],[1.0,1.0]]; aretes: int 2..8 = 4; arete_strength: float 0..1 = 0.7; sharpness: float 0..1 = 0.8; roughness: float 0..1 = 0.5; detail: float 16..8000 blocks = 120; warp: float 0..1 = 0.35; seed: seed (from the node's id when left out).
- Volcano() -> height: height, mask: mask — Volcanoes placed by hand: steep stratovolcanoes, broad shields, calderas or cinder cones, with a crater, gullies down their flanks and smaller cones on them. Settings: volcanoes: points [{ x, z, height, radius }, …] (up to 16; x, z from 0 to 1); kind: "strato"|"shield"|"caldera"|"cinder" = "strato"; base: float -2048..4096 blocks = 70; crater: float 0..0.6 = 0.1; crater_depth: float 0..1024 blocks = 35; gullies: float 0..1 = 0.5; gully_count: int 3..96 = 14; cones: int 0..16 = 3; roughness: float 0..1 = 0.4; seed: seed (from the node's id when left out).
- Mesas(area?: mask) -> height: height, mask: mask — Flat-topped tablelands cut by cliffs: mesas, buttes standing alone, stepped walls of rock layers, talus at their feet. Settings: height*: float 0..2048 blocks = 60; base: float -2048..4096 blocks = 70; scale: float 32..32000 blocks = 900; coverage: float 0..1 = 0.45; cliff: float 0..1 = 0.85; layers: int 0..16 = 3; talus: float 0..1 = 0.5; dissection: float 0..1 = 0.35; roughness: float 0..1 = 0.4; seed: seed (from the node's id when left out).
- Canyon(height?: height) -> height: height, mask: mask — A canyon cut into the relief: along a line drawn on the terrain, or a network of them winding across it, with a flat floor and stepped walls. Settings: path: curve [{ x, z, depth, width }, …] (x, z from 0 to 1); depth*: float 0..2048 blocks = 55; width: float 4..16000 blocks = 260; floor: float 0..0.95 = 0.22; steps: int 0..16 = 3; meander: float 0..1 = 0.4; scale: float 64..32000 blocks = 1600; branches: float 0..1 = 0.4; base: float -2048..4096 blocks = 140; seed: seed (from the node's id when left out).
- GlacialValley(height?: height) -> height: height, mask: mask — A U-shaped valley a glacier carved, along a line drawn on the terrain: a wide flat floor gently descending between steep walls. Settings: path: curve [{ x, z, width }, …] (x, z from 0 to 1); floor_height*: float -2048..4096 blocks = 75; drop: float -2048..2048 blocks = 20; width: float 16..16000 blocks = 700; floor: float 0..0.95 = 0.35; steepness: float 0..1 = 0.75; meander: float 0..1 = 0.2; base: float -2048..4096 blocks = 140; seed: seed (from the node's id when left out).
- Fault(height?: height) -> height: height, mask: mask — A fault along a line drawn on the terrain: the ground on one side lifted, a scarp between the two. Settings: path: curve [{ x, z }, …] (x, z from 0 to 1); throw*: float -1024..1024 blocks = 30; width: float 1..4000 blocks = 40; ragged: float 0..1 = 0.4; base: float -2048..4096 blocks = 140; seed: seed (from the node's id when left out).
- Dunes(sand?: mask) -> height: height, mask: mask — Sand shaped by the wind: transverse crests, linear ridges, crescent barchans or star dunes, with ripples over them. Settings: kind: "transverse"|"linear"|"barchan"|"star" = "transverse"; direction: float -180..180° = 20; wavelength: float 8..8000 blocks = 140; height*: float 0..512 blocks = 14; base: float -2048..4096 blocks = 70; irregularity: float 0..1 = 0.5; ripples: float 0..1 = 0.3; seed: seed (from the node's id when left out).
- Karst() -> height: height, mask: mask — Limestone dissolved by water: steep towers on a plain, crowded cones around hollows, or a plain pitted with sinkholes. Settings: kind: "towers"|"cockpit"|"sinkholes" = "towers"; spacing: float 8..8000 blocks = 160; height*: float 0..1024 blocks = 70; base: float -2048..4096 blocks = 70; steepness: float 0..1 = 0.85; coverage: float 0..1 = 0.6; roughness: float 0..1 = 0.4; seed: seed (from the node's id when left out).
- Craters(height?: height) -> height: height, mask: mask — Impact craters: a bowl with a raised rim and its ejecta, a central peak in the large ones; placed by hand or scattered, many small and few large. Settings: craters: points [{ x, z, radius, depth }, …] (up to 64; x, z from 0 to 1); count: int 0..2000 = 30; smallest: float 4..8000 blocks = 20; largest: float 4..16000 blocks = 400; depth_ratio: float 0..1 = 0.18; rim: float 0..1 = 0.35; peak_size: float 8..16000 blocks = 250; base: float -2048..4096 blocks = 70; seed: seed (from the node's id when left out).
- Island() -> height: height, land: mask — An island rising from the sea: a ragged coast, a beach, a shallow shelf around it, land rising inland. Settings: center: points [{ x, z }, …] (up to 1; x, z from 0 to 1); radius: float 16..32000 blocks = 1400; height*: float 0..4096 blocks = 70; elongation: float 0..0.9 = 0.3; angle: float -180..180° = 30; profile: profile [[x, y], …] = [[0.0,0.0],[0.35,0.18],[0.75,0.7],[1.0,1.0]]; beach: float 0..2000 blocks = 40; shelf: float 0..16000 blocks = 400; depth: float 0..1024 blocks = 40; roughness: float 0..1 = 0.5; outline_scale: float 16..16000 blocks = 600; hills: float 0..1 = 0.35; seed: seed (from the node's id when left out).
- Archipelago() -> height: height, land: mask — A scatter of islands in the sea, many small and a few large, over the terrain or along a line. Settings: count: int 0..400 = 14; smallest: float 16..16000 blocks = 70; largest: float 16..32000 blocks = 700; height*: float 0..4096 blocks = 60; chain: curve [{ x, z }, …] (x, z from 0 to 1); spread: float 0..32000 blocks = 1500; islands: points [{ x, z, radius, height }, …] (up to 128; x, z from 0 to 1); profile: profile [[x, y], …] = [[0.0,0.0],[0.35,0.18],[0.75,0.7],[1.0,1.0]]; beach: float 0..2000 blocks = 40; shelf: float 0..16000 blocks = 400; depth: float 0..1024 blocks = 40; roughness: float 0..1 = 0.5; hills: float 0..1 = 0.35; seed: seed (from the node's id when left out).
- Coastline(shape?: mask) -> height: height, land: mask — Land and sea across the terrain: continents and seas, a ragged coast with capes and bays, beaches, a shelf, fjords if wanted. Settings: land: float 0..1 = 0.5; scale: float 64..65536 blocks = 5000; height*: float 0..4096 blocks = 80; inland: float 16..32000 blocks = 1500; outline: closed curve [{ x, z }, …] (x, z from 0 to 1); profile: profile [[x, y], …] = [[0.0,0.0],[0.35,0.18],[0.75,0.7],[1.0,1.0]]; beach: float 0..2000 blocks = 40; shelf: float 0..16000 blocks = 400; depth: float 0..1024 blocks = 40; fjords: float 0..1 = 0; fjord_size: float 16..8000 blocks = 350; roughness: float 0..1 = 0.5; hills: float 0..1 = 0.35; seed: seed (from the node's id when left out).

### Adjust

- Levels(in: height) -> out: height — Maps one range of heights onto another, along a curve. Settings: in_low: float -4096..4096 blocks = 0; in_high: float -4096..4096 blocks = 256; out_low: float -4096..4096 blocks = 0; out_high: float -4096..4096 blocks = 256; gamma: float 0.05..20 = 1.
- Curve(in: height) -> out: height — Reshapes a relief along a curve drawn from its low to its high values: flatten the valleys, sharpen the peaks, lift a plateau. Settings: curve: profile [[x, y], …] = [[0.0,0.0],[0.5,0.35],[1.0,1.0]]; in_low: float -4096..4096 blocks = 62; in_high: float -4096..4096 blocks = 300; out_low: float -4096..4096 blocks = 62; out_high: float -4096..4096 blocks = 300; range: "given"|"own" = "given".
- Terraces(in: height) -> out: height — Steps cut into a relief between two heights: flat ledges joined by risers, as rock beds or rice terraces make them. Settings: steps: int 1..256 = 8; sharpness: float 0..1 = 0.7; low: float -4096..4096 blocks = 62; high: float -4096..4096 blocks = 300; strength: float 0..1 = 1; range: "given"|"own" = "given".
- Strata(in: height) -> out: height — Layers of rock of uneven thickness cut into a relief: ledges on the slopes, as sedimentary rock weathers. Settings: layers: int 1..200 = 14; strength: float 0..1 = 0.7; tilt: float -60..60° = 6; seed: seed (from the node's id when left out).
- Clamp(in: height) -> out: height — A relief held between two heights: nothing under Min, nothing over Max. Settings: min: float -4096..4096 blocks = -64; max: float -4096..4096 blocks = 320.
- Fold(in: height) -> out: height — Mirrors what rises over a level back down: ridges folded into crests and troughs. Settings: level: float -4096..4096 blocks = 120; strength: float 0..1 = 1.
- Invert(in: height) -> out: height — Turns a relief upside down within its own range: its highest point where its lowest was.
- Normalize(in: height) -> out: height — Stretches a relief so that its lowest point lies at Low and its highest at High. Settings: low: float -4096..4096 blocks = 0; high: float -4096..4096 blocks = 1.
- Blur(in: height) -> out: height — Smooths a relief: the detail smaller than Radius goes, the larger shapes stay. Settings: radius: float 0..8192 blocks = 32.
- Sharpen(in: height) -> out: height — Brings out a relief's detail: what stands above its surroundings rises, what lies below sinks. Settings: radius: float 1..1024 blocks = 24; amount: float 0..8 = 0.8.
- Warp(in: height) -> out: height — Pushes a relief around by a noise: straight lines wander, round shapes twist, everything looks less made. Settings: amount: float 0..4096 blocks = 60; scale: float 4..65536 blocks = 400; octaves: int 1..8 = 3; seed: seed (from the node's id when left out).
- Transform(in: height) -> out: height — Moves, turns, scales and deforms a relief over the terrain, at full detail. Settings: offset_x: float -1..1 of the side = 0; offset_z: float -1..1 of the side = 0; rotate: float -360..360° = 0; scale: float 0.05..20 = 1; stretch: float 0.1..10 = 1; deform: float 0..2048 blocks = 0; deform_size: float 16..65536 blocks = 600; seed: seed (from the node's id when left out).
- Swirl(in: height) -> out: height — Twists a relief around a point, most at the middle, not at all at Radius. (simulation) Settings: angle: float -1080..1080° = 120; radius: float 0.01..2 of the side = 0.5; x: float -1..2 of the side = 0.5; z: float -1..2 of the side = 0.5.

### Simulate

- Erosion(height: height, hardness?: mask, rainfall?: mask) -> height: height, flow: mask, wear: mask, deposition: mask, sediment: mask — Rivers cut valleys that branch up into every slope, leaving sharp ridges between them; the slopes creep and slide, and what the water carries settles on valley floors and in fans at their feet. (simulation) Settings: duration: float 0..20 = 1; downcutting: float 0..1 = 0.35; scale: float 4..4096 blocks = 64; removal: float 0..1 = 0.2; softness: float 0..1 = 0.5; base: float -64..512 blocks = -64; talus: float 10..80° = 42; steps: int 4..400 = 24; seed: seed (from the node's id when left out).
- ParticleErosion(height: height, hardness?: mask, rainfall?: mask) -> height: height, flow: mask, wear: mask, deposition: mask — Raindrops run down the slopes, wearing gullies and rills where they speed up and laying soil where they slow down. (simulation) Settings: rain: float 0.05..16 = 1; erosion: float 0..1 = 0.3; deposition: float 0..1 = 0.3; capacity: float 0..32 = 4; evaporation: float 0..0.5 = 0.02; radius: float 1..64 blocks = 6; inertia: float 0..0.95 = 0.05; seed: seed (from the node's id when left out).
- ThermalErosion(height: height, hardness?: mask) -> height: height, talus: mask — Loose rock slides down slopes steeper than its angle of repose, and gathers at their feet as scree. (simulation) Settings: angle: float 5..80° = 35; iterations: int 1..1000 = 50; strength: float 0.01..1 = 0.5; strata: float 0..128 blocks = 0; strata_hardness: float 0..1 = 0.85; strata_share: float 0.05..0.95 = 0.4; strata_warp: float 0..2 = 0.3; seed: seed (from the node's id when left out).
- Sediment(height: height, source?: mask) -> height: height, deposition: mask, wear: mask, sediment: mask — Loose rock and soil shed by steep slopes run down with the water and settle where it slows: fans at the feet of mountains, filled valleys and basins, deltas at the sea. (simulation) Settings: amount: float 0..200 blocks = 6; deposition: float 0..1 = 0.7; iterations: int 10..2000 = 150; creep: float 0..1 = 0.05; seed: seed (from the node's id when left out).
- Glaciers(height: height, snow?: mask) -> height: height, ice: mask, wear: mask, deposition: mask — Ice gathers above the snow line, flows down the valleys and scours them into U-shapes, with cirques at their heads, fjords below the sea and moraines at their ends. (simulation) Settings: ela: float -64..320 blocks = 170; accumulation: float 0..10 = 1; strength: float 0..10 = 1; iterations: int 10..10000 = 800; moraines: float 0..32 blocks = 3; flow: float 0..10 = 1.
- WindErosion(height: height, sand?: mask, wind?: vector) -> height: height, sand: mask, wear: mask — The wind blows loose sand into dunes, ridged where sand is plenty and crescents where it is scarce, and wears bare rock that faces it. (simulation) Settings: direction: float 0..360° = 0; sand_depth: float 0..64 blocks = 6; hop: float 1..128 blocks = 12; iterations: int 10..10000 = 400; strength: float 0..5 = 1; abrasion: float 0..5 = 0; seed: seed (from the node's id when left out).
- Flow(height: height, rainfall?: mask) -> flow: mask — Where the rain runs off, as a mask: bright in the rivers, fading up every rill. (simulation) Settings: spread: float 0..1 = 0.5; seed: seed (from the node's id when left out).
- Rivers(height: height, water?: height) -> height: height, rivers: mask, water: height — Rivers where enough of the land drains, their beds and banks dug into the relief, wider and deeper downstream, holding lakes in deep hollows. (simulation) Settings: threshold: float 0..1 = 0.55; width: float 1..256 blocks = 14; depth: float 0.5..64 blocks = 5; banks: float 0..8 = 1; meander: float 0..3 = 0.5; lakes: float 0..1 = 0.5; sea: float -64..320 blocks = 62; seed: seed (from the node's id when left out).
- Lakes(height: height) -> height: height, lakes: mask, water: height — Every hollow fills up to where it spills: with water, or with ground. (simulation) Settings: amount: float 0..1 = 0; sea: float -64..320 blocks = 62.
- Sea(height: height) -> height: height, water: height, shore: mask — The sea up to a level, where the terrain's edge lets it in, and the shore around it. (simulation) Settings: level: float -64..320 blocks = 62; shore: float 0.5..64 blocks = 4; connected: bool = true.

### Climate

- Temperature(height?: height) -> temperature: mask, celsius: height — How warm each place is: warmer toward one side of the terrain, colder with height, milder by the sea. Settings: toward: float -360..360° = 90; warm*: float -30..45 °C = 28; cold*: float -30..45 °C = -8; lapse: float 0..30 °C = 5; sea_moderation: float 0..1 = 0.3; variation: float 0..20 °C = 2; variation_scale: float 32..65536 blocks = 3000; seed: seed (from the node's id when left out).
- Wind(height?: height) -> wind: vector — The prevailing wind over the terrain, bent around high ground. Settings: direction: float -360..360° = 0; strength: float 0..10 = 1; deflection: float 0..1 = 0.5.
- Rainfall(height: height) -> rain: mask — Where rain falls: air carries moisture from the sea downwind and loses it as it rises, so that slopes facing the wind are wet and the land behind mountains dry. (simulation) Settings: direction: float -360..360° = 0; moisture: float 0..4 = 1; orographic: float 0..10 = 1; refill: float 0..10 = 0.5; drying: float 0..10 = 0.5.
- Humidity(height: height, rainfall?: mask) -> humidity: mask — How humid each place is: damp by the sea, drying inland, wetter where rain falls. (simulation) Settings: inland: float 1..65536 blocks = 1500; rain_weight: float 0..1 = 0.6; variation: float 0..1 = 0.1; irregularity: float 0..1 = 0.5; seed: seed (from the node's id when left out).
- Biomes(temperature: mask, humidity: mask, height?: height, water?: height) -> biome: biome, color: color — The game's biomes from the climate: oceans by their warmth, beaches, rivers, peaks and meadows by height, and on land a table of temperature against humidity. Settings: peaks: float 0..4096 blocks = 170; beach: float 0..64 blocks = 3; wiggle: float 0..1 = 0.5; wiggle_scale: float 16..16000 blocks = 300; seed: seed (from the node's id when left out).

### Masks

- HeightMask(height: height) -> mask: mask — Where the ground lies between two heights: 1 from From to To, fading over Softness on either side. Settings: from: float -2048..4096 blocks = 90; to: float -2048..4096 blocks = 200; softness: float 0..1024 blocks = 10.
- SlopeMask(height: height) -> mask: mask — Where the ground is steep: 0 below From degrees, 1 above To, a smooth step between (and back to 0 above Upper, when set). Settings: from: float 0..90° = 25; to: float 0..90° = 45; upper: float 0..90° = 90.
- CurvatureMask(height: height) -> mask: mask — Where the ground bulges into ridges or hollows into valleys, at the scale asked for. Settings: mode: "convex"|"concave"|"both" = "convex"; scale: float 2..8192 blocks = 32; sensitivity: float 0..1 = 0.5.
- FacingMask(height: height) -> mask: mask — Where slopes face a direction: 1 on slopes turned toward it, fading within Spread degrees, 0 on flat ground. Settings: direction: float -360..360° = 270; spread: float 1..180° = 90; min_slope: float 0..45° = 5.
- SunlightMask(height: height) -> mask: mask — How much sunlight the ground takes from a sun at a given direction and height: 1 facing it, 0 turned away. Settings: azimuth: float -360..360° = 90; elevation: float 0..90° = 35.
- OpennessMask(height: height) -> mask: mask — How open to the sky the ground is: 1 on summits and plains, lower in gorges, gullies and at the feet of cliffs. Settings: radius: float 2..4096 blocks = 64; directions: int 4..32 = 8.
- SnowMask(height: height) -> mask: mask, height: height — Where snow lies: above a snow line, lower on slopes facing north and in hollows where it drifts, never on slopes too steep to hold it. Settings: line*: float -2048..4096 blocks = 180; softness: float 0..512 blocks = 20; max_slope: float 0..90° = 40; north_bias: float 0..512 blocks = 20; drift: float 0..1 = 0.5; breakup: float 0..1 = 0.3; depth: float 0..64 blocks = 0; seed: seed (from the node's id when left out).
- WetnessMask(height: height, flow?: mask) -> mask: mask — Where the ground stays wet: where much water gathers and it is flat (with a Flow mask), or in flat hollows. Settings: scale: float 2..512 blocks = 32; sensitivity: float 0..1 = 0.5.
- DistanceMask(mask: mask) -> mask: mask, distance: height — How far each place lies from where a mask is: 1 on the mask, fading to 0 Reach blocks away, and the distance itself in blocks. (simulation) Settings: threshold: float 0..1 = 0.5; reach: float 1..65536 blocks = 200.
- Threshold(mask: mask) -> mask: mask — A mask made sharp: 1 above At, 0 below, over Softness. Settings: at: float -4096..4096 = 0.5; softness: float 0..4096 = 0.05.
- InvertMask(mask: mask) -> mask: mask — A mask turned over: 1 where it was 0, 0 where it was 1.
- GrowMask(mask: mask) -> mask: mask — A mask grown outward by Radius blocks, or shrunk inward with a negative radius. Settings: radius: float -4096..4096 blocks = 8.

### Colours

- ColorRamp(height: height) -> color: color — Colours the ground by its height, from one colour at Low to another at High. Settings: low: float -2048..4096 blocks = 60; high: float -2048..4096 blocks = 250; color_low: color = "#5c7a3a"; color_high: color = "#e8e8ea".
- ColorGradient(in: height) -> color: color — Colours from a value — a height, a mask — mapped from Low to High onto a gradient: one from nature, or one of your own. Settings: preset: "custom"|"alpine"|"desert"|"tropical"|"tundra"|"volcanic"|"badlands" = "alpine"; gradient: gradient [{ at, color }, …]; low: float -4096..4096 blocks = 62; high: float -4096..4096 blocks = 300; auto: bool = false; reverse: bool = false; variation: float 0..1 = 0.05; seed: seed (from the node's id when left out).
- SolidColor() -> color: color — One colour everywhere. Settings: color: color = "#7f7f7f".
- MixColors(a: color, b: color, mask?: mask) -> color: color — Two colourings combined through a mask: B laid over A where the mask is 1, by a mode. Settings: mode: "mix"|"multiply"|"screen"|"overlay" = "mix"; strength: float 0..1 = 1.
- AdjustColors(color: color) -> color: color — Colours turned in hue, made more or less vivid, lighter or darker, with more or less contrast. Settings: hue: float -180..180° = 0; saturation: float 0..3 = 1; brightness: float 0..3 = 1; contrast: float 0..3 = 1.
- TerrainLayers(mask_1?: mask, mask_2?: mask, mask_3?: mask, mask_4?: mask, mask_5?: mask, mask_6?: mask, under?: terrain) -> terrain: terrain, color: color — The terrain (block material) of each place, from masks: a base terrain, and up to six layers laid over it where their masks say. Settings: base: text = "Grass"; terrain_1: text = "Rock"; terrain_2: text = "Gravel"; terrain_3: text = "Sand"; terrain_4: text = "Snow"; terrain_5: text = ""; terrain_6: text = ""; threshold: float 0..1 = 0.5; dither: float 0..1 = 0.3.
- SelectTerrain(terrain: terrain) -> mask: mask — A mask of where a terrain lies: 1 where the terrain is the one named. Settings: name: text = "Grass".

### Combine

- Mixer(in.<name>?: height, mask.<name>?: mask) -> height: height — Combines any number of reliefs or masks into one by a mode, each input through its own mask. Settings: mode: "blend"|"add"|"subtract"|"max"|"min"|"multiply"|"average"|"difference"|"screen"|"overlay"|"power" = "max"; strength: float 0..1 = 1.
- Blend(a: height, b: height, mask?: mask) -> height: height — Combines two reliefs, A and B, by a mode, through a mask where one is given. Settings: mode: "add"|"subtract"|"multiply"|"max"|"min"|"average"|"difference"|"screen"|"overlay"|"replace" = "add"; strength: float 0..1 = 1.

### Geometry

- Scatter(density?: mask, height?: height) -> points: points — Points spread over the terrain, as many as a density mask asks, never closer than a spacing. Settings: spacing: float 2..16384 blocks = 64; jitter: float 0..1 = 0.8; seed: seed (from the node's id when left out).
- PointsToMask(points: points) -> mask: mask — A mask of discs round points: 1 within Radius of one, fading to 0 over Softness. Settings: radius: float 0..4096 blocks = 8; softness: float 0..4096 blocks = 4.
- VillageSite(height?: height, area?: mask) -> squares: points — Where a village's squares go: on flat dry land of the area, one for each district. Settings: houses: int 3..120 = 18; density: float 0.2..1 = 0.6; spread: float 0..1 = 0.5; flat: float 0..1 = 0.7; seed: seed (from the node's id when left out).
- VillageRoads(height?: height, area?: mask, squares: points) -> roads: splines — A village's roads: main roads that follow the land between its squares and out of it, side roads off them. Settings: pack: text = ""; road: "theme"|"dirt"|"premium"|"stone"|"planks" = "theme"; spread: float 0..1 = 0.5; width: int 0..9 = 0; theme: text = ""; seed: seed (from the node's id when left out).
- VillageWalls(height?: height, area?: mask, roads: splines, squares?: points, buildings?: instances, fields?: footprints) -> walls: splines, towers: points, roads: splines — What runs round a village, three blocks inside its area: a fence, a hedge, a low wall or ramparts with towers, a gate where a main road crosses. Settings: on: bool = true; style: "wooden-fence"|"spruce-fence"|"post-and-rail"|"hedge"|"flowering-hedge"|"low-stone-wall"|"dry-stone-wall"|"stone-and-fence"|"palisade"|"ramparts" = "wooden-fence"; pack: text = ""; theme: text = "".
- VillagePlots(squares: points, roads: splines) -> plots: footprints — Plots along both sides of a village's roads, each the size of building it would like and its door to the road. Settings: density: float 0.2..1 = 0.6; size_bias: float -1..1 = 0; pack: text = ""; theme: text = ""; seed: seed (from the node's id when left out).
- VillageBuildings(height?: height, area?: mask, squares: points, roads: splines, walls?: splines, plots: footprints) -> buildings: instances, fields: footprints, roads: splines — A building of the pack on each plot with room for it, its door to the road: houses, the church by the square, farms with their fields toward the edge. Settings: pack: text = ""; houses: int 3..120 = 18; size_bias: float -1..1 = 0; fields: float 0..1 = 1; mill: "auto"|"wind"|"water"|"none" = "auto"; max_slope: int 1..32 = 10; margin: int 0..8 = 1; theme: text = ""; seed: seed (from the node's id when left out).
- VillageProps(height?: height, area?: mask, squares: points, roads: splines, walls?: splines, buildings: instances, fields?: footprints) -> props: instances, all: instances — A village's props: market stalls round the square, lampposts and benches along the roads, hay and carts by the farms, fences round the fields. Settings: pack: text = ""; props: float 0..1 = 0.5; theme: text = ""; seed: seed (from the node's id when left out).
- VillageGround(height?: height, buildings: instances, fields?: footprints, roads?: splines) -> height: height, mask: mask — The relief levelled under a village's buildings and fields, and a mask of what the village takes.

### Volumes

- ReliefToVolume(height: height) -> volume: density — The ground of a relief as a volume: matter under its heights, air above.
- SphereVolume(points?: points) -> volume: density — A ball, or one round each point: a hall to carve, a boulder to add. Settings: x: float -1..2 of the side = 0.5; z: float -1..2 of the side = 0.5; y: float -2048..4096 blocks = 20; radius: float 0..4096 blocks = 16; squash: float 0.1..4 = 1; depth: float 0..4096 blocks = 0.
- BoxVolume() -> volume: density — A box with rounded edges: a room, a gallery, a block of rock. Settings: x: float -1..2 of the side = 0.5; z: float -1..2 of the side = 0.5; y: float -2048..4096 blocks = 20; width: float 0..4096 blocks = 32; height: float 0..4096 blocks = 16; length: float 0..4096 blocks = 32; turn: float -180..180° = 0; rounding: float 0..4096 blocks = 2.
- TubeVolume(splines?: splines) -> volume: density — A tunnel along lines: a cave that follows a drawn path, a lava tube, a mine. Settings: path: curve [{ x, z }, …] (x, z from 0 to 1); y: float -2048..4096 blocks = 20; radius: float 0..4096 blocks = 4; depth: float 0..4096 blocks = 0; variation: float 0..1 = 0.3; seed: seed (from the node's id when left out).
- NoiseVolume() -> volume: density — Blobs of 3D noise taking a share of space: pockets, sponge-like rock, floating islands. Settings: share: float 0..1 = 0.2; scale: float 0..4096 blocks = 48; squash: float 0.1..4 = 0.6; seed: seed (from the node's id when left out).
- CaveNoise(mask?: mask) -> volume: density — The game's noise caves as a volume — chambers with pillars, winding tunnels, noodles — between two heights, where a mask says. Settings: cheese: float 0..1 = 0.5; spaghetti: float 0..1 = 0.5; noodles: float 0..1 = 0.3; low: float -2048..4096 blocks = -56; high: float -2048..4096 blocks = 48; size: float 0.5..2 = 1; seed: seed (from the node's id when left out).
- CombineVolumes(a?: density, b?: density) -> volume: density — Two volumes as one: their union, A without B, or what they share, with a smooth seam if asked. Settings: op: "union"|"subtract"|"intersect" = "union"; smooth: float 0..4096 blocks = 0.
- ShapeVolume(volume: density) -> volume: density — A volume grown or shrunk, and roughened: rock walls that are not smooth. Settings: grow: float -64..64 blocks = 0; roughness: float 0..4096 blocks = 2; scale: float 0..4096 blocks = 8; seed: seed (from the node's id when left out).
- LimitVolume(volume: density, mask?: mask, height?: height) -> volume: density — A volume kept between two heights, or under the ground, and where a mask says. Settings: low: float -2048..4096 blocks = -64; high: float -2048..4096 blocks = 320; fade: float 0..4096 blocks = 4; under: bool = true; cover: float 0..4096 blocks = 4.
- CaveLayer(mask?: mask) -> layer: layer — Caves as the game carves them since 1.18 — chambers, tunnels, noodles, ravines, water and lava — where a mask says. Settings: amount*: float 0..1 = 1; cheese: bool = true; cheese_density: float 0..1 = 0.5; spaghetti: bool = true; spaghetti_density: float 0..1 = 0.5; noodles: bool = true; noodles_density: float 0..1 = 0.5; ravines: bool = true; ravines_density: float 0..1 = 0.3; low: float -2048..4096 blocks = -56; high: float -2048..4096 blocks = 319; chambers_high: float -2048..4096 blocks = 54; roof: int 0..64 = 6; openings: float 0..1 = 0.15; cheese_size: float 0.5..2 = 1; spaghetti_size: float 0.5..2 = 1; noodles_size: float 0.5..2 = 1; ravines_size: float 0.5..2 = 1; aquifers: bool = true; aquifer_density: float 0..1 = 0.35; aquifer_size: float 0.5..2 = 1; water_high: float -2048..4096 blocks = 56; lava: float 0..1 = 0.35; lava_under: float -2048..4096 blocks = 0; lava_floor: bool = true; lava_floor_y: float -2048..4096 blocks = -54; seed: seed (from the node's id when left out).
- CaveBiome(mask?: mask) -> layer: layer — A cave biome — lush, dripstone or deep dark — between two heights, where a mask says. Settings: biome: "lush"|"dripstone"|"deep_dark" = "lush"; low: float -2048..4096 blocks = -64; high: float -2048..4096 blocks = 40; amount*: float 0..1 = 1.
- MergeCaveLayers(a?: layer, b?: layer) -> layer: layer — Two cave layers (or cave biomes) as one: each place takes the stronger, or, for cave biomes, both at their own heights. Settings: mode: "stronger"|"both" = "stronger".

### Outputs

- MapOutput(height?: height, water?: height, albedo?: color, terrain?: terrain, biome?: biome, temperature?: mask, humidity?: mask, level.1.top?: height, level.1.bottom?: height, level.1.color?: color, level.2.top?: height, level.2.bottom?: height, level.2.color?: color, level.3.top?: height, level.3.bottom?: height, level.3.color?: color, level.4.top?: height, level.4.bottom?: height, level.4.color?: color, cover?: layer, objects?: layer, understory?: layer, pockets?: layer, caves?: layer, cave_biomes?: layer, annotations?: layer, structures?: instances, roads?: splines, volume.carve?: density, volume.add?: density, mask.flow?: mask, mask.wear?: mask, mask.deposition?: mask, mask.snow?: mask, mask.<name>?: mask) -> the map — What the graph gives the map: its ground, water, surface, climate, life, underground and masks, each input optional. Settings: active: bool = true; albedo_palette: "natural"|"rock"|"desert"|"snowy"|"every" = "natural"; paint_biomes: bool = false.

### Legacy (kept for graphs of 1.2's first previews; prefer the nodes above)

- LegacyFloatingIslands() -> top: height, bottom: height, islands: mask — Islands of rock floating in the air: a rolling top on sheer sides, over an underside hanging down to a point. (simulation) Settings: altitude: float -64..4096 blocks = 230; spread: float 0..1000 blocks = 25; size: float 4..4000 blocks = 70; coverage: float 0..1 = 0.5; depth: float 0..10 = 1.3; seed: int 0..4294967295 = 11.
- LegacyOverhang(in: height) -> top: height, bottom: height — Rims jutting out from the top of cliffs over the void below: a slab of rock, as its top and bottom. (simulation) Settings: reach: float 0..200 blocks = 10; thickness: float 1..200 blocks = 8; drop: float 1..400 blocks = 24; variation: float 0..1 = 0.6; seed: int 0..4294967295 = 12.
- LegacyRocks() -> out: height — Rocky detail around 0, to add to a relief. (simulation) Settings: scale: float 2..8000 blocks = 90; height: float 0..1000 blocks = 12; sharpness: float 0..1 = 0.6; seed: int 0..4294967295 = 10.
- LegacyTectonicPlates() -> height: height, ranges: mask, faults: mask — Relief from tectonic plates: continents and oceans, mountains where plates collide, rifts where they pull apart. (simulation) Settings: plates: text = "[]"; relief: float 0..4096 blocks = 170; width: float 1..20000 blocks = 260; roughness: float 0..1 = 0.6; sea: float -64..320 blocks = 63; seed: int 0..4294967295 = 7.
- LegacyWorldFile() -> height: height, water: mask, colour: color — A world brought in (WorldPainter's .world, Massif's .massif): its surface, its water and its colours. (simulation) Settings: path: file (.world, .massif, .png, .r16, .r32); low: float -4096..4096 blocks = 40; high: float -4096..4096 blocks = 220; fit: bool = true.
- LegacyErosion(in: height) -> height: height, flow: mask, deposits: mask, wear: mask — Water running down the slopes carries soil from the heights to the valleys: gullies, smoothed slopes, alluvial fans. (simulation) Settings: rain: float 0..20 = 1; erosion: float 0..1 = 0.3; deposition: float 0..1 = 0.3; capacity: float 0..64 = 4; evaporation: float 0..0.5 = 0.02; radius: float 1..64 blocks = 6; seed: int 0..4294967295 = 5.
- LegacyRivers(in: height, water_level?: height) -> height: height, rivers: mask, water: height — Rivers traced where the water of the whole terrain runs, wider and deeper downstream, dug into their valleys, with lakes in the hollows they cross. (simulation) Settings: threshold: float 0..1 = 0.55; depth: float 0..100 blocks = 5; width: float 1..400 blocks = 14; sea: float -10000..320 blocks = 63; banks: float 0..10 = 1; lakes: float 0..1 = 0.5.
- LegacyLakes(in: height) -> height: height, lakes: mask, water: height — The hollows water would fill, filled: lakes and flat valley floors. (simulation) Settings: amount: float 0..1 = 1; sea: float -10000..320 blocks = 63.
- LegacySea(in: height) -> height: height, water: mask, shore: mask — A sea level: where the ground is under water, and its shore. (simulation) Settings: level: float -64..320 blocks = 62; shore: float 0..100 blocks = 4.
- LegacyFlow(in: height) -> mask: mask — Where the water of the terrain runs and gathers. (simulation) Settings: spread: float 0..1 = 0.5.
- LegacyAutoColor(height: height) -> colour: color — Natural colours from the relief alone: valleys, meadows, rock on the steep slopes, snow on the peaks. (simulation) Settings: palette: "temperate"|"alpine"|"desert"|"arctic"|"volcanic"|"tropical"|"lunar"|"martian" = "temperate"; snow_line: float -64..4096 blocks = 200; rock_slope: float 0..90° = 38; seed: int 0..4294967295 = 1.

### Composites shipped with Massif

- AlpineRange() -> height, snow, flow, wear, deposition, range — A mountain chain along a drawn line, grown by its rivers: a snowy crest branching into spurs, valleys cut up every flank, plains at its feet. Settings: spine; seed; height 0..2000 = 220; base = 70; width = 2600; spurs; erosion 0..20 = 2; downcutting 0..1 = 0.35; snowline = 210.
- VillageGenerator(height?: height, area?: mask) -> height, structures, roads, taken, squares, plots, walls, fields — A village of a builder's pack on the terrain: squares on flat land, roads that follow it, buildings facing them on levelled ground, farms with their fields, props, and walls if you want them. Settings: pack; houses; size_bias; road; walls = false; wall_style; density; spread; flat; fields; props; mill; seed; theme; max_slope; margin; width.
- CaveSystem(height: height, area?: mask) -> caves, biomes — The game's caves under a relief — chambers, tunnels, noodles, ravines, aquifers and a lava floor — with lush caves under wet ground, deep dark under the mountains and dripstone caves between. Settings: seed; chambers; tunnels; noodles; ravines; size; low; high; entrances; aquifers; flooded; lava; lava_floor; lush; dripstone; deep_dark; mountains.

### Aliases

`Add` = Blend(mode: "add"), `Average` = Blend(mode: "average"), `BillowNoise` = FractalNoise(style: "billow"), `Cells` = CellNoise, `ColourGradient` = ColorGradient, `ColourRamp` = ColorRamp, `Combine` = Blend, `Difference` = Blend(mode: "difference"), `DomainWarp` = Warp, `Fbm` = FractalNoise, `HydraulicErosion` = Erosion, `Max` = Blend(mode: "max"), `Min` = Blend(mode: "min"), `Multiply` = Blend(mode: "multiply"), `Noise` = FractalNoise, `Output` = MapOutput, `Overlay` = Blend(mode: "overlay"), `Perlin` = FractalNoise, `Remap` = Levels, `RidgedNoise` = FractalNoise(style: "ridged"), `Screen` = Blend(mode: "screen"), `Slope` = SlopeMask, `StreamErosion` = Erosion, `Subtract` = Blend(mode: "subtract"), `Voronoi` = CellNoise

## Examples

Alpine mountains: noise, erosion, masks, groups and outputs:

```mscript
project "Alpine Mountains" {
    seed = 4268;

    group "Base Terrain" {
        color = #347a52;
        base = FractalNoise(scale: 2600, height: 700, base: 40, octaves: 9);
        ridges = RidgedNoise(scale: 1400, height: 500, seed: 7);
        mountains = Add(base, ridges, strength: 0.8);
        lifted = Levels(mountains, in_low: 40, in_high: 1200, out_low: 30, out_high: 300, gamma: 1.3);
    }

    group "Geology" {
        color = #8a6a3a;
        warped = Warp(lifted);
        eroded = StreamErosion(warped);
    }

    group "Environment" {
        color = #3a6ea5;
        slope = SlopeMask(eroded);
        flat = InvertMask(slope);
    }

    output height = eroded;
    output mask.flat = flat;
}
```

A component with exposed settings, used twice, and a loop:

```mscript
project "Twin peaks" {
    seed = 12;

    component MountainGenerator(seed: seed = 42, height: float = 800, roughness: float = 0.5) {
        label = "Mountain generator";
        expose height { label: "Mountain height"; min: 0; max: 2000; unit: "blocks"; }
        expose roughness { label: "Roughness"; min: 0.05; max: 0.95; }

        base = FractalNoise(seed: seed, height: height, roughness: roughness);
        ridges = RidgedNoise(seed: seed, scale: 900, height: 300);
        terrain = Max(base, ridges);
        output terrain;
    }

    west = MountainGenerator(height: 900, seed: 1);
    east = MountainGenerator(height: 600, roughness: 0.7, seed: 2);
    both = Max(west, east);

    // Three terraced copies, each finer than the last: loops build several nodes.
    for i in 0..3 {
        step[i] = Terraces(both, steps: 4 + i * 4);
    }
    output height = step[2];
}
```
