// WorldPainter's scripting API on Massif.
//
// What WorldPainter's `ScriptingContext` (`wp`), its operations and the
// Java objects scripts reach (`World2`, `Dimension`, `Terrain`, layers)
// do, written over the natives of `__m` (see js.rs). The operations check
// their arguments as WorldPainter does and fail with its messages; what
// Massif does not do says so ("… is not supported yet"), never nothing.
"use strict";
(function (global) {
    var m = global.__m;
    delete global.__m;

    // Errors named as WorldPainter's Java exceptions, which is what a
    // script's `print(e)` shows.
    function ScriptException(message) {
        var e = new Error(message);
        e.name = "org.pepsoft.worldpainter.tools.scripts.ScriptException";
        return e;
    }
    function IllegalArgument(message) {
        var e = new Error(message);
        e.name = "java.lang.IllegalArgumentException";
        return e;
    }
    function notSupported(what) {
        var e = new Error(what + " is not supported yet");
        e.name = "java.lang.UnsupportedOperationException";
        return e;
    }

    // The line of the script that is running, for errors reported later.
    var scriptFile = m.scriptName();
    function callerLine() {
        var stack = new Error().stack || "";
        var lines = stack.split("\n");
        for (var i = 0; i < lines.length; i++) {
            var k = lines[i].indexOf("(" + scriptFile + ":");
            if (k >= 0) {
                return parseInt(lines[i].substring(k + scriptFile.length + 2), 10) || 0;
            }
        }
        return 0;
    }

    // Make `obj`'s unknown methods fail with a message naming them: its
    // prototype becomes a guard answering every other name.
    function strict(obj, label) {
        var guard = new Proxy(Object.create(null), {
            get: function (target, prop, receiver) {
                if (typeof prop !== "string" || prop in Object.prototype || prop === "then" || prop === "toJSON" || prop.charAt(0) === "_") {
                    return Reflect.get(Object.prototype, prop, receiver);
                }
                return function () {
                    throw notSupported(label + "." + prop + "()");
                };
            },
            has: function (target, prop) { return prop in Object.prototype; }
        });
        Object.setPrototypeOf(obj, guard);
        return obj;
    }

    // Java-like values.
    function JavaList(items) {
        var a = items.slice();
        a.size = function () { return a.length; };
        a.get = function (i) { return a[i]; };
        a.isEmpty = function () { return a.length === 0; };
        a.contains = function (o) { return a.indexOf(o) >= 0; };
        a.toArray = function () { return a.slice(); };
        a.iterator = function () {
            var i = 0;
            return { hasNext: function () { return i < a.length; }, next: function () { return a[i++]; } };
        };
        return a;
    }
    function Point(x, y) {
        this.x = x;
        this.y = y;
    }
    Point.prototype.getX = function () { return this.x; };
    Point.prototype.getY = function () { return this.y; };
    Point.prototype.toString = function () { return "java.awt.Point[x=" + this.x + ",y=" + this.y + "]"; };
    Point.prototype.equals = function (o) { return o instanceof Point && o.x === this.x && o.y === this.y; };
    function Rectangle(x, y, w, h) {
        this.x = x;
        this.y = y;
        this.width = w;
        this.height = h;
    }
    Rectangle.prototype.getX = function () { return this.x; };
    Rectangle.prototype.getY = function () { return this.y; };
    Rectangle.prototype.getWidth = function () { return this.width; };
    Rectangle.prototype.getHeight = function () { return this.height; };
    Rectangle.prototype.getMinX = function () { return this.x; };
    Rectangle.prototype.getMinY = function () { return this.y; };
    Rectangle.prototype.getMaxX = function () { return this.x + this.width; };
    Rectangle.prototype.getMaxY = function () { return this.y + this.height; };
    Rectangle.prototype.contains = function (x, y) { return x >= this.x && y >= this.y && x < this.x + this.width && y < this.y + this.height; };
    Rectangle.prototype.toString = function () { return "java.awt.Rectangle[x=" + this.x + ",y=" + this.y + ",width=" + this.width + ",height=" + this.height + "]"; };

    // Layer.DataSize.
    var DataSize = {};
    ["BIT", "NIBBLE", "BYTE", "BIT_PER_CHUNK", "NONE"].forEach(function (n, k) {
        DataSize[n] = { name: function () { return n; }, ordinal: function () { return k; }, toString: function () { return n; } };
    });
    var BITS = { BIT: 1, BIT_PER_CHUNK: 1, NIBBLE: 4, BYTE: 8 };

    // Terrains: WorldPainter's, by ordinal, each one object so that `===`
    // compares them as Java's enum constants.
    var names = JSON.parse(m.terrainNames());
    function TerrainValue(ordinal, name, display) {
        this._o = ordinal;
        this._name = name;
        this._display = display;
    }
    TerrainValue.prototype.ordinal = function () { return this._o; };
    TerrainValue.prototype.name = function () { return this._name; };
    TerrainValue.prototype.getName = function () { return this._display; };
    TerrainValue.prototype.toString = function () { return this._display; };
    TerrainValue.prototype.isCustom = function () { return this._name.indexOf("CUSTOM_") === 0; };
    TerrainValue.prototype.equals = function (o) { return o === this; };
    TerrainValue.prototype.hashCode = function () { return this._o; };
    strict(TerrainValue.prototype, "terrain");
    var Terrain = { VALUES: [] };
    names.forEach(function (n, k) {
        var display = n.charAt(0) + n.substring(1).toLowerCase().replace(/_/g, " ");
        if (n.indexOf("CUSTOM_") === 0) {
            display = "Custom " + n.substring(7);
        }
        var t = new TerrainValue(k, n, display);
        Terrain.VALUES.push(t);
        Terrain[n] = t;
    });
    Terrain.values = function () { return Terrain.VALUES.slice(); };
    Terrain.valueOf = function (n) {
        if (!Object.prototype.hasOwnProperty.call(Terrain, n) || !(Terrain[n] instanceof TerrainValue)) {
            throw IllegalArgument("No enum constant org.pepsoft.worldpainter.Terrain." + n);
        }
        return Terrain[n];
    };
    Terrain.CUSTOM_TERRAIN_COUNT = 96;
    Terrain.getCustomTerrain = function (index) { return Terrain["CUSTOM_" + (index + 1)]; };
    strict(Terrain, "org.pepsoft.worldpainter.Terrain");
    // Massif's own terrains, which WorldPainter has not: one object each.
    var massifTerrains = {};
    function terrainOf(code, w) {
        if (code >= 0) {
            return Terrain.VALUES[code];
        }
        if (code === -1) {
            return null;
        }
        var key = w + ":" + code;
        if (!massifTerrains[key]) {
            var name = m.terrainName(w, -code - 2);
            massifTerrains[key] = new TerrainValue(-1, name.toUpperCase().replace(/[^A-Z0-9]+/g, "_"), name);
            massifTerrains[key]._massif = -code - 2;
        }
        return massifTerrains[key];
    }
    function terrainCode(t) {
        if (t instanceof TerrainValue) {
            return t._o >= 0 ? t._o : -(t._massif + 2);
        }
        if (typeof t === "number") {
            return t;
        }
        throw IllegalArgument("Not a terrain: " + t);
    }

    // Layers: one object per layer, so that `===` compares them.
    var layerObjects = [];
    function Layer() {}
    Layer.prototype.getName = function () { return this.name; };
    Layer.prototype.getId = function () { return this.name; };
    Layer.prototype.getDataSize = function () { return this.dataSize; };
    Layer.prototype.toString = function () { return this.name; };
    Layer.prototype.equals = function (o) { return o === this; };
    Layer.prototype.hashCode = function () { return this._h; };
    strict(Layer.prototype, "layer");
    function layer(h) {
        if (!layerObjects[h]) {
            var info = JSON.parse(m.layerInfo(h));
            var o = Object.create(Layer.prototype);
            o._h = h;
            o.name = info.name;
            o.dataSize = DataSize[info.size];
            layerObjects[h] = o;
        }
        return layerObjects[h];
    }
    function layerHandle(l, what) {
        if (l === null || l === undefined) {
            throw ScriptException((what || "layer") + " may not be null");
        }
        if (!(l instanceof Layer)) {
            throw IllegalArgument("Not a layer: " + l);
        }
        return l._h;
    }
    function standardLayer(name) {
        return function () { return layer(m.layerByName(name)); };
    }

    // Worlds and dimensions.
    var worldObjects = [];
    var dimensionObjects = {};
    var DIM_NAMES = ["Surface", "Nether", "End"];
    function Anchor(dim, role, invert, id) {
        this.dim = dim;
        this.role = role;
        this.invert = !!invert;
        this.id = id || 0;
    }
    function World2(w) {
        this._w = w;
    }
    World2.prototype.getName = function () { return JSON.parse(m.worldInfo(this._w)).name; };
    World2.prototype.setName = function (n) { m.setWorldName(this._w, String(n)); };
    World2.prototype.getDimension = function (a) {
        var dim = (a instanceof Anchor) ? a.dim : (a === undefined ? 0 : a);
        if (a instanceof Anchor && (a.invert || (a.role && a.role !== "DETAIL"))) {
            throw notSupported("world.getDimension() of a ceiling, cave floor or floating dimension");
        }
        return dimension(this._w, dim);
    };
    World2.prototype.getDimensions = function () {
        var w = this._w;
        return JavaList(JSON.parse(m.worldInfo(w)).dims.map(function (d) { return dimension(w, d); }));
    };
    World2.prototype.getMaxHeight = function () { return JSON.parse(m.worldInfo(this._w)).maxHeight; };
    World2.prototype.getMinHeight = function () { return JSON.parse(m.worldInfo(this._w)).minHeight; };
    World2.prototype.getSpawnPoint = function () { var s = JSON.parse(m.worldInfo(this._w)).spawn; return new Point(s[0], s[2]); };
    World2.prototype.setSpawnPoint = function (p) { m.setSpawn(this._w, p.x, p.y); };
    World2.prototype.getPlatform = function () { var f = JSON.parse(m.worldInfo(this._w)).format; return platform(JSON.parse(m.mapFormat(f, null))); };
    World2.prototype.setPlatform = function (p) { m.setFormat(this._w, p.id); };
    World2.prototype.getMixedMaterial = function (i) { throw notSupported("world.getMixedMaterial()"); };
    World2.prototype.toString = function () { return this.getName(); };
    World2.prototype.equals = function (o) { return o === this; };
    strict(World2.prototype, "world");
    function world(w) {
        if (w === null || w === undefined) {
            return null;
        }
        if (!worldObjects[w]) {
            worldObjects[w] = Object.create(World2.prototype);
            worldObjects[w]._w = w;
        }
        return worldObjects[w];
    }
    function worldHandle(o, what) {
        if (o === null || o === undefined) {
            throw ScriptException((what || "world") + " may not be null");
        }
        if (!(o instanceof World2)) {
            throw IllegalArgument("Not a world: " + o);
        }
        return o._w;
    }

    function Dimension(w, d) {
        this._w = w;
        this._d = d;
        // The calls of the loops over the map go straight to Massif.
        this.getHeightAt = m.height.bind(null, w, d);
        this.setHeightAt = m.setHeight.bind(null, w, d);
        this.getRawHeightAt = m.rawHeight.bind(null, w, d);
        this.getWaterLevelAt = m.water.bind(null, w, d);
        this.setWaterLevelAt = m.setWater.bind(null, w, d);
        this.getSlope = m.slope.bind(null, w, d);
        this.isTilePresent = m.tilePresent.bind(null, w, d);
    }
    Dimension.prototype.getIntHeightAt = function (x, y, dflt) {
        if (x instanceof Point) {
            return m.intHeight(this._w, this._d, x.x, x.y, -2147483648);
        }
        return m.intHeight(this._w, this._d, x, y, dflt === undefined ? -2147483648 : dflt);
    };
    Dimension.prototype.getTerrainAt = function (x, y) { return terrainOf(m.terrainAt(this._w, this._d, x, y), this._w); };
    Dimension.prototype.setTerrainAt = function (x, y, t) {
        if (x instanceof Point) {
            return m.setTerrain(this._w, this._d, x.x, x.y, terrainCode(y));
        }
        m.setTerrain(this._w, this._d, x, y, terrainCode(t));
    };
    Dimension.prototype.getLayerValueAt = function (l, x, y) {
        if (x instanceof Point) {
            return m.layerValue(this._w, this._d, layerHandle(l), x.x, x.y);
        }
        return m.layerValue(this._w, this._d, layerHandle(l), x, y);
    };
    Dimension.prototype.setLayerValueAt = function (l, x, y, v) { m.setLayerValue(this._w, this._d, layerHandle(l), x, y, v); };
    Dimension.prototype.getBitLayerValueAt = function (l, x, y) { return m.layerValue(this._w, this._d, layerHandle(l), x, y) !== 0; };
    Dimension.prototype.setBitLayerValueAt = function (l, x, y, v) { m.setLayerValue(this._w, this._d, layerHandle(l), x, y, v ? 1 : 0); };
    Dimension.prototype.clearLayerData = function (l) {
        if (arguments.length > 1) {
            throw notSupported("dimension.clearLayerData(x, y, excludedLayers)");
        }
        m.clearLayer(this._w, this._d, layerHandle(l));
    };
    Dimension.prototype.getLayersAt = function (x, y) { throw notSupported("dimension.getLayersAt()"); };
    Dimension.prototype.getExtent = function () { var e = JSON.parse(m.extent(this._w, this._d)); return new Rectangle(e.x, e.y, e.width, e.height); };
    Dimension.prototype.getWidth = function () { return JSON.parse(m.extent(this._w, this._d)).width; };
    Dimension.prototype.getHeight = function () { return JSON.parse(m.extent(this._w, this._d)).height; };
    Dimension.prototype.getLowestX = function () { return JSON.parse(m.extent(this._w, this._d)).x; };
    Dimension.prototype.getLowestY = function () { return JSON.parse(m.extent(this._w, this._d)).y; };
    Dimension.prototype.getHighestX = function () { var e = JSON.parse(m.extent(this._w, this._d)); return e.x + e.width - 1; };
    Dimension.prototype.getHighestY = function () { var e = JSON.parse(m.extent(this._w, this._d)); return e.y + e.height - 1; };
    Dimension.prototype.getTileCount = function () { return JSON.parse(m.extent(this._w, this._d)).tiles; };
    Dimension.prototype.getTileCoords = function () {
        return JavaList(JSON.parse(m.tileCoords(this._w, this._d)).map(function (p) { return new Point(p[0], p[1]); }));
    };
    Dimension.prototype.getMinHeight = function () { return JSON.parse(m.dimInfo(this._w, this._d)).minHeight; };
    Dimension.prototype.getMaxHeight = function () { return JSON.parse(m.dimInfo(this._w, this._d)).maxHeight; };
    Dimension.prototype.getLowestHeight = function () { return JSON.parse(m.heightRange(this._w, this._d))[0]; };
    Dimension.prototype.getHighestHeight = function () { return JSON.parse(m.heightRange(this._w, this._d))[1]; };
    Dimension.prototype.getHeightRange = function () { return JSON.parse(m.heightRange(this._w, this._d)); };
    Dimension.prototype.getLowestIntHeight = function () { return Math.round(this.getLowestHeight()); };
    Dimension.prototype.getHighestIntHeight = function () { return Math.round(this.getHighestHeight()); };
    Dimension.prototype.getIntHeightRange = function () { var r = this.getHeightRange(); return [Math.round(r[0]), Math.round(r[1])]; };
    Dimension.prototype.getName = function () { return DIM_NAMES[this._d] || "Dimension"; };
    Dimension.prototype.getWorld = function () { return world(this._w); };
    Dimension.prototype.getAnchor = function () { return new Anchor(this._d, "DETAIL", false, 0); };
    Dimension.prototype.getMinecraftSeed = function () { return JSON.parse(m.worldInfo(this._w)).seed; };
    Dimension.prototype.getSeed = Dimension.prototype.getMinecraftSeed;
    Dimension.prototype.isWater = function (x, y) { return m.water(this._w, this._d, x, y) > this.getIntHeightAt(x, y) && !m.lava(this._w, this._d, x, y); };
    Dimension.prototype.isLava = function (x, y) { return m.water(this._w, this._d, x, y) > this.getIntHeightAt(x, y) && m.lava(this._w, this._d, x, y); };
    Dimension.prototype.getAllLayers = function () {
        return JavaList(JSON.parse(m.worldLayers(this._w, this._d)).map(layer));
    };
    Dimension.prototype.getCustomLayers = Dimension.prototype.getAllLayers;
    Dimension.prototype.setEventsInhibited = function () {};
    Dimension.prototype.isEventsInhibited = function () { return false; };
    Dimension.prototype.toString = function () { return this.getName(); };
    Dimension.prototype.equals = function (o) { return o === this; };
    strict(Dimension.prototype, "dimension");
    function dimension(w, d) {
        var key = w + ":" + d;
        if (!dimensionObjects[key]) {
            if (!m.hasDimension(w, d)) {
                return null;
            }
            dimensionObjects[key] = new Dimension(w, d);
        }
        return dimensionObjects[key];
    }

    // Height maps.
    function HeightMap(h) {
        this._h = h;
        var info = JSON.parse(m.heightMapInfo(h));
        this._name = info.name;
        this._width = info.width;
        this._height = info.height;
    }
    HeightMap.prototype.getName = function () { return this._name; };
    HeightMap.prototype.getWidth = function () { return this._width; };
    HeightMap.prototype.getHeight = function (x, y) {
        if (arguments.length === 0) {
            return this._height;
        }
        return m.heightMapValue(this._h, x, y);
    };
    HeightMap.prototype.getColour = function (x, y) { return m.heightMapColour(this._h, x, y); };
    HeightMap.prototype.getExtent = function () { return new Rectangle(0, 0, this._width, this._height); };
    HeightMap.prototype.toString = function () { return this._name; };
    strict(HeightMap.prototype, "heightMap");

    // Map formats (Platform).
    function platform(p) {
        return { id: p.id, displayName: p.name, getId: function () { return p.id; }, getDisplayName: function () { return p.name; }, toString: function () { return p.name; } };
    }

    // The operations. `go()` has to end each one, as in WorldPainter.
    var goCalled = true, lastCommand = null;
    function command(name) {
        if (!goCalled) {
            throw new Error("You forgot to invoke go() on the " + lastCommand + "() operation");
        }
        goCalled = false;
        lastCommand = name;
    }
    function done() {
        goCalled = true;
    }

    function GetWorldOp() {}
    GetWorldOp.prototype.fromFile = function (f) { this._file = f; return this; };
    GetWorldOp.prototype.go = function () {
        done();
        return world(m.openWorld(this._file === undefined ? "" : String(this._file)));
    };

    function GetLayerOp() {}
    GetLayerOp.prototype.fromFile = function (f) { this._file = f; return this; };
    GetLayerOp.prototype.fromWorld = function (w) { this._world = w; return this; };
    GetLayerOp.prototype.withName = function (n) { this._name = n; return this; };
    GetLayerOp.prototype.go = function () {
        done();
        if (this._file !== undefined && this._file !== null) {
            return layer(m.layerFromFile(String(this._file)));
        }
        if (!this._name) {
            throw ScriptException("name not set");
        }
        if (this._world !== undefined && this._world !== null) {
            return layer(m.layerFromWorld(worldHandle(this._world), String(this._name)));
        }
        return layer(m.layerByName(String(this._name)));
    };

    function MixedMaterial(k) {
        this._k = k;
        this._name = m.materialName(k);
    }
    MixedMaterial.prototype.getName = function () { return this._name; };
    MixedMaterial.prototype.toString = function () { return this._name; };
    function GetTerrainOp() {}
    GetTerrainOp.prototype.fromFile = function (f) { this._file = f; return this; };
    GetTerrainOp.prototype.go = function () {
        done();
        return new MixedMaterial(m.terrainFile(this._file === undefined || this._file === null ? "" : String(this._file)));
    };

    function InstallCustomTerrainOp(t) {
        if (t === null || t === undefined) {
            throw ScriptException("terrain may not be null");
        }
        if (!(t instanceof MixedMaterial)) {
            throw IllegalArgument("Not a custom terrain: " + t);
        }
        this._t = t;
        this._slot = 0;
    }
    InstallCustomTerrainOp.prototype.toWorld = function (w) { this._world = w; return this; };
    InstallCustomTerrainOp.prototype.inSlot = function (i) { this._slot = i; return this; };
    InstallCustomTerrainOp.prototype.go = function () {
        done();
        if (this._world === undefined || this._world === null) {
            throw ScriptException("world not set");
        }
        return m.installTerrain(this._t._k, worldHandle(this._world), this._slot | 0);
    };

    function ImportHeightMapOp() {
        this._o = { scale: 100, dx: 0, dy: 0, waterLevel: 62 };
    }
    ImportHeightMapOp.prototype.fromHeightMap = function (h) { this._o.heightMap = h ? h._h : null; this._hm = h; return this; };
    ImportHeightMapOp.prototype.scale = function (s) { this._o.scale = s; return this; };
    ImportHeightMapOp.prototype.shift = function (x, y) { this._o.dx = x; this._o.dy = y; return this; };
    ImportHeightMapOp.prototype.fromLevels = function (lo, hi) {
        if (this._from) {
            throw ScriptException("Only one mapped range supported, for now");
        }
        this._from = true;
        this._o.imageLow = lo;
        this._o.imageHigh = hi;
        return this;
    };
    ImportHeightMapOp.prototype.toLevels = function (lo, hi) {
        if (this._to) {
            throw ScriptException("Only one mapped range supported, for now");
        }
        this._to = true;
        this._o.worldLow = lo;
        this._o.worldHigh = hi;
        return this;
    };
    ImportHeightMapOp.prototype.withWaterLevel = function (l) { this._o.waterLevel = l; return this; };
    ImportHeightMapOp.prototype.withMapFormat = function (p) { this._o.format = p ? p.id : null; return this; };
    ImportHeightMapOp.prototype.withLowerBuildLimit = function (l) { this._o.lowerBuildLimit = l; return this; };
    ImportHeightMapOp.prototype.withUpperBuildLimit = function (l) { this._o.upperBuildLimit = l; return this; };
    ImportHeightMapOp.prototype.go = function () {
        done();
        if (this._o.heightMap === undefined || this._o.heightMap === null) {
            throw ScriptException("heightMap not set");
        }
        return world(m.createWorld(JSON.stringify(this._o)));
    };

    function SaveWorldOp(w) {
        this._w = worldHandle(w);
    }
    SaveWorldOp.prototype.toFile = function (f) { this._file = f; return this; };
    SaveWorldOp.prototype.go = function () {
        done();
        if (this._file === undefined || this._file === null) {
            throw ScriptException("fileName not set");
        }
        m.saveWorld(this._w, String(this._file));
    };

    function ExportWorldOp(w) {
        this._w = worldHandle(w);
        this._dir = null;
    }
    ExportWorldOp.prototype.toDirectory = function (d) { this._dir = d; return this; };
    ExportWorldOp.prototype.go = function () {
        done();
        m.exportWorld(this._w, this._dir === null || this._dir === undefined ? null : String(this._dir));
    };

    function GetHeightMapOp() {
        this._channel = -1;
    }
    GetHeightMapOp.prototype.fromFile = function (f) {
        if (f === null || f === undefined) {
            throw ScriptException("fileName may not be null");
        }
        this._file = String(f);
        return this;
    };
    GetHeightMapOp.prototype.selectRedChannel = function () { this._channel = 0; return this; };
    GetHeightMapOp.prototype.selectGreenChannel = function () { this._channel = 1; return this; };
    GetHeightMapOp.prototype.selectBlueChannel = function () { this._channel = 2; return this; };
    GetHeightMapOp.prototype.go = function () {
        done();
        return new HeightMap(m.loadHeightMap(this._file === undefined ? "" : this._file, this._channel));
    };

    var TERRAIN_COUNT = Terrain.VALUES.length;
    function MappingOp(call, arg) {
        this._call = call;
        this._hm = null;
        this._layer = null;
        this._terrain = -1;
        this._world = null;
        this._dim = 0;
        this._ceiling = false;
        this._grey = [];
        this._colour = [];
        this._lower = 0;
        this._upper = 0;
        this._storedColour = -1;
        this._scale = 100;
        this._dx = 0;
        this._dy = 0;
        this._mode = "set";
        this._filter = null;
        this._layerValue = 0;
        if (call === "applyHeightMap") {
            if (arg === null || arg === undefined) {
                throw ScriptException("heightMap may not be null");
            }
            this._hm = arg;
        } else if (call === "applyLayer") {
            layerHandle(arg);
            this._layer = arg;
            switch (arg.dataSize.name()) {
                case "BIT":
                case "BIT_PER_CHUNK":
                    this._layerValue = 1;
                    break;
                case "NIBBLE":
                    this._layerValue = 8;
                    break;
                case "BYTE":
                    this._layerValue = 128;
                    break;
                default:
                    throw ScriptException("Layer type " + arg.name + " not supported");
            }
        } else {
            if (typeof arg !== "number" || arg < 0 || arg >= TERRAIN_COUNT) {
                throw ScriptException("Invalid terrain index specified");
            }
            this._terrain = arg;
            this._mode = "terrain";
        }
    }
    MappingOp.prototype.applyToLayer = function (l) {
        this._layer = l;
        if (this._mode === "terrain") {
            this._mode = "set";
        }
        return this;
    };
    MappingOp.prototype.applyToTerrain = function () {
        this._layer = null;
        this._mode = "terrain";
        return this;
    };
    MappingOp.prototype.toWorld = function (w) { this._world = w; return this; };
    MappingOp.prototype.applyToSurface = function () { this._dim = 0; this._ceiling = false; return this; };
    MappingOp.prototype.applyToNether = function () { this._dim = 1; this._ceiling = false; return this; };
    MappingOp.prototype.applyToEnd = function () { this._dim = 2; this._ceiling = false; return this; };
    MappingOp.prototype.applyToSurfaceCeiling = function () { this._dim = 0; this._ceiling = true; return this; };
    MappingOp.prototype.applyToNetherCeiling = function () { this._dim = 1; this._ceiling = true; return this; };
    MappingOp.prototype.applyToEndCeiling = function () { this._dim = 2; this._ceiling = true; return this; };
    MappingOp.prototype.fromLevel = function (level) {
        if (this._colour.length > 0) {
            throw ScriptException("Cannot mix grey scale and colour mapping");
        }
        this._lower = level;
        this._upper = level;
        return this;
    };
    MappingOp.prototype.fromLevels = function (lower, upper) {
        if (this._colour.length > 0) {
            throw ScriptException("Cannot mix grey scale and colour mapping");
        }
        this._lower = lower;
        this._upper = upper;
        return this;
    };
    function rgb(r, g, b) {
        if (r < 0 || r > 255) {
            throw ScriptException("Invalid red value " + r + " specified");
        }
        if (g < 0 || g > 255) {
            throw ScriptException("Invalid green value " + g + " specified");
        }
        if (b < 0 || b > 255) {
            throw ScriptException("Invalid blue value " + b + " specified");
        }
    }
    MappingOp.prototype.fromColour = function (a, r, g, b) {
        if (arguments.length === 3) {
            rgb(a, r, g);
            this._storedColour = ((0xff << 24) | (a << 16) | (r << 8) | g) >>> 0;
        } else {
            if (a < 0 || a > 255) {
                throw ScriptException("Invalid alpha value " + a + " specified");
            }
            rgb(r, g, b);
            this._storedColour = ((a << 24) | (r << 16) | (g << 8) | b) >>> 0;
        }
        return this;
    };
    MappingOp.prototype._mapStoredTo = function (v) {
        if (this._storedColour !== -1) {
            this._colour.push([this._storedColour, v]);
            this._storedColour = -1;
        } else {
            if (this._lower < 0 || this._upper > 65535) {
                throw IllegalArgument("Level out of range (0 to 65535): " + (this._lower < 0 ? this._lower : this._upper));
            }
            this._grey.push([this._lower, this._upper, v, v]);
        }
    };
    MappingOp.prototype.toLevel = function (level) {
        if (level < 0 || level > 255) {
            throw ScriptException("Illegal value for layer: " + level);
        }
        this._mapStoredTo(level);
        this._layerValue = level;
        return this;
    };
    MappingOp.prototype.toTerrain = function (t) {
        t = terrainCode(t);
        if (t < 0 || t >= TERRAIN_COUNT) {
            throw ScriptException("Illegal value for terrain index: " + t);
        }
        this._mapStoredTo(t);
        return this;
    };
    MappingOp.prototype.toLevels = function (lower, upper) {
        if (this._storedColour !== -1) {
            throw IllegalArgument("Cannot map a colour to a range");
        } else if (this._lower === this._upper) {
            if (lower === upper) {
                this._grey.push([this._lower, this._lower, lower, lower]);
            } else {
                throw IllegalArgument("Cannot map a single value to a range");
            }
        } else {
            if (this._lower < 0 || this._upper > 65535) {
                throw IllegalArgument("Level out of range (0 to 65535): " + (this._lower < 0 ? this._lower : this._upper));
            }
            this._grey.push([this._lower, this._upper, lower, upper]);
        }
        return this;
    };
    MappingOp.prototype.setAlways = function () { this._mode = "set"; return this; };
    MappingOp.prototype.setWhenLower = function () { this._mode = "lower"; return this; };
    MappingOp.prototype.setWhenHigher = function () { this._mode = "higher"; return this; };
    MappingOp.prototype.scale = function (s) { this._scale = s; return this; };
    MappingOp.prototype.shift = function (x, y) { this._dx = x; this._dy = y; return this; };
    MappingOp.prototype.withFilter = function (f) { this._filter = f; return this; };
    MappingOp.prototype.go = function () {
        done();
        if (this._hm === null && this._layer === null && this._terrain === -1) {
            throw ScriptException("No data source (heightMap, layer or terrain) specified");
        }
        if (this._mode !== "terrain" && this._layer === null) {
            throw ScriptException("layer not specified");
        }
        if (this._world === null || this._world === undefined) {
            throw ScriptException("world not specified");
        }
        var grey = this._grey.length > 0, colour = this._colour.length > 0;
        var all = this._grey.map(function (r) { return [r[2], r[3]]; }).concat(this._colour.map(function (c) { return [c[1], c[1]]; }));
        if ((grey || colour) && this._hm === null && this._layer === null) {
            throw ScriptException("Mapping specified but no height map or layer specified");
        } else if (this._hm !== null) {
            if (!grey && !colour) {
                throw ScriptException("mapping not specified");
            }
            if (grey && colour) {
                throw ScriptException("Cannot mix grey scale and colour mapping");
            }
            if (this._layer !== null) {
                var size = this._layer.dataSize.name();
                if (size === "NONE") {
                    throw ScriptException("Layer of unsupported type specified: " + this._layer);
                }
                var bits = BITS[size], max = (1 << bits) - 1, layerName = this._layer;
                all.forEach(function (r) {
                    [r[0], r[1]].forEach(function (v) {
                        if (v < -1 || v > max) {
                            throw ScriptException("Invalid destination level " + v + " specified for " + bits + "-bit layer " + layerName);
                        }
                    });
                });
            } else {
                all.forEach(function (r) {
                    [r[0], r[1]].forEach(function (v) {
                        if (v < -1 || v >= TERRAIN_COUNT) {
                            throw ScriptException("Invalid terrain index " + v + " specified");
                        }
                    });
                });
            }
        }
        m.queue(JSON.stringify({
            world: worldHandle(this._world),
            dim: this._dim,
            ceiling: this._ceiling,
            heightMap: this._hm === null ? null : this._hm._h,
            scale: this._scale,
            dx: this._dx,
            dy: this._dy,
            grey: this._grey,
            colour: this._colour,
            layer: this._layer === null ? null : layerHandle(this._layer),
            terrain: this._terrain,
            layerValue: this._layerValue,
            mode: this._mode,
            filter: this._filter === null || this._filter === undefined ? null : this._filter._f,
            line: callerLine(),
            call: this._call
        }));
    };

    function Filter(f) {
        this._f = f;
    }
    function CreateFilterOp() {
        this._o = { feather: false, slopeIsAbove: false, inSelection: false, outsideSelection: false };
        this._only = null;
        this._except = null;
        this._exceptLast = false;
    }
    CreateFilterOp.prototype.aboveLevel = function (l) { this._o.aboveLevel = l; return this; };
    CreateFilterOp.prototype.belowLevel = function (l) { this._o.belowLevel = l; return this; };
    CreateFilterOp.prototype.feather = function () { this._o.feather = true; return this; };
    CreateFilterOp.prototype._setOnly = function (c) {
        if (this._only !== null) {
            throw ScriptException("Only one \"only on\" or condition may be specified");
        }
        this._only = c;
        this._exceptLast = false;
        return this;
    };
    CreateFilterOp.prototype._setExcept = function (c) {
        if (this._except !== null) {
            throw ScriptException("Only one or \"except on\" condition may be specified");
        }
        this._except = c;
        this._exceptLast = true;
        return this;
    };
    function terrainIndex(i) {
        var t = terrainCode(i);
        if (t < 0 || t >= TERRAIN_COUNT) {
            throw new Error("Index " + t + " out of bounds for length " + TERRAIN_COUNT);
        }
        return t;
    }
    CreateFilterOp.prototype.onlyOnTerrain = function (i) { return this._setOnly({ kind: "terrain", index: terrainIndex(i) }); };
    CreateFilterOp.prototype.onlyOnLayer = function (l) { return this._setOnly({ kind: "layer", layer: layerHandle(l), layerObject: l }); };
    CreateFilterOp.prototype.onlyOnBiome = function (b) { return this._setOnly({ kind: "biome", id: b }); };
    CreateFilterOp.prototype.onlyOnAutoBiome = function (b) { return this._setOnly(b === 0 ? { kind: "biome", id: 0 } : { kind: "autoBiome", id: b }); };
    CreateFilterOp.prototype.onlyOnAutoBiomes = function () { return this._setOnly({ kind: "autoBiomes" }); };
    CreateFilterOp.prototype.onlyOnWater = function () { return this._setOnly({ kind: "water" }); };
    CreateFilterOp.prototype.onlyOnLand = function () { return this._setOnly({ kind: "land" }); };
    CreateFilterOp.prototype.exceptOnTerrain = function (i) { return this._setExcept({ kind: "terrain", index: terrainIndex(i) }); };
    CreateFilterOp.prototype.exceptOnLayer = function (l) { return this._setExcept({ kind: "layer", layer: layerHandle(l), layerObject: l }); };
    CreateFilterOp.prototype.exceptOnBiome = function (b) { return this._setExcept({ kind: "biome", id: b }); };
    CreateFilterOp.prototype.exceptOnAutoBiome = function (b) { return this._setExcept(b === 0 ? { kind: "biome", id: 0 } : { kind: "autoBiome", id: b }); };
    CreateFilterOp.prototype.exceptOnAutoBiomes = function () { return this._setExcept({ kind: "autoBiomes" }); };
    CreateFilterOp.prototype.exceptOnWater = function () { return this._setExcept({ kind: "water" }); };
    CreateFilterOp.prototype.exceptOnLand = function () { return this._setExcept({ kind: "land" }); };
    CreateFilterOp.prototype.withValue = function (v) {
        var which = this._exceptLast ? "except on" : "only on";
        var c = this._exceptLast ? this._except : this._only;
        if (c === null || c.kind !== "layer" || c.value !== undefined) {
            throw ScriptException("No \"" + which + "\" layer selected for \"with value\", or more than one \"with value\" specified");
        }
        var size = c.layerObject.dataSize.name();
        if (size === "BIT" || size === "BIT_PER_CHUNK" || size === "NONE") {
            throw ScriptException("A value may only be specified for continuous or discrete layer types");
        }
        c.value = v;
        c.cmp = "equal";
        return this;
    };
    CreateFilterOp.prototype._orHigherLower = function (cmp, words) {
        var which = this._exceptLast ? "except on" : "only on";
        var c = this._exceptLast ? this._except : this._only;
        if (c === null && !this._exceptLast) {
            throw ScriptException("No \"only on\" layer specified for \"" + words + "\"");
        }
        if (c === null || c.kind !== "layer" || c.value === undefined) {
            throw ScriptException("No \"" + which + "\" layer value specified for \"" + words + "\"");
        }
        if (c.cmp !== "equal") {
            throw ScriptException("Only one of \"or lower\" and \"or higher\" may be specified for \"" + which + "\" value");
        }
        c.cmp = cmp;
        return this;
    };
    CreateFilterOp.prototype.orHigher = function () { return this._orHigherLower("higher", "or higher"); };
    CreateFilterOp.prototype.orLower = function () { return this._orHigherLower("lower", "or lower"); };
    CreateFilterOp.prototype.aboveDegrees = function (d) {
        if (d < 0 || d > 90) {
            throw ScriptException("Degrees must be between 0 and 90 (inclusive)");
        }
        if (this._o.degrees !== undefined) {
            throw ScriptException("aboveDegrees and belowDegrees may not both be specified");
        }
        this._o.degrees = d;
        this._o.slopeIsAbove = true;
        return this;
    };
    CreateFilterOp.prototype.belowDegrees = function (d) {
        if (d < 0 || d > 90) {
            throw ScriptException("Degrees must be between 0 and 90 (inclusive)");
        }
        if (this._o.degrees !== undefined) {
            throw ScriptException("aboveDegrees and belowDegrees may not both be specified");
        }
        this._o.degrees = d;
        return this;
    };
    CreateFilterOp.prototype.inSelection = function () {
        if (this._o.outsideSelection) {
            throw ScriptException("inSelection and outsideSelection may not both be specified");
        }
        this._o.inSelection = true;
        return this;
    };
    CreateFilterOp.prototype.outsideSelection = function () {
        if (this._o.inSelection) {
            throw ScriptException("inSelection and outsideSelection may not both be specified");
        }
        this._o.outsideSelection = true;
        return this;
    };
    function plain(c) {
        if (c === null) {
            return null;
        }
        var o = {};
        for (var k in c) {
            if (k !== "layerObject") {
                o[k] = c[k];
            }
        }
        return o;
    }
    CreateFilterOp.prototype.go = function () {
        done();
        var o = this._o;
        o.onlyOn = plain(this._only);
        o.exceptOn = plain(this._except);
        return new Filter(m.filter(JSON.stringify(o)));
    };

    function GetPlatformOp() {}
    GetPlatformOp.prototype.withId = function (id) { this._id = id; return this; };
    GetPlatformOp.prototype.withName = function (n) { this._name = n; return this; };
    GetPlatformOp.prototype.go = function () {
        done();
        var id = this._id === undefined ? null : this._id, name = this._name === undefined ? null : this._name;
        if (id !== null && name !== null) {
            throw ScriptException("Both name and id set");
        }
        if (id === null && name === null) {
            throw ScriptException("name or id not set");
        }
        return platform(JSON.parse(m.mapFormat(id, name)));
    };

    function GetBiomeIdOp() {}
    GetBiomeIdOp.prototype.fromWorld = function (w) { this._world = w; return this; };
    GetBiomeIdOp.prototype.withName = function (n) { this._name = n; return this; };
    GetBiomeIdOp.prototype.go = function () {
        done();
        if (this._name === undefined || this._name === null) {
            throw ScriptException("name not set");
        }
        return m.biomeId(String(this._name), this._world ? worldHandle(this._world) : -1);
    };

    [[GetWorldOp, "getWorld"], [GetLayerOp, "getLayer"], [GetTerrainOp, "getTerrain"], [ImportHeightMapOp, "createWorld"], [ExportWorldOp, "exportWorld"], [SaveWorldOp, "saveWorld"], [GetHeightMapOp, "getHeightMap"],
        [MappingOp, "applyHeightMap"], [InstallCustomTerrainOp, "installCustomTerrain"], [CreateFilterOp, "createFilter"], [GetPlatformOp, "getMapFormat"], [GetBiomeIdOp, "getBiomeId"]].forEach(function (c) {
        strict(c[0].prototype, "wp." + c[1] + "(…)");
    });
    function op(name, make) {
        return function () {
            command(name);
            return make.apply(null, arguments);
        };
    }
    function make(Ctor) {
        return function (a) { return new Ctor(a); };
    }

    var wp = {
        getVersion: function () { return m.version(); },
        getWorld: op("getWorld", make(GetWorldOp)),
        getLayer: op("getLayer", make(GetLayerOp)),
        getTerrain: op("getTerrain", make(GetTerrainOp)),
        createWorld: op("createWorld", make(ImportHeightMapOp)),
        exportWorld: op("exportWorld", make(ExportWorldOp)),
        saveWorld: op("saveWorld", make(SaveWorldOp)),
        mergeWorld: op("mergeWorld", function () { return { go: function () { done(); throw notSupported("wp.mergeWorld()"); } }; }),
        getHeightMap: op("getHeightMap", make(GetHeightMapOp)),
        applyHeightMap: op("applyHeightMap", function (h) { return new MappingOp("applyHeightMap", h); }),
        applyLayer: op("applyLayer", function (l) { return new MappingOp("applyLayer", l); }),
        applyTerrain: op("applyTerrain", function (t) { return new MappingOp("applyTerrain", terrainCode(t)); }),
        installCustomTerrain: op("installCustomTerrain", make(InstallCustomTerrainOp)),
        createFilter: op("createFilter", make(CreateFilterOp)),
        getMapFormat: op("getMapFormat", make(GetPlatformOp)),
        getBiomeId: op("getBiomeId", make(GetBiomeIdOp)),
        isInterrupted: function () { return m.cancelled(); },
        checkForInterrupt: function () {
            if (m.cancelled()) {
                throw new Error("Script execution interrupted by user");
            }
        },
        checkGoCalled: function (name) { command(name); },
        toString: function () { return "org.pepsoft.worldpainter.tools.scripts.ScriptingContext"; }
    };
    // Java bean properties, as Nashorn shows them: `wp.version`,
    // `wp.world.fromFile(…)`, `wp.interrupted`…
    ["World", "Layer", "Terrain", "HeightMap", "MapFormat", "BiomeId"].forEach(function (n) {
        Object.defineProperty(wp, n.charAt(0).toLowerCase() + n.substring(1), { get: function () { return wp["get" + n](); } });
    });
    Object.defineProperty(wp, "version", { get: function () { return m.version(); } });
    Object.defineProperty(wp, "interrupted", { get: function () { return m.cancelled(); } });
    strict(wp, "wp");

    // The Java classes scripts reach by name.
    function layerClass(name) {
        var c = {};
        Object.defineProperty(c, "INSTANCE", { get: standardLayer(name) });
        return c;
    }
    var classes = {
        "org.pepsoft.worldpainter.Terrain": Terrain,
        "org.pepsoft.worldpainter.layers.Annotations": layerClass("Annotations"),
        "org.pepsoft.worldpainter.layers.Biome": layerClass("Biomes"),
        "org.pepsoft.worldpainter.layers.Frost": layerClass("Frost"),
        "org.pepsoft.worldpainter.layers.Void": layerClass("Void"),
        "org.pepsoft.worldpainter.layers.FloodWithLava": layerClass("Lava"),
        "org.pepsoft.worldpainter.layers.Caverns": layerClass("Caverns"),
        "org.pepsoft.worldpainter.layers.Chasms": layerClass("Chasms"),
        "org.pepsoft.worldpainter.layers.Caves": layerClass("Caves"),
        "org.pepsoft.worldpainter.layers.Populate": layerClass("Populate"),
        "org.pepsoft.worldpainter.layers.ReadOnly": layerClass("Read Only"),
        "org.pepsoft.worldpainter.layers.Resources": layerClass("Resources"),
        "org.pepsoft.worldpainter.layers.DeciduousForest": layerClass("Deciduous"),
        "org.pepsoft.worldpainter.layers.PineForest": layerClass("Pine"),
        "org.pepsoft.worldpainter.layers.Jungle": layerClass("Jungle"),
        "org.pepsoft.worldpainter.layers.SwampLand": layerClass("Swamp"),
        "org.pepsoft.worldpainter.layers.Layer": { DataSize: DataSize },
        "org.pepsoft.worldpainter.layers.Layer$DataSize": DataSize,
        "org.pepsoft.worldpainter.Constants": { DIM_NORMAL: 0, DIM_NETHER: 1, DIM_END: 2, TILE_SIZE: 128, TILE_SIZE_BITS: 7, TILE_SIZE_MASK: 127, DEFAULT_WATER_LEVEL: 62 },
        "org.pepsoft.worldpainter.Dimension$Anchor": Anchor,
        "org.pepsoft.worldpainter.Dimension$Role": { DETAIL: "DETAIL", MASTER: "MASTER", CAVE_FLOOR: "CAVE_FLOOR", FLOATING_FLOOR: "FLOATING_FLOOR" },
        "java.awt.Point": Point,
        "java.awt.Rectangle": Rectangle,
        "java.lang.System": { out: { println: function (s) { m.print(s === undefined ? "" : String(s)); }, print: function (s) { m.print(String(s)); } }, err: { println: function (s) { m.printError(s === undefined ? "" : String(s)); } }, currentTimeMillis: function () { return Date.now(); }, nanoTime: function () { return Date.now() * 1e6; } },
        "java.lang.Math": Math,
        "java.lang.Integer": { MAX_VALUE: 2147483647, MIN_VALUE: -2147483648, parseInt: function (s) { return parseInt(s, 10); }, valueOf: function (s) { return parseInt(s, 10); } },
        "java.lang.Float": { MAX_VALUE: 3.4028234663852886e38, MIN_VALUE: 1.401298464324817e-45, parseFloat: parseFloat },
        "java.lang.Double": { MAX_VALUE: Number.MAX_VALUE, MIN_VALUE: Number.MIN_VALUE, parseDouble: parseFloat }
    };
    function pkg(path) {
        return new Proxy(function () {}, {
            get: function (target, prop) {
                if (typeof prop !== "string") {
                    return undefined;
                }
                var full = path + "." + prop;
                if (Object.prototype.hasOwnProperty.call(classes, full)) {
                    return classes[full];
                }
                return pkg(full);
            },
            apply: function () { throw notSupported("The Java class " + path); },
            construct: function () { throw notSupported("The Java class " + path); }
        });
    }
    // `Packages.org.…` and `org.…`, `java.…`: packages down to a class.
    global.Packages = new Proxy(function () {}, {
        get: function (target, prop) { return typeof prop === "string" ? pkg(prop) : undefined; }
    });
    ["org", "java", "javax", "com", "net"].forEach(function (root) {
        global[root] = pkg(root);
    });
    global.Java = {
        type: function (name) {
            if (Object.prototype.hasOwnProperty.call(classes, name)) {
                return classes[name];
            }
            throw notSupported("The Java class " + name);
        },
        from: function (a) { return Array.prototype.slice.call(a); },
        to: function (a) { return Array.prototype.slice.call(a); }
    };

    // Output.
    function text(args) {
        return Array.prototype.map.call(args, function (a) {
            if (typeof a === "object" && a !== null && !(a instanceof Error) && typeof a.toString !== "function") {
                try { return JSON.stringify(a); } catch (e) { return String(a); }
            }
            return String(a);
        }).join(" ");
    }
    global.print = function () { m.print(text(arguments)); };
    global.console = {
        log: function () { m.print(text(arguments)); },
        info: function () { m.print(text(arguments)); },
        warn: function () { m.printWarning(text(arguments)); },
        error: function () { m.printError(text(arguments)); }
    };
    global.exit = function (code) {
        var e = new Error("exit(" + (code === undefined ? 0 : code) + ")");
        e.name = "Exit";
        e.exitCode = code === undefined ? 0 : code;
        throw e;
    };

    global.wp = wp;
    global.DataSize = DataSize;
    global.Terrain = Terrain;

    // For massif.js and the runner.
    global.__wp = {
        world: world,
        dimension: dimension,
        layer: layer,
        layerHandle: layerHandle,
        worldHandle: worldHandle,
        terrainOf: terrainOf,
        terrainCode: terrainCode,
        HeightMap: HeightMap,
        Point: Point,
        Rectangle: Rectangle,
        notSupported: notSupported,
        callerLine: callerLine,
        end: function () {
            if (!goCalled) {
                throw new Error("You forgot to invoke go() on the " + lastCommand + "() operation");
            }
        }
    };
})(globalThis);
