package org.pepsoft.worldpainter.layers.plants;

import org.pepsoft.worldpainter.*;
import org.pepsoft.worldpainter.exporting.JavaWorldExporter;

import java.io.*;
import java.util.UUID;
import java.util.zip.GZIPInputStream;

/**
 * Exports a WorldPainter world with WorldPainter itself, headless, to compare
 * Massif's export of the same world against:
 *
 *   ExportWorld <in.world> <folder>
 */
public class ExportWorld {
    public static void main(String[] args) throws Exception {
        Class.forName("org.pepsoft.worldpainter.DefaultPlugin");
        Configuration.setInstance(new Configuration());
        org.pepsoft.worldpainter.plugins.WPPluginManager.initialise(UUID.randomUUID(), WPContext.INSTANCE);
        WorldIO io = new WorldIO();
        try (InputStream in = new BufferedInputStream(new FileInputStream(args[0]))) {
            io.load(in);
        }
        World2 world = io.getWorld();
        for (int i = 0; i < Terrain.CUSTOM_TERRAIN_COUNT; i++) {
            Terrain.setCustomMaterial(i, world.getMixedMaterial(i));
        }
        long start = System.currentTimeMillis();
        JavaWorldExporter exporter = new JavaWorldExporter(world, null);
        File dir = new File(args[1]);
        exporter.export(dir, world.getName(), exporter.selectBackupDir(dir, world.getName()), null);
        System.out.println("exported in " + (System.currentTimeMillis() - start) + " ms");
    }
}
