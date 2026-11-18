# Scripts

[← Back to the README](../README.md)

Massif runs scripts on a map: **WorldPainter's JavaScript scripts as they
are**, and scripts written for Massif's own API. A script applies hundreds of
masks, does sums over heights and rivers, checks a map before it is exported,
or makes a map from nothing — what one does by hand, repeated.

- [Running a script](#running-a-script)
- [WorldPainter scripts](#worldpainter-scripts)
- [What a script may touch](#what-a-script-may-touch)
- [Speed](#speed)
- [How Massif checks it gives WorldPainter's map](#how-massif-checks-it-gives-worldpainters-map)

Coming from WorldPainter? [Coming from WorldPainter scripts](coming-from-worldpainter-scripts.md)
lists what is the same, what differs and what Massif does not do yet.

## Running a script

**In Massif: File ▸ Run Script…** picks a `.js` file and opens it in a
dialog: its name and description, the parameters its descriptor names (see
below), and the arguments, one a line. **Run** runs it on the open map, in
the background: what it prints shows in the dialog as it comes, warnings in
grey, errors in red with their **line and column**; a bar shows how far it
is, and **Stop** stops it. The script works on a copy of the map: when it
is done, what it changed comes back as **one step to undo** (Edit ▸ Undo
takes the whole run back); a script that fails or is stopped leaves the map
as it was. **Run again** reads the file again, so a script can be edited
beside Massif and run once more.

The open map is the script's `world`, and the dimension shown its
`dimension`, as in WorldPainter's Tools ▸ Run Script.

**On the command line:**

```bash
massif-cli script my-script.js first-argument second-argument
```

Everything after the script is its arguments (`arguments`, `argv`). Options
come before the script:

| Option | What it does |
|---|---|
| `--world <map>` | opens a map (a `.massif`, or a WorldPainter `.world`) as the script's `world` |
| `--out <map.massif>` | saves that map once the script is done well |
| `--param name=value` | sets a parameter of the script's descriptor (the others take their default) |
| `--allow <folder>` | lets the script use files in that folder too |

It prints what the script prints, warnings and errors on the standard
error with the line, and ends with the script's `exit(code)`, 1 on an
error, 0 otherwise.

### The descriptor

A script names its parameters in comments at its head, as WorldPainter's
do; the Run Script dialog asks for them:

```js
// script.name=Frost up high
// script.description=Frost over the land above a height.
// script.param.above.type=integer
// script.param.above.default=120
// script.param.above.description=Lowest height with frost
// script.param.mask.type=file
// script.param.mask.optional=true
// script.hideCmdLineParams=true
var high = wp.createFilter().aboveLevel(params['above']).go();
```

Types are `string`, `integer`, `percentage`, `float`, `boolean` and `file`;
each parameter may have a `description` (shown on hover), a `displayName`,
a `default` and `optional=true`. `script.hideCmdLineParams=true` hides the
arguments box.

## WorldPainter scripts

A WorldPainter script runs as it is: its `wp` object, its operations ended
by `go()`, and the Java objects scripts reach.

```js
var heightMap = wp.getHeightMap().fromFile('heightmap.png').go();
var world = wp.createWorld().fromHeightMap(heightMap).go();
var frost = wp.getLayer().withName('Frost').go();
wp.applyHeightMap(heightMap)
    .toWorld(world)
    .applyToLayer(frost)
    .fromLevels(0, 119).toLevel(0)
    .fromLevels(120, 255).toLevel(1)
    .go();
var river = wp.getLayer().fromFile('River.layer').go();
var mask = wp.getHeightMap().fromFile('rivermask.png').go();
wp.applyHeightMap(mask).toWorld(world).applyToLayer(river).fromLevels(1, 255).toLevel(1).go();
wp.saveWorld(world).toFile('output.world').go();
```

What Massif has of WorldPainter's API:

| Call | In Massif |
|---|---|
| `wp.getHeightMap().fromFile(f)[.selectRedChannel() …].go()` | PNG (any bit depth, palettes as their indices, as Java reads them), TIFF, BMP, JPEG |
| `wp.applyHeightMap(hm)`, `wp.applyLayer(l)`, `wp.applyTerrain(i)` with `toWorld`, `applyToLayer`, `applyToTerrain`, `fromLevel(s)`, `fromColour`, `toLevel(s)`, `toTerrain`, `setAlways`/`setWhenHigher`/`setWhenLower`, `scale`, `shift`, `withFilter`, `applyToSurface`/`Nether`/`End` | all, as WorldPainter maps them |
| `wp.createFilter()` with `aboveLevel`, `belowLevel`, `feather`, `aboveDegrees`, `belowDegrees`, `onlyOn…`/`exceptOn…` (terrain, layer, `withValue`, `orHigher`, `orLower`, biome, automatic biomes, water, land), `inSelection`, `outsideSelection` | all but `onlyOnAutoBiome(id)` and `exceptOnAutoBiome(id)` |
| `wp.getLayer()` `.withName(n)` (Frost, Lava, Void, Deciduous, Pine, Jungle, Swamp, Annotations, Biomes, Caverns, Chasms, Caves, the tree pack's trees), `.fromFile(f)` (a `.layer`), `.fromWorld(w).withName(n)` | as a `.world`'s import makes them |
| `wp.getTerrain().fromFile(f)`, `wp.installCustomTerrain(t).toWorld(w)[.inSlot(n)]` | custom terrains |
| `wp.createWorld().fromHeightMap(hm)` with `scale`, `shift`, `fromLevels`, `toLevels`, `withWaterLevel`, `withMapFormat`, `withLowerBuildLimit`, `withUpperBuildLimit` | as WorldPainter's importer |
| `wp.getWorld().fromFile(f)` | a WorldPainter `.world` or a `.massif` |
| `wp.saveWorld(w).toFile(f)` | writes a `.massif` (next to the name given) |
| `wp.exportWorld(w)[.toDirectory(d)]` | exports to Minecraft |
| `wp.getMapFormat()`, `wp.getBiomeId()`, `wp.version`, `wp.interrupted`, `wp.checkForInterrupt()` | yes |
| `world` and `dimension`: heights, water levels, terrains, layer values, slope, extent, tiles | the calls scripts make (`getHeightAt`, `setHeightAt`, `getWaterLevelAt`, `getTerrainAt`, `setLayerValueAt`, `getExtent`, `isTilePresent`…) |
| `org.pepsoft.worldpainter.Terrain`, `….layers.Annotations.INSTANCE` and the other standard layers, `Java.type(…)` of those | yes |
| `print`, `console.log`/`warn`/`error`, `arguments`, `argv`, `argc`, `params`, `scriptDir`, `exit(code)` | yes |

Anything else says so when the script calls it — `wp.mergeWorld() is not
supported yet`, `dimension.getLayersAt() is not supported yet`, `The Java
class java.io.File is not supported yet` — and stops the script there, with
its line: never nothing done in silence.

The language is modern JavaScript (QuickJS), which runs WorldPainter's
older scripts (written for Nashorn's ECMAScript 5.1) unchanged.

## What a script may touch

A script reads and writes only:

- files in its own folder (and the folders under it);
- files and folders it is given: a `file` parameter picked in the dialog,
  a path given as an argument on the command line, a folder allowed with
  `--allow`, and on the command line the folder it is run from;
- Minecraft's `saves` folder, to export a map into it.

Any other path is refused with a message saying how to allow it. Scripts
have no network at all, and cannot run programs.

## Speed

Massif keeps a script's mappings until the map is read, saved or the script
ends, then decodes their images side by side and applies them tile by tile
on every core, skipping the tiles an image maps nothing to. Measured on a
4096 × 4096 map made from a height map, with 600 masks of 4096² pixels
onto terrains, forests on land, frost up high and annotations:

| | Time | CPU |
|---|---|---|
| WorldPainter 2.27 | 5 min 55 s | 124 s |
| Massif | 8.0 s | 12.4 s |

The two maps are the same, column for column.

A script's own loops over columns (`dimension.getHeightAt(x, y)` and the
like) cost about a tenth of a microsecond a call.

## How Massif checks it gives WorldPainter's map

WorldPainter's own scripting tests, and scripts written as map makers write
them — rivers dug with height maths, checks before export, sixty masks with
filters — were run once in WorldPainter, and what its maps hold kept in
`crates/massif-script/tests/wp/expected`: columns by terrain, layer, biome,
annotation, water and height. The tests run the same scripts in Massif and
compare. The tools that made them are in `crates/massif-script/examples`
(`compare`, `masks`); [Coming from WorldPainter scripts](coming-from-worldpainter-scripts.md)
lists the differences found.
