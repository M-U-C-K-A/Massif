# Scripts

[← Back to the README](../README.md)

Massif runs scripts on a map: **WorldPainter's JavaScript scripts as they
are**, and scripts written for **Massif's own API, in Lua**. A script applies
hundreds of masks, does sums over heights and rivers, checks a map before it
is exported, or makes a map from nothing — what one does by hand, repeated.

- [Running a script](#running-a-script)
- [WorldPainter scripts](#worldpainter-scripts)
- [Massif's own API, in Lua](#massifs-own-api-in-lua)
- [Examples](#examples)
- [What a script may touch](#what-a-script-may-touch)
- [Other programs on the open map](#other-programs-on-the-open-map)
- [Speed](#speed)
- [How Massif checks it gives WorldPainter's map](#how-massif-checks-it-gives-worldpainters-map)

Coming from WorldPainter? [Coming from WorldPainter scripts](coming-from-worldpainter-scripts.md)
lists what is the same, what differs and what Massif does not do yet.

## Running a script

**In Massif: File ▸ Run Script…** picks a script (`.lua` for Massif's own
API, `.js` for WorldPainter's) and opens it in a dialog: its name and description, the parameters its descriptor names (see
below), and the arguments, one a line. **Run** runs it on the open map, in
the background: what it prints shows in the dialog as it comes, warnings in
grey, errors in red with their **line and column**; a bar shows how far it
is, and **Stop** stops it. The script works on a copy of the map: when it
is done, what it changed comes back as **one step to undo** (Edit ▸ Undo
takes the whole run back); a script that fails or is stopped leaves the map
as it was. **Run again** reads the file again, so a script can be edited
beside Massif and run once more.

The open map is a WorldPainter script's `world`, and the dimension shown its
`dimension`, as in WorldPainter's Tools ▸ Run Script; in Lua, it is
`massif.map`.

**On the command line:**

```bash
massif-cli script my-script.lua first-argument second-argument
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
do (`//` in JavaScript, `--` in Lua); the Run Script dialog asks for them:

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

## Massif's own API, in Lua

Scripts for Massif itself are written in **Lua** (Luau, the Lua of Roblox:
Lua 5.1 with types you may add, `--!strict`, and a few more conveniences).
A `.lua` script runs as a `.js` one does: File ▸ Run Script…, or
`massif-cli script my.lua arguments…`. Where WorldPainter's API goes column
by column, Massif's reads and writes a **rectangle of columns at once**, a
grid, so that a script over a whole map makes a few calls, not millions.

```lua
-- Frost over the high land of the open map.
local map = massif.map
local frost = map:layer("Frost")
for x0, z0, x1, z1 in map:tiles() do
    local h = map:get("height", x0, z0, x1, z1)
    local cover = map:get("cover", x0, z0, x1, z1)
    for i = 1, #h do
        if h[i] and h[i] > 150 then
            cover[i] = frost.id
        end
    end
    map:set("cover", x0, z0, x1, z1, cover)
    if massif.cancelled() then return end
end
```

### Maps

| | |
|---|---|
| `massif.map` | the map open in Massif (in File ▸ Run Script…), `nil` on the command line unless `--world` gives one |
| `massif.open(file)` | a `.massif`, or a WorldPainter `.world` imported |
| `massif.create{ width, depth, landform, seed, seaLevel, minY, maxY, … }` | a new map, as File ▸ New World makes one (`landform`: `"flat"`, `"plains"`, `"hills"`, `"mountains"`, `"islands"`) |
| `map.name`, `map.seed`, `map.sea_level` | read and set |
| `map:bounds()` | `x0, z0, x1, z1`: the blocks the map covers, `x1` and `z1` past the last |
| `map:tiles()` | for `x0, z0, x1, z1 in map:tiles()`: each tile of 128 × 128 blocks |
| `map:dimension("nether")` | the same map's Nether (or `"end"`, `"surface"`) |
| `map:info()` | a table: name, seed, sea level, spawn, dimensions and their bounds |
| `map:save(file)` | writes a `.massif` |
| `map:export(folder, { version = "1.21.11" })` | exports to Minecraft |

### Columns, by rectangles

`map:get(kind, x0, z0, x1, z1)` gives a **grid** of the rectangle's columns;
`map:set(kind, x0, z0, x1, z1, values)` writes one back (a grid, a table of
numbers, or one number for all). A grid is read and written as `g[i]` (1 to
`#g`, row after row, `x` first), or `g:at(x, z)` and `g:set(x, z, v)` in
the map's coordinates; `nil` is a column the map does not have (and, to
write, one to leave as it is). `g.width`, `g.depth`, `g.x0`, `g.z0`; and,
without a loop in the script, `g:sum()`, `g:min()`, `g:max()`,
`g:count(v)`, `g:fill(v)`, `g:copy()`.

| Kind | Values |
|---|---|
| `"height"` | the y of the top block (fractions too) |
| `"water"` | the y of the top water block; `nil` where there is no water (write `-math.huge` to take water away) |
| `"terrain"` | the map's terrain numbers: `map:terrain_id("Grass")`, `map:terrains()` |
| `"biome"` | biome numbers (0: automatic): `map:biome_id("plains")`, `map:biomes()` |
| `"cover"`, `"trees"`, `"understory"`, `"pockets"` | the layer of that slot (0: none): `map:layer("Frost")` gives `{ kind = "cover", id = … }`, `map:layers()` all of them |
| `"annotations"` | 0 to 15, WorldPainter's colours |
| `"sizes"` | the size of tree, 0 (any) to 4 |
| `"selection"` | 0 to 255, read only |

`map:height(x, z)` and `map:set_height(x, z, h)` read and write one column.

### Masks from images

`map:import_mask(file, options)` paints what an image is on: what
WorldPainter scripts do with `applyHeightMap`, in Massif's own terms, and as
fast (600 masks of 4096² on a 4096² map in under 3 seconds).

```lua
local map = massif.map
local x0, z0 = map:bounds()
map:import_mask("rock.png", { paint = "terrain", value = map:terrain_id("Rock"), x = x0, z = z0 })
map:import_mask("pines.png", { paint = "trees", value = map:layer("Pine").id, threshold = 0.25 })
map:import_mask("relief.png", { paint = "height", black = 40, white = 200 })
```

| Option | |
|---|---|
| `paint` | `"terrain"`, `"biome"`, `"cover"`, `"trees"`, `"understory"`, `"pockets"`, `"sizes"`, `"annotations"`, `"water"` or `"height"` |
| `value` | what it paints: the terrain, biome or layer number, the annotation colour (1 to 15), the water's level |
| `threshold` | where a pixel is on: from this share of white (0 to 1, 0.5 by default) |
| `invert` | the dark pixels on instead |
| `black`, `white` | for `"height"`: the heights black and white give, the greys between |
| `x`, `z` | where the image's top left corner lands (0, 0 by default, as WorldPainter's) |
| `scale` | blocks a pixel (1 by default) |
| `channel` | `"red"`, `"green"` or `"blue"` of a colour image (its first by default) |

Masks wait until the map is read, saved or the script ends, then are read
side by side, so that hundreds of them take seconds.

### The whole map

`map:whole_map(name, settings)` runs one of Edit ▸ Whole map's passes:
`"erosion"`, `"thermalErosion"`, `"smooth"`, `"sharpen"`, `"terraces"`,
`"strata"`, `"lakes"`, `"rivers"`, `"scaleAndRaise"`, with the settings of
its dialog by name (`{ rain = 1.5 }`, `{ radius = 4 }`, `{ scale = 1.2,
shift = 10 }`); those left out keep the dialog's. `map:select(x0, z0, x1,
z1)` selects a rectangle (`map:select(nil)` none).

### The run

`print(…)` writes to the dialog's output; `massif.warn(…)` a warning.
`massif.progress(fraction, label)` moves the progress bar;
`massif.cancelled()` says whether Stop was pressed (a loop is stopped
anyway). `massif.args` are the arguments, `massif.params` the parameters
of the descriptor — written with `--` in Lua:

```lua
-- script.name=Frost up high
-- script.param.above.type=integer
-- script.param.above.default=150
local above = massif.params.above
```

Lua scripts run in a sandbox: no `io`, no `os.execute`, no `require` of
files, the global environment read-only; files only through Massif, and
only those a script may touch.

### Examples

Sand along the sea, three blocks over it at most:

```lua
local map = massif.map
local sand = map:terrain_id("Sand")
local sea = map.sea_level
for x0, z0, x1, z1 in map:tiles() do
    local h = map:get("height", x0, z0, x1, z1)
    local t = map:get("terrain", x0, z0, x1, z1)
    for i = 1, #h do
        if h[i] and h[i] >= sea - 2 and h[i] <= sea + 3 then t[i] = sand end
    end
    map:set("terrain", x0, z0, x1, z1, t)
end
```

How much of the map stands above a height, given as a parameter:

```lua
-- script.name=Land above
-- script.param.above.type=integer
-- script.param.above.default=200
local map, above = massif.map, massif.params.above
local high, all = 0, 0
for x0, z0, x1, z1 in map:tiles() do
    local h = map:get("height", x0, z0, x1, z1)
    for i = 1, #h do
        if h[i] then
            all += 1
            if h[i] > above then high += 1 end
        end
    end
end
print(string.format("%.1f %% of the land is above y %d", 100 * high / all, above))
```

A map made, eroded, saved and exported from the command line (`massif-cli script make.lua`):

```lua
local map = massif.create{ width = 2048, depth = 2048, landform = "mountains", seed = 42 }
map.name = "Eroded range"
map:whole_map("erosion", { rain = 1.5 })
map:whole_map("rivers", {})
map:save("eroded.massif")
map:export(".", { version = "1.21.11" })
```

The columns marked with one annotation colour made into rock, then rubbed out (colours are numbered 1 to 15, in the order of WorldPainter's list):

```lua
local map = massif.map
local rock = map:terrain_id("Rock")
local MARK = 1
for x0, z0, x1, z1 in map:tiles() do
    local a = map:get("annotations", x0, z0, x1, z1)
    local t = map:get("terrain", x0, z0, x1, z1)
    local changed = false
    for i = 1, #a do
        if a[i] == MARK then t[i] = rock; a[i] = 0; changed = true end
    end
    if changed then
        map:set("terrain", x0, z0, x1, z1, t)
        map:set("annotations", x0, z0, x1, z1, a)
    end
end
```

## What a script may touch

A script reads and writes only:

- files in its own folder (and the folders under it);
- files and folders it is given: a `file` parameter picked in the dialog,
  a path given as an argument on the command line, a folder allowed with
  `--allow`, and on the command line the folder it is run from;
- Minecraft's `saves` folder, to export a map into it.

Any other path is refused with a message saying how to allow it. Scripts
have no network at all, and cannot run programs.

## Other programs on the open map

A program running on the same computer, such as an AI assistant through the
[massif-mcp](https://github.com/Swift-ality/massif-mcp) server, can read the
map open in Massif and change it while you watch. Massif listens on the
loopback address only, on a port it picks, and writes the port with a token
of its own to `bridge/<process>.json` beside its log (`%LOCALAPPDATA%\Massif`
on Windows): only programs of your own session can read it, and a request
without the token is refused. The file goes when Massif closes.

A request is one line of JSON, `{"cmd": …, "token": …}`, answered by one
line, `{"ok": true, "status": {…}}` or `{"ok": false, "error": "…"}`:

| `cmd` | |
|---|---|
| `status` | the open map: `name`, `file`, `modified`, `generation`, `revision`, `undo` (the last step's name) |
| `map` | writes a copy of the open map to `file` (an absolute path to a `.massif` that does not exist yet) |
| `apply` | takes the map in `file` in place of the open one, as **one step to undo** named `label`, when `generation` and `revision` are still those `map` gave: a map changed in Massif meanwhile, a stroke under way, a dialog open, or a map open read-only is refused, and nothing is written over |
| `undo`, `redo` | as Edit ▸ Undo and Redo |
| `save` | saves the map to its own file |

So a program reads the map with `map`, changes the copy (with `massif-cli`
and a script, say), and gives it back with `apply`. `MASSIF_BRIDGE=0` in
Massif's environment turns this off.

## Speed

Massif keeps a script's mappings until the map is read, saved or the script
ends, then decodes their images side by side and applies them tile by tile
on every core, skipping the tiles an image maps nothing to. Measured on a
4096 × 4096 map made from a height map, with 600 masks of 4096² pixels
onto terrains, forests on land, frost up high and annotations:

| | Time | CPU |
|---|---|---|
| WorldPainter 2.27 | 5 min 55 s | 124 s |
| Massif, the same WorldPainter script | 8.0 s | 12.4 s |
| Massif, `map:import_mask` in Lua | 2.8 s | 8.6 s |

The maps of the WorldPainter script are the same, column for column.

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
