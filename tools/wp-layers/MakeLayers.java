package org.pepsoft.worldpainter.layers.plants;

import org.pepsoft.minecraft.Material;
import org.pepsoft.worldpainter.DefaultPlugin;
import org.pepsoft.worldpainter.MixedMaterial;
import org.pepsoft.worldpainter.Terrain;
import org.pepsoft.worldpainter.layers.*;
import org.pepsoft.worldpainter.layers.groundcover.GroundCoverLayer;
import org.pepsoft.worldpainter.layers.pockets.UndergroundPocketsLayer;
import org.pepsoft.worldpainter.layers.tunnel.TunnelLayer;

import java.awt.Color;
import java.io.*;
import java.util.*;
import java.util.zip.GZIPOutputStream;

/** Writes one .layer file of each kind of custom layer, as WorldPainter's Export does. */
public class MakeLayers {
    static void write(File dir, String name, CustomLayer layer) throws IOException {
        try (ObjectOutputStream out = new ObjectOutputStream(new GZIPOutputStream(new BufferedOutputStream(new FileOutputStream(new File(dir, name + ".layer")))))) {
            out.writeObject(layer);
        }
        System.out.println("wrote " + name + ".layer");
    }

    static PlantLayer.PlantSettings plant(int occurrence, int from, int to) {
        PlantLayer.PlantSettings s = new PlantLayer.PlantSettings();
        s.occurrence = (short) occurrence;
        s.growthFrom = from;
        s.growthTo = to;
        return s;
    }

    static int index(Plant plant) {
        return Arrays.asList(Plants.ALL_PLANTS).indexOf(plant);
    }

    public static void main(String[] args) throws Exception {
        File dir = new File(args[0]);
        // Plants: grass, poppies, dandelions, and tall grass of two blocks.
        PlantLayer meadow = new PlantLayer("Meadow", "Grass and flowers", new Color(120, 180, 60));
        meadow.setSettings(index(Plants.GRASS), plant(60, 1, 1));
        meadow.setSettings(index(Plants.POPPY), plant(10, 1, 1));
        meadow.setSettings(index(Plants.DANDELION), plant(8, 1, 1));
        meadow.setSettings(index(Plants.TALL_GRASS), plant(12, 2, 2));
        meadow.setOnlyOnValidBlocks(true);
        write(dir, "Meadow", meadow);
        // A ground cover of two blocks of snow block.
        MixedMaterial snow = new MixedMaterial("Snow block", new MixedMaterial.Row(Material.SNOW_BLOCK, 1000, 1.0f), -1, null);
        GroundCoverLayer cover = new GroundCoverLayer("Snow cover", snow, new Color(240, 240, 255));
        cover.setThickness(2);
        write(dir, "Snow cover", cover);
        // Pockets of gravel and clay.
        MixedMaterial gravel = new MixedMaterial("Gravel and clay", new MixedMaterial.Row[] { new MixedMaterial.Row(Material.GRAVEL, 750, 1.0f), new MixedMaterial.Row(Material.CLAY, 250, 1.0f) }, -1, null);
        UndergroundPocketsLayer pockets = new UndergroundPocketsLayer("Gravel pockets", gravel, null, 120, -40, 50, 150, new Color(130, 120, 110));
        write(dir, "Gravel pockets", pockets);
        // A tunnel.
        TunnelLayer mine = new TunnelLayer("Mine", TunnelLayer.LayerMode.CAVE, new Color(90, 70, 50), DefaultPlugin.JAVA_ANVIL_1_20_5);
        mine.setFloorLevel(12);
        mine.setRoofLevel(5);
        write(dir, "Mine", mine);
        // Annotations of one's own.
        CustomAnnotationLayer roads = new CustomAnnotationLayer("Roads to build", "Where the roads go", new Color(230, 60, 40));
        write(dir, "Roads to build", roads);
        // A combined layer: frost at half, the meadow, the pockets, on grass, in a forest.
        CombinedLayer snowy = new CombinedLayer("Snowy meadow", "Frost, plants and pockets", new Color(200, 220, 240));
        snowy.setLayers(Arrays.asList(Frost.INSTANCE, meadow, pockets));
        Map<Layer, Float> factors = new HashMap<>();
        factors.put(Frost.INSTANCE, 0.5f);
        factors.put(meadow, 1.0f);
        factors.put(pockets, 0.75f);
        snowy.setFactors(factors);
        snowy.setTerrain(Terrain.GRASS);
        snowy.setBiome(4);
        write(dir, "Snowy meadow", snowy);
        // A combined layer with a custom terrain of its own.
        MixedMaterial moss = new MixedMaterial("Mossy stone", new MixedMaterial.Row[] { new MixedMaterial.Row(Material.get("minecraft:moss_block"), 600, 1.0f), new MixedMaterial.Row(Material.get("minecraft:mossy_cobblestone"), 400, 1.0f) }, -1, null);
        Terrain.setCustomMaterial(0, moss);
        CombinedLayer ruins = new CombinedLayer("Mossy ruins", "A custom terrain and annotations", new Color(90, 130, 70));
        ruins.setLayers(Arrays.asList(roads));
        ruins.setTerrain(Terrain.CUSTOM_1);
        write(dir, "Mossy ruins", ruins);
    }
}
