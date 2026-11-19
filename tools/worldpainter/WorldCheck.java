import org.pepsoft.worldpainter.*;
import org.pepsoft.worldpainter.exporting.JavaWorldExporter;
import org.pepsoft.worldpainter.layers.*;

import java.io.*;
import java.nio.charset.StandardCharsets;
import java.util.*;

/**
 * Opens a .world with WorldPainter's own loading code, the way its Open
 * does, and tells what WorldPainter sees in it. Optionally checks columns
 * against what Massif says they are, saves it again with WorldPainter's
 * Save, and exports it as a Minecraft map with WorldPainter's exporter.
 *
 * Usage: WorldCheck <file.world> [--expect <columns.tsv>] [--compare <other.world>] [--save <out.world>] [--export <folder> [--count <block,block…|all>]]
 *        WorldCheck --count-map <Minecraft map folder> <platform id>
 *
 * columns.tsv, one column a line: dimension x z height terrain water biome
 * layers, the layers as name=value pairs joined by ';' ("-" for none), the
 * water "dry" where there is none. Exits 1 on any difference.
 *
 * Run it with a home of its own (-Duser.home=...) so that WorldPainter's
 * configuration of the user is never read or changed; see check.sh.
 */
public class WorldCheck extends AbstractTool {
    public static void main(String[] args) throws Exception {
        if (args[0].equals("--count-map")) {
            // A Minecraft map's blocks, read with WorldPainter's code:
            // --count-map <map folder> <platform id>.
            initialisePlatform();
            Platform platform = Platform.getById(args[2]);
            countBlocks(org.pepsoft.worldpainter.plugins.PlatformManager.getInstance().getChunkStore(platform, new File(args[1]), 0), "all");
            System.exit(0);
        }
        String file = args[0], expect = null, save = null, export = null, count = null, compare = null;
        for (int i = 1; i < args.length; i += 2) {
            switch (args[i]) {
                case "--expect": expect = args[i + 1]; break;
                case "--save": save = args[i + 1]; break;
                case "--export": export = args[i + 1]; break;
                case "--count": count = args[i + 1]; break;
                case "--compare": compare = args[i + 1]; break;
                default: throw new IllegalArgumentException(args[i]);
            }
        }
        initialisePlatform();
        long start = System.currentTimeMillis();
        WorldIO io = new WorldIO();
        try (InputStream in = new BufferedInputStream(new FileInputStream(file), 1 << 20)) {
            io.load(in);
        }
        World2 world = io.getWorld();
        System.out.println("loaded in " + (System.currentTimeMillis() - start) + " ms");
        // As WorldPainter's Open does: the world's custom terrains.
        for (int i = 0; i < Terrain.CUSTOM_TERRAIN_COUNT; i++) {
            Terrain.setCustomMaterial(i, world.getMixedMaterial(i));
        }
        System.out.println("world \"" + world.getName() + "\" platform " + world.getPlatform().id + " (" + world.getPlatform().displayName + ") heights " + world.getMinHeight() + ".." + world.getMaxHeight()
                + " spawn " + world.getSpawnPoint().x + "," + world.getSpawnPoint().y + " game " + world.getGameType() + " difficulty " + world.getDifficulty());
        // The warnings WorldPainter's Open shows (package private).
        java.lang.reflect.Method warnings = World2.class.getDeclaredMethod("getWarnings");
        warnings.setAccessible(true);
        Object found = warnings.invoke(world);
        if ((found != null) && (! ((Collection<?>) found).isEmpty())) {
            System.out.println("warnings " + found);
        }
        int customs = 0;
        for (int i = 0; i < Terrain.CUSTOM_TERRAIN_COUNT; i++) {
            MixedMaterial m = world.getMixedMaterial(i);
            if (m != null) {
                customs++;
                System.out.println("custom terrain " + (i + 1) + " \"" + m.getName() + "\" " + m.getMode() + " " + Arrays.toString(m.getRows()));
            }
        }
        List<Dimension> dimensions = new ArrayList<>(world.getDimensions());
        dimensions.sort(Comparator.comparing(d -> d.getAnchor().toString()));
        for (Dimension dim : dimensions) {
            Map<String, Integer> terrains = new TreeMap<>();
            Map<String, Integer> layerTiles = new TreeMap<>();
            float lowest = Float.MAX_VALUE, highest = -Float.MAX_VALUE;
            long water = 0;
            for (Tile tile : dim.getTiles()) {
                for (Layer layer : tile.getLayers()) {
                    layerTiles.merge(layer.getName(), 1, Integer::sum);
                }
                for (int x = 0; x < 128; x += 8) {
                    for (int y = 0; y < 128; y += 8) {
                        terrains.merge(tile.getTerrain(x, y).getName(), 1, Integer::sum);
                        float h = tile.getHeight(x, y);
                        lowest = Math.min(lowest, h);
                        highest = Math.max(highest, h);
                        if (tile.getWaterLevel(x, y) > tile.getIntHeight(x, y)) {
                            water++;
                        }
                    }
                }
            }
            System.out.println("dimension " + dim.getName() + " " + dim.getAnchor() + " tiles " + dim.getTileCount() + " heights " + dim.getMinHeight() + ".." + dim.getMaxHeight()
                    + " land " + lowest + ".." + highest + " wet " + water + " generator " + dim.getGenerator() + " seed " + dim.getMinecraftSeed());
            System.out.println("  terrains (1 in 64 columns) " + terrains);
            System.out.println("  layers (tiles) " + layerTiles);
            for (CustomLayer layer : dim.getCustomLayers()) {
                System.out.println("  custom layer \"" + layer.getName() + "\" " + layer.getClass().getSimpleName() + " " + layer.getDataSize());
            }
            if (dim.getCustomBiomes() != null) {
                for (org.pepsoft.worldpainter.biomeschemes.CustomBiome b : dim.getCustomBiomes()) {
                    System.out.println("  custom biome " + b.getId() + " " + b.getName());
                }
            }
        }
        int failures = 0;
        if (expect != null) {
            failures = check(world, expect);
        }
        if (compare != null) {
            compare(world, compare);
        }
        if (save != null) {
            long t = System.currentTimeMillis();
            try (OutputStream out = new FileOutputStream(save)) {
                new WorldIO(world).save(out);
            }
            System.out.println("saved again by WorldPainter in " + (System.currentTimeMillis() - t) + " ms: " + save);
        }
        if (export != null) {
            long t = System.currentTimeMillis();
            File dir = new File(export);
            dir.mkdirs();
            JavaWorldExporter exporter = new JavaWorldExporter(world, null);
            File backups = exporter.selectBackupDir(dir, world.getName());
            Map<Integer, ?> stats = exporter.export(dir, world.getName(), backups, null);
            System.out.println("exported by WorldPainter in " + (System.currentTimeMillis() - t) + " ms to " + new File(dir, world.getName()));
            if (count != null) {
                // The blocks of the map WorldPainter wrote, read back from
                // its region files.
                countBlocks(org.pepsoft.worldpainter.plugins.PlatformManager.getInstance().getChunkStore(world.getPlatform(), new File(dir, world.getName()), 0), count);
            }
        }
        System.exit(failures == 0 ? 0 : 1);
    }

    /** Counts the blocks of a map: "all", or the blocks named. */
    static void countBlocks(org.pepsoft.minecraft.ChunkStore store, String count) {
        // Chunks are visited on several threads.
        Map<String, java.util.concurrent.atomic.LongAdder> blocks = new java.util.concurrent.ConcurrentHashMap<>();
        final boolean all = count.equals("all");
        if (! all) for (String name : count.split(",")) blocks.put(name.contains(":") ? name : "minecraft:" + name, new java.util.concurrent.atomic.LongAdder());
        java.util.concurrent.atomic.LongAdder chunks = new java.util.concurrent.atomic.LongAdder();
        store.visitChunks(chunk -> {
            chunks.increment();
            for (int x = 0; x < 16; x++) for (int z = 0; z < 16; z++) for (int y = chunk.getMinHeight(); y < chunk.getMaxHeight(); y++) {
                org.pepsoft.minecraft.Material m = chunk.getMaterial(x, y, z);
                if ((m != null) && (all || blocks.containsKey(m.name))) blocks.computeIfAbsent(m.name, k -> new java.util.concurrent.atomic.LongAdder()).increment();
            }
            return true;
        });
        System.out.println("blocks in " + chunks.sum() + " chunks:");
        blocks.entrySet().stream().sorted((a, b) -> Long.compare(b.getValue().sum(), a.getValue().sum())).forEach(e -> System.out.println("  " + e.getKey() + " " + e.getValue().sum()));
    }

    /** Compares the surface of this world with another's, column by column (one in 4 × 4), as WorldPainter sees both. */
    static void compare(World2 world, String file) throws Exception {
        WorldIO io = new WorldIO();
        try (InputStream in = new BufferedInputStream(new FileInputStream(file), 1 << 20)) {
            io.load(in);
        }
        World2 other = io.getWorld();
        Dimension a = world.getDimension(Dimension.Anchor.NORMAL_DETAIL), b = other.getDimension(Dimension.Anchor.NORMAL_DETAIL);
        // Each world's custom terrains by their names.
        Map<String, Long> differences = new TreeMap<>();
        Map<String, Long> terrainPairs = new TreeMap<>();
        long columns = 0, missing = 0;
        double heightError = 0;
        for (Tile tile : a.getTiles()) {
            Tile t2 = b.getTile(tile.getX(), tile.getY());
            if (t2 == null) { missing++; continue; }
            for (int x = 0; x < 128; x += 4) {
                for (int y = 0; y < 128; y += 4) {
                    columns++;
                    float h1 = tile.getHeight(x, y), h2 = t2.getHeight(x, y);
                    if (Math.abs(h1 - h2) > 0.5f / 256) { differences.merge("height", 1L, Long::sum); heightError = Math.max(heightError, Math.abs(h1 - h2)); }
                    String n1 = name(world, tile.getTerrain(x, y)), n2 = name(other, t2.getTerrain(x, y));
                    if (! n1.equals(n2)) { differences.merge("terrain", 1L, Long::sum); terrainPairs.merge(n1 + " -> " + n2, 1L, Long::sum); }
                    boolean wet1 = tile.getWaterLevel(x, y) > tile.getIntHeight(x, y), wet2 = t2.getWaterLevel(x, y) > t2.getIntHeight(x, y);
                    if (wet1 != wet2 || (wet1 && tile.getWaterLevel(x, y) != t2.getWaterLevel(x, y))) differences.merge("water", 1L, Long::sum);
                    if (tile.getBitLayerValue(org.pepsoft.worldpainter.layers.FloodWithLava.INSTANCE, x, y) != t2.getBitLayerValue(org.pepsoft.worldpainter.layers.FloodWithLava.INSTANCE, x, y)) differences.merge("lava", 1L, Long::sum);
                    if (tile.getLayerValue(Biome.INSTANCE, x, y) != t2.getLayerValue(Biome.INSTANCE, x, y)) differences.merge("biome", 1L, Long::sum);
                    if (tile.getBitLayerValue(org.pepsoft.worldpainter.layers.Frost.INSTANCE, x, y) != t2.getBitLayerValue(org.pepsoft.worldpainter.layers.Frost.INSTANCE, x, y)) differences.merge("frost", 1L, Long::sum);
                }
            }
        }
        System.out.println("compared " + columns + " columns of " + a.getTileCount() + " tiles with " + file + " (" + b.getTileCount() + " tiles, " + missing + " missing): differences " + differences + (heightError > 0 ? " largest height difference " + heightError : ""));
        terrainPairs.entrySet().stream().sorted((p, q) -> Long.compare(q.getValue(), p.getValue())).limit(15).forEach(e -> System.out.println("  terrain " + e.getKey() + ": " + e.getValue()));
    }

    static String name(World2 world, Terrain terrain) {
        return terrain.isCustom() ? "custom \"" + world.getMixedMaterial(terrain.getCustomTerrainIndex()).getName() + "\"" : terrain.name();
    }

    static int check(World2 world, String file) throws IOException {
        int lines = 0, failures = 0;
        for (String line : new String(new FileInputStream(file).readAllBytes(), StandardCharsets.UTF_8).split("\n")) {
            if (line.isBlank() || line.startsWith("#")) continue;
            lines++;
            String[] f = line.split("\t");
            int dimNo = Integer.parseInt(f[0]), x = Integer.parseInt(f[1]), y = Integer.parseInt(f[2]);
            Dimension dim = world.getDimension(new Dimension.Anchor(dimNo, Dimension.Role.DETAIL, false, 0));
            List<String> wrong = new ArrayList<>();
            float height = dim.getHeightAt(x, y);
            if (Math.abs(height - Float.parseFloat(f[3])) > 0.5f / 256) wrong.add("height " + height + " not " + f[3]);
            Terrain terrain = dim.getTerrainAt(x, y);
            if (! terrain.name().equals(f[4])) wrong.add("terrain " + terrain.name() + " not " + f[4]);
            int level = dim.getWaterLevelAt(x, y);
            if (f[5].equals("dry")) {
                if (level > dim.getIntHeightAt(x, y)) wrong.add("water " + level + " not dry");
            } else if (level != Integer.parseInt(f[5])) {
                wrong.add("water " + level + " not " + f[5]);
            }
            int biome = dim.getLayerValueAt(Biome.INSTANCE, x, y);
            if (biome != Integer.parseInt(f[6])) wrong.add("biome " + biome + " not " + f[6]);
            Map<String, Integer> expected = new HashMap<>();
            if (! f[7].equals("-")) {
                for (String kv : f[7].split(";")) {
                    String[] p = kv.split("=");
                    expected.put(p[0], Integer.parseInt(p[1]));
                }
            }
            for (Layer layer : dim.getAllLayers(false)) {
                if (layer instanceof Biome) continue;
                int value = ((layer.getDataSize() == Layer.DataSize.BIT) || (layer.getDataSize() == Layer.DataSize.BIT_PER_CHUNK))
                        ? (dim.getBitLayerValueAt(layer, x, y) ? 1 : 0)
                        : dim.getLayerValueAt(layer, x, y);
                int want = expected.getOrDefault(layer.getName(), layer.getDefaultValue());
                if (value != want) wrong.add(layer.getName() + " " + value + " not " + want);
                expected.remove(layer.getName());
            }
            expected.forEach((name, value) -> { if (value != 0) wrong.add("no layer " + name); });
            if (! wrong.isEmpty()) {
                failures++;
                if (failures <= 20) System.out.println("MISMATCH at " + x + "," + y + ": " + wrong);
            }
        }
        System.out.println("checked " + lines + " columns: " + failures + " different");
        return failures;
    }
}
