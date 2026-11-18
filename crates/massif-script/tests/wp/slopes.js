// WorldPainter's slopetest.js: Frost on slopes steeper than 30 degrees,
// and Pine on gentle land between 70 and 120, feathered.
var heightMap = wp.getHeightMap().fromFile(scriptDir + '/heightmap.png').go();
var world = wp.createWorld().fromHeightMap(heightMap).go();
var frostLayer = wp.getLayer().withName('Frost').go();
var slopeFilter = wp.createFilter().aboveDegrees(30).go();
wp.applyLayer(frostLayer).toWorld(world).withFilter(slopeFilter).go();
var pine = wp.getLayer().withName('Pine').go();
var gentle = wp.createFilter().belowDegrees(20).aboveLevel(70).belowLevel(120).feather().onlyOnLand().go();
wp.applyLayer(pine).toWorld(world).toLevel(12).withFilter(gentle).go();
wp.saveWorld(world).toFile(arguments[0]).go();
