# Coming from WorldPainter scripts

[← Scripts](scripting.md)

Your WorldPainter scripts run in Massif without changes: **File ▸ Run
Script…** in the editor, `massif-cli script your-script.js arguments…` on
the command line. This page says what to expect.

## The same

- **The API.** `wp` and its operations, their options, their checks and
  their error messages are WorldPainter's: a script that ran in
  WorldPainter runs here, and a script with a mistake fails with the
  message WorldPainter gives ("You forgot to invoke go() on the getLayer()
  operation"). `print(e)` of an error shows the same Java exception names.
- **The results.** Massif applies masks, filters, layers and terrains as
  WorldPainter does, column for column: what a script makes is, once
  exported, what WorldPainter would have made. Measured against
  WorldPainter itself on its own test scripts and on scripts of 60 and 600
  masks: the same heights, water, terrains, biomes, frost, forests and
  annotations.
- **Terrain numbers.** `applyTerrain(36)`, `toTerrain(0)` and
  `Terrain.VALUES[i]` number terrains as WorldPainter's
  [terrain type values](https://www.worldpainter.net/trac/wiki/Scripting/TerrainTypeValues)
  do (36 the beaches, 0 grass).
- **Images.** Masks read as Java reads them: a 1-bit mask gives 0 and 1, a
  mask with a palette gives the palette's indices, a 16-bit height map 0 to
  65,535. `fromLevels(1, 255)` on a black and white mask works as before.
- **Quirks kept.** Where WorldPainter does something surprising, Massif
  does it too, so that results match: a filter's
  `onlyOnLayer(layer).withValue(v)` keeps the columns where the layer does
  *not* have that value (and `exceptOnLayer(layer).withValue(v)` those
  where it does); a custom terrain used before `installCustomTerrain`
  paints an empty terrain (Massif warns); a map made from a height map one
  for one sits 0.4375 of a block lower, as WorldPainter makes it.
- **The bindings.** `world`, `dimension`, `arguments`, `argv`, `argc`,
  `params`, `scriptDir`, `print`, and the descriptor at the script's head.

## Different

- **Saving.** `wp.saveWorld(world).toFile('map.world')` writes `map.massif`:
  Massif keeps its own format. It says so when it does. Massif opens
  `.world` files (`wp.getWorld().fromFile(…)`) as its import does.
- **Beaches.** WorldPainter's Beaches terrain becomes grass with patches of
  sand, gravel and clay, as WorldPainter exports it: `getTerrainAt()` on a
  beach gives that grass, sand, gravel or clay, not `Terrain.BEACHES`.
- **Forests at part strength.** A forest painted at half strength keeps
  half its columns, as many as WorldPainter's: which columns is Massif's
  own choice (WorldPainter places its trees at random when it exports).
  Where two forests overlap, the stronger wins, and where they are as
  strong, the one WorldPainter keeps first.
- **Maps made from a height map.** Heights, water and the theme's rules are
  WorldPainter's (beaches from two under the water to one over, then grass,
  permadirt, stone mix and deep snow by height, frost high up); where
  WorldPainter jitters the bands between them with noise it seeds at
  random, Massif uses its own noise, so the edges of those bands differ
  from one WorldPainter run to the next as from Massif. A scaled height
  map is read bilinearly (WorldPainter bicubic).
- **Files.** A script may only use files in its folder, those it is given
  and Minecraft's saves; see [What a script may touch](scripting.md#what-a-script-may-touch).
- **Undo.** In the editor, a whole run is one step to undo.
- **Speed.** Hundreds of masks take seconds: 600 masks on a 4096² map, 8 s
  against 6 minutes.

## Not yet

These stop the script with "… is not supported yet":

- `wp.mergeWorld()` (a TODO in WorldPainter too);
- the ceiling dimensions (`applyToSurfaceCeiling()` and the like);
- `onlyOnAutoBiome(id)` and `exceptOnAutoBiome(id)` (Massif decides the
  automatic biomes as it exports);
- the Resources layer (Massif lays its ores over the whole map: Edit ▸
  Ores), Half Blocks, and WorldPainter's River layer (Massif's rivers:
  Edit ▸ Whole map ▸ Rivers);
- Java classes other than those scripts use for maps (`java.io.File`,
  `java.util.*`…): use JavaScript's own (`Math`, arrays, `JSON`);
- a few calls of `dimension` and `world` scripts seldom make
  (`getLayersAt`, `getMixedMaterial`…).

Populate and Read Only do nothing in Massif, which writes finished chunks:
it says so once.
