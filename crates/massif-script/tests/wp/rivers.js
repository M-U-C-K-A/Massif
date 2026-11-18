// Rivers and height maths, as a map maker's script does them: a map from a
// height map, the river mask dug into it with water, then measured.
// Runs in WorldPainter and in Massif alike.
var heightMap = wp.getHeightMap().fromFile(scriptDir + '/heightmap.png').go();
var world = wp.createWorld().fromHeightMap(heightMap).go();
var mask = wp.getHeightMap().fromFile(scriptDir + '/rivermask.png').go();
var dim = world.getDimension(0);
var rect = dim.getExtent();
var x0 = rect.getX() * 128, y0 = rect.getY() * 128;
var width = rect.getWidth() * 128, depth = rect.getHeight() * 128;
print('Map: ' + width + ' x ' + depth + ' blocks (' + rect.getWidth() + ' x ' + rect.getHeight() + ' tiles), heights ' + dim.getLowestIntHeight() + ' to ' + dim.getHighestIntHeight());

var DEPTH = 3, columns = 0, sum = 0, lowest = 1e9, highest = -1e9, flooded = 0;
for (var y = y0; y < y0 + depth; y++) {
    for (var x = x0; x < x0 + width; x++) {
        if (mask.getHeight(x, y) <= 0) {
            continue;
        }
        // The bed: under the lowest ground around, as deep as DEPTH.
        var low = 1e9;
        for (var dy = -1; dy <= 1; dy++) {
            for (var dx = -1; dx <= 1; dx++) {
                low = Math.min(low, dim.getHeightAt(x + dx, y + dy));
            }
        }
        var bed = Math.floor(low) - DEPTH;
        dim.setHeightAt(x, y, bed);
        dim.setWaterLevelAt(x, y, bed + DEPTH - 1);
        columns++;
        sum += bed;
        lowest = Math.min(lowest, bed);
        highest = Math.max(highest, bed);
        if (dim.getWaterLevelAt(x, y) > dim.getIntHeightAt(x, y)) {
            flooded++;
        }
    }
}
print('River: ' + columns + ' columns, beds ' + lowest + ' to ' + highest + ', mean ' + (sum / columns).toFixed(3) + ', ' + flooded + ' under water');

// The river layer over the bed, and gravel under it.
var river = wp.getLayer().fromFile(scriptDir + '/River.layer').go();
wp.applyHeightMap(mask).toWorld(world).applyToLayer(river).fromLevels(1, 255).toLevel(1).go();
wp.applyHeightMap(mask).toWorld(world).applyToTerrain().fromLevel(1).toTerrain(34).go();

var out = arguments.length > 0 ? arguments[0] : scriptDir + '/rivers-out.world';
wp.saveWorld(world).toFile(out).go();
print('Saved');
