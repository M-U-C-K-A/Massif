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
