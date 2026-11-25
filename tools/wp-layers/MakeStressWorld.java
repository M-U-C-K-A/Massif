package org.pepsoft.worldpainter.layers.plants;

import org.pepsoft.minecraft.Material;
import org.pepsoft.worldpainter.*;
import org.pepsoft.worldpainter.layers.*;
import org.pepsoft.worldpainter.layers.bo2.Bo2ObjectTube;
import org.pepsoft.worldpainter.layers.bo2.Schem;
import org.pepsoft.worldpainter.layers.groundcover.GroundCoverLayer;
import org.pepsoft.worldpainter.layers.pockets.UndergroundPocketsLayer;
import org.pepsoft.worldpainter.objects.GenericObject;
import org.pepsoft.worldpainter.objects.WPObject;

import java.awt.Color;
import java.io.*;
import java.util.*;

import static org.pepsoft.worldpainter.Dimension.Anchor.NORMAL_DETAIL;

/**
 * Writes a WorldPainter world that stresses the import of overlapping layers,
 * with WorldPainter's own classes, as a big Conquest Reforged map is made:
 *
 *   stress <out.world> [statue.schem...]
 *     8 × 8 tiles; fourteen layers (plants, custom ground covers of Conquest
 *     and snow layers, Frost, custom object layers, a forest, annotations, a
 *     combined layer, custom annotations, pockets) painted in cells of 8 × 8
 *     blocks, cell k holding the layers of the bits of k: every combination
 *     of them, 16,384 in all. Nibble layers vary in strength column by
 *     column (1 to 15). Custom terrains of Conquest blocks in bands.
 *
 *   big <out.world> <tiles a side> [statue.schem...]
 *     A large map: the same layers in broad overlapping patches, as painted
 *     by hand, to time the import, the views and the brushes.
 *
 * The layers' order (bit k is layer k) is printed, for the checks of
 * `massif-legacy/tests/stress.rs`.
 */
public class MakeStressWorld {
    static PlantLayer.PlantSettings plant(int occurrence) {
        PlantLayer.PlantSettings s = new PlantLayer.PlantSettings();
        s.occurrence = (short) occurrence;
        s.growthFrom = 1;
        s.growthTo = 1;
        return s;
    }

    static int index(Plant plant) {
        return Arrays.asList(Plants.ALL_PLANTS).indexOf(plant);
    }

    static MixedMaterial mixed(String name, Object[][] rows) {
        if (rows.length == 1) {
            return new MixedMaterial(name, new MixedMaterial.Row((Material) rows[0][0], (Integer) rows[0][1], 1.0f), -1, null);
        }
        MixedMaterial.Row[] r = new MixedMaterial.Row[rows.length];
        for (int i = 0; i < rows.length; i++) {
            r[i] = new MixedMaterial.Row((Material) rows[i][0], (Integer) rows[i][1], 1.0f);
        }
        return new MixedMaterial(name, r, -1, null);
    }

    static GroundCoverLayer cover(String name, MixedMaterial m, Color c, int thickness, String palette) {
        GroundCoverLayer l = new GroundCoverLayer(name, m, c);
        l.setThickness(thickness);
        l.setPalette(palette);
        return l;
    }

    /** A small tree of Conquest blocks: a trunk of mallorn and a crown of leaves. */
    static WPObject tree(String name, int height) {
        int w = 5, d = 5;
        Material[] data = new Material[w * d * height];
        Material log = Material.get("conquest:mallorn_log", "axis", "y");
        Material leaves = Material.get("conquest:apple_tree_leaves", "persistent", "true");
        for (int z = 0; z < height; z++) {
            for (int y = 0; y < d; y++) {
                for (int x = 0; x < w; x++) {
                    Material m = Material.AIR;
                    if (x == 2 && y == 2 && z < height - 1) {
                        m = log;
                    } else if (z >= height - 3 && Math.abs(x - 2) + Math.abs(y - 2) <= 3) {
                        m = leaves;
                    }
                    data[x + y * w + z * w * d] = m;
                }
            }
        }
        GenericObject o = new GenericObject(name, w, d, height, data);
        o.setAttribute(WPObject.ATTRIBUTE_OFFSET, new javax.vecmath.Point3i(-2, -2, 0));
        return o;
    }

    public static void main(String[] args) throws Exception {
        boolean big = args[0].equals("big");
        File out = new File(args[1]);
        int side = big ? Integer.parseInt(args[2]) : 8;
        List<WPObject> statues = new ArrayList<>();
        for (int i = big ? 3 : 2; i < args.length; i++) {
            statues.add(Schem.load(new File(args[i])));
        }

        Class.forName("org.pepsoft.worldpainter.DefaultPlugin");
        Configuration.setInstance(new Configuration());
        org.pepsoft.worldpainter.plugins.WPPluginManager.initialise(UUID.randomUUID(), WPContext.INSTANCE);

        // Custom terrains of Conquest blocks.
        MixedMaterial grassy = mixed("Conquest meadow", new Object[][] { { Material.get("minecraft:grass_block"), 700 }, { Material.get("conquest:cushion_moss_covered_loamy_dirt"), 300 } });
        MixedMaterial dusty = mixed("Dusty ground", new Object[][] { { Material.get("conquest:dusty_yellow_dirt"), 1000 } });
        Terrain.setCustomMaterial(0, grassy);
        Terrain.setCustomMaterial(1, dusty);

        long seed = 4242;
        TileFactory factory = TileFactoryFactory.createNoiseTileFactory(seed, Terrain.GRASS, -64, 320, 72, 62, false, true, 6f, 1.0);
        World2 world = new World2(DefaultPlugin.JAVA_ANVIL_1_20_5, seed, factory);
        world.setName(big ? "Massif big import" : "Massif overlap stress");
        world.setMixedMaterial(0, grassy);
        world.setMixedMaterial(1, dusty);
        Dimension dim = world.getDimension(NORMAL_DETAIL);
        // Custom biomes: a data pack's, by its id, and one named as a map
        // maker names them.
        dim.setCustomBiomes(Arrays.asList(new org.pepsoft.worldpainter.biomeschemes.CustomBiome("terralith:alpine_highlands", 200, 0x88aa55), new org.pepsoft.worldpainter.biomeschemes.CustomBiome("Misties Biome 8", 201, 0x5577cc)));
        int first = -side / 2;
        for (int ty = first; ty < first + side; ty++) {
            for (int tx = first; tx < first + side; tx++) {
                dim.addTile(factory.createTile(tx, ty));
            }
        }

        // The layers, bit k for layer k.
        PlantLayer grass = new PlantLayer("Grass", "Short grass", new Color(90, 170, 60));
        grass.setSettings(index(Plants.GRASS), plant(100));
        grass.setPalette("Vegetation");
        PlantLayer flowers = new PlantLayer("Flowers", "Poppies and dandelions", new Color(220, 60, 60));
        flowers.setSettings(index(Plants.POPPY), plant(50));
        flowers.setSettings(index(Plants.DANDELION), plant(50));
        flowers.setPalette("Vegetation");
        GroundCoverLayer grassLayer = cover("CR grass layer", mixed("Grass layer", new Object[][] { { Material.get("conquest:grass_block_layer", "layers", "2"), 1000 } }), new Color(110, 160, 70), 1, "Conquest ground");
        GroundCoverLayer snowLayer = cover("Snow layers", mixed("Snow", new Object[][] { { Material.get("minecraft:snow", "layers", "3"), 1000 } }), new Color(240, 240, 250), 1, "Conquest ground");
        GroundCoverLayer sandGrass = cover("CR grass and sand", mixed("Grass and sand", new Object[][] { { Material.get("conquest:grass_and_sand_layer", "layers", "4"), 1000 } }), new Color(190, 180, 110), 1, "Conquest ground");
        Bo2Layer mallorns = new Bo2Layer(new Bo2ObjectTube("Mallorns", Arrays.asList(tree("Mallorn small", 7), tree("Mallorn tall", 11))), "Conquest trees", new Color(40, 110, 40));
        mallorns.setPalette("Trees");
        Bo2Layer statueLayer = statues.isEmpty() ? null : new Bo2Layer(new Bo2ObjectTube("Statues", statues), "Statues", new Color(120, 120, 120));
        if (statueLayer != null) {
            statueLayer.setPalette("Objects");
        }
        PlantLayer ferns = new PlantLayer("Ferns", "Ferns", new Color(60, 120, 60));
        ferns.setSettings(index(Plants.FERN), plant(100));
        GroundCoverLayer debris = cover("CR forest floor", mixed("Forest floor and stones", new Object[][] { { Material.get("conquest:fir_forest_floor_layer", "layers", "1"), 600 }, { Material.get("conquest:small_stones"), 400 } }), new Color(100, 80, 50), 1, "Conquest ground");
        CombinedLayer shire = new CombinedLayer("Shire", "Ferns, forest floor, pines, on dusty ground", new Color(150, 170, 80));
        shire.setLayers(Arrays.asList(ferns, debris, PineForest.INSTANCE));
        Map<Layer, Float> factors = new HashMap<>();
        factors.put(ferns, 1.0f);
        factors.put(debris, 1.0f);
        factors.put(PineForest.INSTANCE, 0.5f);
        shire.setFactors(factors);
        shire.setTerrain(Terrain.CUSTOM_2);
        shire.setBiome(4);
        shire.setPalette("Combined");
        CustomAnnotationLayer roads = new CustomAnnotationLayer("Roads to build", "Planned roads", new Color(230, 60, 40));
        UndergroundPocketsLayer pockets = new UndergroundPocketsLayer("Gravel pockets", mixed("Gravel", new Object[][] { { Material.GRAVEL, 1000 } }), null, 120, -40, 50, 150, new Color(130, 120, 110));
        GroundCoverLayer path = cover("Gravel path", mixed("Path", new Object[][] { { Material.get("conquest:gravel_layer", "layers", "3"), 1000 } }), new Color(140, 130, 120), 1, "Conquest ground");

        Layer[] layers = { grass, flowers, grassLayer, snowLayer, sandGrass, Frost.INSTANCE, mallorns, statueLayer == null ? DeciduousForest.INSTANCE : statueLayer, DeciduousForest.INSTANCE, Annotations.INSTANCE, shire, roads, pockets, path };
        List<CustomLayer> custom = new ArrayList<>();
        for (Layer l : layers) {
            if (l instanceof CustomLayer && !custom.contains(l)) {
                custom.add((CustomLayer) l);
            }
        }
        custom.add(ferns);
        custom.add(debris);
        dim.setCustomLayers(custom);
        for (int k = 0; k < layers.length; k++) {
            System.out.println("layer " + k + " " + layers[k].getName() + " " + layers[k].getDataSize());
        }

        int x0 = first * 128, size = side * 128;
        Random random = new Random(seed);
        // Patches for the big map: discs of each layer, overlapping.
        List<int[]> patches = new ArrayList<>();
        if (big) {
            int n = side * side / 2;
            for (int i = 0; i < n; i++) {
                patches.add(new int[] { random.nextInt(layers.length), x0 + random.nextInt(size), x0 + random.nextInt(size), 20 + random.nextInt(120) });
            }
        }
        for (int z = x0; z < x0 + size; z++) {
            for (int x = x0; x < x0 + size; x++) {
                int band = Math.floorMod(z - x0, 256) / 64;
                dim.setTerrainAt(x, z, band == 1 ? Terrain.CUSTOM_1 : band == 3 ? Terrain.CUSTOM_2 : Terrain.GRASS);
                // Biomes painted in bands across: meadow, the two custom
                // biomes, automatic.
                int biomeBand = Math.floorMod(x - x0, 256) / 64;
                if (biomeBand < 3) {
                    dim.setLayerValueAt(Biome.INSTANCE, x, z, new int[] { 252, 200, 201 }[biomeBand]);
                }
                if (big) {
                    continue;
                }
                int cell = ((z - x0) / 8) * (size / 8) + (x - x0) / 8;
                int strength = 1 + Math.floorMod(x * 7 + z * 13, 15);
                for (int k = 0; k < layers.length; k++) {
                    if ((cell >> k & 1) == 0) {
                        continue;
                    }
                    paint(dim, layers[k], x, z, strength);
                }
            }
        }
        if (big) {
            for (int[] p : patches) {
                Layer l = layers[p[0]];
                int r = p[3];
                for (int z = Math.max(x0, p[2] - r); z < Math.min(x0 + size, p[2] + r); z++) {
                    for (int x = Math.max(x0, p[1] - r); x < Math.min(x0 + size, p[1] + r); x++) {
                        int dx = x - p[1], dz = z - p[2];
                        int d2 = dx * dx + dz * dz;
                        if (d2 < r * r) {
                            paint(dim, l, x, z, Math.max(1, 15 - 14 * d2 / (r * r)));
                        }
                    }
                }
            }
        }
        try (OutputStream o = new BufferedOutputStream(new FileOutputStream(out))) {
            new WorldIO(world).save(o);
        }
        System.out.println("wrote " + out + " (" + side + "×" + side + " tiles)");
    }

    static void paint(Dimension dim, Layer l, int x, int z, int strength) {
        if (l.getDataSize() == Layer.DataSize.BIT || l.getDataSize() == Layer.DataSize.BIT_PER_CHUNK) {
            dim.setBitLayerValueAt(l, x, z, true);
        } else if (l == Annotations.INSTANCE) {
            dim.setLayerValueAt(l, x, z, 1 + strength % 15);
        } else {
            dim.setLayerValueAt(l, x, z, strength);
        }
    }
}
