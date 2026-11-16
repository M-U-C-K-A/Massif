# Development

[← Back to the README](../README.md)

Building Massif, its tests, and checking an export in the game itself. The
crates are listed in the [README](../README.md#project-layout).

## Building

Massif needs a Rust toolchain. The exact version is pinned in
`rust-toolchain.toml` and `rustup` installs it automatically.

```bash
cargo run --release -p massif-app
```

On a Mac, `tools/install-macos.sh` builds it and installs **Massif.app** in
Applications (or the folder given), to open like any other app; run it again
after pulling changes to update it.

## Testing

```bash
cargo test --workspace
```

Two environment variables enable the heavier tests:

- `MASSIF_SNAPSHOT_DIR=<folder>` renders the whole window offscreen (with
  egui_kittest and wgpu), the map, the 3D view, the brush panels, the
  dialogs, the terrain graph and its viewport (a cratered moon, a desert
  canyon, snowy peaks under clouds, every template) and the real blocks to PNG files in that folder, and
  drives the editor with simulated mouse strokes, 3D sculpting, area
  splitting, converting a terrain graph to the map and painting a real
  block. The real
  blocks are only tested when a Minecraft installation is found.
- `MASSIF_WORLD_SAMPLES=<file.world>` also imports a real WorldPainter world
  through the editor, renders it whole in the 3D view and measures what it
  costs.
- `MASSIF_RIVER_WORLD=<world.massif>` (with `MASSIF_SNAPSHOT_DIR`) runs the
  rivers on a real map as a user does — the Rivers on this map template,
  a river laid out from the mountains to the sea, another let go at a source — and renders
  them from above and in 3D.
- `MASSIF_GRAPH_BUILD=<samples>` times a whole build of the starter graph
  (run it with `--release`), and with `MASSIF_GRAPH_APPLY=1` its writing to
  a map of as many blocks; `MASSIF_GRAPH_TEMPLATE=<word>` times a template
  instead (`floating`, `karst`…).

## In the game itself

`tools/minecraft_check.py` checks an export in Minecraft: it exports a
world for a version, starts that version's own server on it without a
window (the game installed on the computer: its client jar, libraries and
Java), and has the game confirm, column by column, what
`massif-cli probe` says was written there — the top block, the water over
it, the biome — then reports what matches, what does not and any error the
game logged while reading the chunks. Running a server means accepting
Mojang's EULA: it only starts one with `--accept-eula`.

```bash
tools/minecraft_check.py --world my.massif --version 1.21.11 --minecraft "<game folder>" --accept-eula
```

### Chunks in each version's own format

From 1.18.2 on, every chunk carries the data version of the version
exported for, and holds only blocks and biomes that version has, under its
names (`massif_anvil::version`: `blocks.txt` dates every block from 1.18.2
to 26.3, read from the client jars' `assets/minecraft/blockstates` and the
wiki's version pages; renames come from the game's data fixers). The game
then loads the chunks without upgrading them. To list the blocks or biomes
of an installed version:

```bash
unzip -Z1 "<versions folder>/1.21.11/1.21.11.jar" 'assets/minecraft/blockstates/*' | sed 's#.*/##;s#\.json##'
```

`chunks_are_each_versions_own` (in `crates/massif-anvil/tests/export.rs`)
exports a map of blocks and biomes added or renamed since 1.18.2 for every
release from 1.18.2 to 26.3, reads each chunk back with `fastnbt` and
`fastanvil`, and checks its data version, status, keys, and that each block
and biome is one the version has, as it names it.

`level.dat` and the saved data are the version's own as well
(`massif_anvil::level_dat`): what changed since 1.18.2 was read from the
game's data fixers in 26.3's jar, which turn an older world into the newer
one and so say exactly what each version writes (`LegacyWorldBorderFix`,
`WorldSpawnDataFix`, `WorldBorderWarningTimeFix`, `LevelDatDifficultyFix`,
`DayTimeToClockFix`, `LevelDatToSavedDataFileFix`,
`DimensionStorageFileFix`), and from its codecs. Since 26.1 the jar is not
obfuscated, so the names read plainly from its classes' constant pools;
`javap` from Java 8 cannot read them, a short script over the class files
can. `each_version_gets_its_own_level_dat` checks every release from 1.19 to
26.3 field by field, and `older_versions_keep_their_bytes` that
`level.dat` up to 1.18.2 is still what Massif 0.6 wrote. 26.3 also renamed
the keys of a block state in the chunks' palettes (`id` and `properties`,
`BlockStateFieldNamesFix`), which `Target::state_keys` gives.

To check one in the game by hand:

1. Export a map for the version (a small one, or **Test Export**), with
   cherry or pale oak trees, short grass and a cherry grove in it.
2. Open the world in that version of Minecraft. It should open at once,
   without asking to make a backup (the game asks that of a world it has
   to convert).
3. In the game, F3 shows the biome under the player; the blocks should be
   those the version has.
4. Quit, and look in `logs/latest.log` of the game folder for lines with
   `Recoverable errors when loading section`, `Unknown registry key`,
   `Failed to read` or `Couldn't load chunk`: there should be none.
