// WorldPainter's colourmappingtest.js on a map made from its height map:
// biomes painted from a colour map, exact colours to biome numbers.
var heightMap = wp.getHeightMap().fromFile(scriptDir + '/heightmap.png').go();
var world = wp.createWorld().fromHeightMap(heightMap).go();
var colourMap = wp.getHeightMap().fromFile(scriptDir + '/colourmap.png').go();
var biomesLayer = wp.getLayer().withName('Biomes').go();
wp.applyHeightMap(colourMap)
    .toWorld(world)
    .applyToLayer(biomesLayer)
    .fromColour(255, 0, 0).toLevel(4) // Red -> Forest
    .fromColour(0, 255, 0).toLevel(5) // Green -> Taiga
    .fromColour(0, 0, 255).toLevel(6) // Blue -> Swampland
    .go();
wp.saveWorld(world).toFile(arguments[0]).go();
