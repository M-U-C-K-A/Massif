// Massif's own scripting API: `massif`, beside WorldPainter's `wp`.
//
// Maps are opened, created, read and written by regions of columns at
// once (typed arrays), so that a script working over a whole map does a
// few calls, not one a column. See docs/scripting.md.
"use strict";
(function (global) {
    var m = global.__massif;
    delete global.__massif;
    var wpi = global.__wp;
    var DIMS = { surface: 0, overworld: 0, nether: 1, end: 2 };

    function call(name, args) {
        var out = m.call(name, JSON.stringify(args || {}));
        return out === "null" ? null : JSON.parse(out);
    }
    function dimOf(d) {
        if (d === undefined || d === null) {
            return 0;
        }
        if (typeof d === "string") {
            var k = DIMS[d.toLowerCase()];
            if (k === undefined) {
                throw new Error("Unknown dimension \"" + d + "\": surface, nether or end");
            }
            return k;
        }
        return d;
    }
    function rect(r, what) {
        if (!r || r.length !== 4) {
            throw new Error((what || "A region") + " is [x0, z0, x1, z1]: from the first column to past the last");
        }
        return r;
    }

    function Map(w, d) {
        this._w = w;
        this._d = d || 0;
    }
    Map.prototype = {
        get name() { return call("info", { world: this._w }).name; },
        set name(n) { call("set", { world: this._w, name: String(n) }); },
        info: function () { return call("info", { world: this._w }); },
        set: function (what, a, b, c, d, e) {
            // set(what, [x0, z0, x1, z1], values) or set({ name, seaLevel, seed, spawn })
            if (typeof what === "object" && what !== null && !Array.isArray(what)) {
                var o = { world: this._w };
                for (var k in what) {
                    o[k] = what[k];
                }
                return call("set", o);
            }
            var r = Array.isArray(a) ? a : [a, b, c, d];
            var values = Array.isArray(a) ? b : e;
            rect(r);
            m.regionSet(this._w, this._d, String(what), r, values);
        },
        get: function (what, a, b, c, d) {
            var r = Array.isArray(a) ? a : [a, b, c, d];
            rect(r);
            return m.regionGet(this._w, this._d, String(what), r);
        },
        // One column, for convenience (slow over many: use get and set).
        height: function (x, z) { return m.regionGet(this._w, this._d, "height", [x, z, x + 1, z + 1])[0]; },
        setHeight: function (x, z, h) { m.regionSet(this._w, this._d, "height", [x, z, x + 1, z + 1], new Float32Array([h])); },
        bounds: function () {
            var dim = call("info", { world: this._w }).dims[this._d];
            return dim ? dim.bounds : null;
        },
        // The rectangles of the map's tiles, 128 blocks a side.
        tiles: function () {
            var b = this.bounds();
            var out = [];
            if (!b) {
                return out;
            }
            for (var z = b[1]; z < b[3]; z += 128) {
                for (var x = b[0]; x < b[2]; x += 128) {
                    out.push([x, z, x + 128, z + 128]);
                }
            }
            return out;
        },
        dimension: function (d) { return new Map(this._w, dimOf(d)); },
        terrains: function () { return call("terrains", { world: this._w }); },
        biomes: function () { return call("biomes", { world: this._w }); },
        layers: function () { return call("layers", { world: this._w }); },
        terrainId: function (name) { return call("terrainId", { world: this._w, name: String(name) }); },
        biomeId: function (name) { return call("biomeId", { world: this._w, name: String(name) }); },
        layerId: function (name) { return call("layerId", { world: this._w, name: String(name) }); },
        select: function (r) { call("select", { world: this._w, dim: this._d, rect: r ? rect(r, "A selection") : null }); },
        wholeMap: function (op, options) { return call("wholeMap", { world: this._w, dim: this._d, op: String(op), options: options || {} }); },
        save: function (file) { return call("save", { world: this._w, file: String(file) }); },
        export: function (dir, options) {
            var o = { world: this._w, dir: String(dir) };
            options = options || {};
            if (options.version) {
                o.version = String(options.version);
            }
            o.overwrite = !!options.overwrite;
            return call("export", o);
        },
        // WorldPainter's view of the same map, for its operations.
        wp: function () { return wpi.world(this._w); },
        toString: function () { return "Map " + this.name; }
    };

    var massif = {
        version: m.version(),
        open: function (file) { return new Map(call("open", { file: String(file) })); },
        create: function (options) { return new Map(call("create", { options: options || {} })); },
        // A map from WorldPainter's API: massif.of(world).
        of: function (world) { return new Map(wpi.worldHandle(world)); },
        progress: function (fraction, label) { m.progress(fraction, label === undefined ? null : String(label)); },
        get cancelled() { return m.cancelled(); },
        print: function () { global.print.apply(null, arguments); }
    };
    global.massif = massif;
    global.__massifOpen = function (w, d) { massif.world = new Map(w, d); };
})(globalThis);
