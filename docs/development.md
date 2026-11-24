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
- `MASSIF_CONQUEST_DIR=<instance>` reads Conquest Reforged from its own
  instance (or the best of a folder of instances, such as CurseForge's
  `Instances`): every block with its textures, its states turned and
  mirrored against its list for WorldPainter, an export for 1.20.1 read
  back and checked against every state the game registers, and the app
  from end to end (`Use` in the Blocks panel, a map painted, the export's
  choices). `MASSIF_CONQUEST_WORLD=<folder>` keeps that map and its world;
  `MASSIF_CONQUEST_CHECK=<world>` checks another exported world the same
  way; `MASSIF_CONQUEST_JAR=<jar>` renders the gradient brush with its
  blocks.
- `MASSIF_INSTANCES_DIR=<folder>`: in tests, the instances the Blocks panel
  offers are only those of this folder (else of `MASSIF_CONQUEST_DIR`), never
  what the computer running the tests has installed.
- Mod profiles (`crates/massif-core/assets/mods/<namespace>/`, see its
  README) are data: a mod's name, the version of the game it is made for and
  an optional palette. `build.rs` bundles every folder found; no mod is
  named in the code outside tests.
- `MASSIF_MOD_JARS=<jar;jar…>` reads mods' jars over the version of the game
  each is made for: every block file gives a block, nearly all with their
  textures, and it prints how many show at first and each family's count.
- `MASSIF_VANILLA_WORLDS=<folder>` compares Massif's level.dat with those
  the game itself saved there (a saves folder, an instance, a folder of
  instances, or `auto` for the official launcher's and every instance).
  These read the player's own game, so they run only when asked.
- `MASSIF_GRAPH_BUILD=<samples>` times a whole build of the starter graph
  (run it with `--release`), and with `MASSIF_GRAPH_APPLY=1` its writing to
  a map of as many blocks; `MASSIF_GRAPH_TEMPLATE=<word>` times a template
  instead (`floating`, `karst`…).

## Performance

The performance recorder (`crates/massif-app/src/perf.rs`) takes the scopes
of the `profiling` crate: egui's, eframe's and wgpu's own, and Massif's
(`profiling::scope!("map view")`; `massif-core` has some too). Massif turns
on its `profile-with-tracing` backend and installs a small `tracing`
subscriber at start that answers "not now" to every scope until a recording
is on (an atomic load a scope; `scopes_cost_nothing_while_off` checks it
stays under 200 ns). A new scope of Massif's shows in recordings by itself;
give it a short name, and one of `Recording::COLUMNS` if it should have a
column of `frames.csv`.

Two tests drive the whole app offscreen with the recorder on (run them with
`--release -- --nocapture`):

- `MASSIF_PROFILE=1 cargo test --release -p massif-app profile_a_session`: a
  map (`MASSIF_MOVE_WORLD=<.massif>`, else one of `MASSIF_PROFILE_TILES`²
  tiles, 32 by default: 4,096 blocks a side) in the Layout workspace with a
  Diorama and a floating 3D View; then still, panned, zoomed, a big brush
  (`MASSIF_PROFILE_RADIUS`, 100 blocks) raising and then painting, a
  4,096 × 3,072 reference image added and dragged. Each stage prints its
  frames, its processor time and where the interface's time went;
  `MASSIF_PROFILE_STAGES=raise,paint` records only those,
  `MASSIF_PROFILE_PLAIN=1` keeps the workspace as it is, and
  `MASSIF_PROFILE_OUT=<folder>` keeps each stage's recording as a zip.
- `MASSIF_REPLAY=<recording .zip or folder> MASSIF_REPLAY_MAP=<.massif>
  cargo test --release -p massif-app replay_a_recording`: a user's
  recording played back on a map, in its workspaces, window size and pixels
  a point, frame by frame, with its input and tool in hand; the frames are
  timed as the recorder times them. Floating panels' own windows are drawn
  inside the window there, and only the main window's input is replayed.

The machine is shared: compare processor time, best of several runs, and
run the binaries before and after a change one after the other.

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
