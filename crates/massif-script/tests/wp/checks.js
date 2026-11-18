// Checks a map before export, as a map maker's script does: water that is
// not there, land under the sea without water, cliffs too steep, grass under
// water, and where the frost is. Reports, and fails when it finds holes.
// script.name=Map checks
// script.param.maxSlope.type=float
// script.param.maxSlope.default=4
// script.param.maxSlope.description=Steepest slope allowed, in blocks a block
var world = wp.getWorld().fromFile(arguments[0]).go();
var dim = world.getDimension(0);
var maxSlope = (typeof params !== 'undefined' && params !== null && params['maxSlope']) ? params['maxSlope'] : 4;
var seaLevel = 62;
var rect = dim.getExtent();
var x0 = rect.getX() * 128, y0 = rect.getY() * 128;
var x1 = x0 + rect.getWidth() * 128, y1 = y0 + rect.getHeight() * 128;
var water = 0, holes = 0, cliffs = 0, wetGrass = 0, frost = 0;
var firstHoles = [];
var frostLayer = wp.getLayer().withName('Frost').go();
for (var y = y0; y < y1; y++) {
    for (var x = x0; x < x1; x++) {
        if (!dim.isTilePresent(x >> 7, y >> 7)) {
            continue;
        }
        var h = dim.getIntHeightAt(x, y);
        var level = dim.getWaterLevelAt(x, y);
        if (level > h) {
            water++;
            if (dim.getTerrainAt(x, y) === org.pepsoft.worldpainter.Terrain.GRASS) {
                wetGrass++;
            }
        } else if (h < seaLevel) {
            holes++;
            if (firstHoles.length < 5) {
                firstHoles.push(x + ',' + y + ' (' + h + ')');
            }
        }
        if (dim.getSlope(x, y) > maxSlope) {
            cliffs++;
        }
        if (dim.getBitLayerValueAt(frostLayer, x, y)) {
            frost++;
        }
    }
}
print('Water: ' + water + ' columns, grass under water: ' + wetGrass);
print('Cliffs steeper than ' + maxSlope + ': ' + cliffs);
print('Frost: ' + frost + ' columns');
print('Holes under the sea without water: ' + holes + (firstHoles.length > 0 ? ' — first at ' + firstHoles.join(', ') : ''));
if (holes > 0) {
    print('ERROR: ' + holes + ' holes');
}
