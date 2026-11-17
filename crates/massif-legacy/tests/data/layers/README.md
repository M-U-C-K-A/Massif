# WorldPainter layer files

Custom layers as WorldPainter's Export writes them (`.layer`: the layer
Java-serialised, gzipped), read by `massif_legacy::layer_file`.

- `River.layer` comes from WorldPainter's own test resources
  (`WPGUI/src/test/resources`), GPL-3.0 like Massif: a ground cover of
  five blocks that takes the place of the top of the land.
- The others were written by `tools/wp-layers/MakeLayers.java` with
  WorldPainter 2.27.2 (modernised fork) classes: plants (Meadow), a ground
  cover (Snow cover), underground pockets (Gravel pockets), a tunnel
  (Mine), a custom annotation layer (Roads to build), and two combined
  layers, one holding the others with frost (Snowy meadow), one with a
  custom terrain of its own (Mossy ruins).

Layers of third-party objects (Bo2 trees, schematics, structures) are not
kept here: point `MASSIF_WP_LAYERS` at a folder of `.layer` files to read
them too.
