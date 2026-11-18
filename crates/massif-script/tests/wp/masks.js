// Many masks onto terrains and layers, as a map maker's script does: zones
// of terrain, forests on land, frost up high, annotations. Arguments: the
// folder of the masks (mask0.png…) and how many, the map to save.
var heightMap = wp.getHeightMap().fromFile(scriptDir + '/heightmap.png').go();
var world = wp.createWorld().fromHeightMap(heightMap).go();
var frost = wp.getLayer().withName('Frost').go();
var deciduous = wp.getLayer().withName('Deciduous').go();
var pine = wp.getLayer().withName('Pine').go();
var annotations = wp.getLayer().withName('Annotations').go();
var land = wp.createFilter().onlyOnLand().go();
var high = wp.createFilter().aboveLevel(110).go();
var terrains = [28, 29, 30, 32];   // stone, rock, cobblestone, obsidian
var count = parseInt(arguments[1], 10);
for (var i = 0; i < count; i++) {
    var mask = wp.getHeightMap().fromFile(arguments[0] + '/mask' + i + '.png').go();
    var k = i % 6;
    if (k < 3) {
        wp.applyHeightMap(mask).toWorld(world).applyToTerrain().fromLevels(128, 255).toTerrain(terrains[(i / 6 | 0) % 4]).go();
    } else if (k === 3) {
        wp.applyHeightMap(mask).toWorld(world).applyToLayer(i % 12 === 3 ? deciduous : pine).fromLevels(1, 255).toLevels(1, 15).withFilter(land).go();
    } else if (k === 4) {
        wp.applyHeightMap(mask).toWorld(world).applyToLayer(frost).fromLevels(128, 255).toLevel(1).withFilter(high).go();
    } else {
        wp.applyHeightMap(mask).toWorld(world).applyToLayer(annotations).fromLevels(1, 255).toLevel((i % 15) + 1).go();
    }
}
wp.saveWorld(world).toFile(arguments[2]).go();
