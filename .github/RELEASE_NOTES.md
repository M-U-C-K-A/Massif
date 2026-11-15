Massif is a map editor for Minecraft Java worlds in the spirit of WorldPainter, with a node-based terrain generator in the spirit of Gaea, written in Rust.

## New in 0.4.2

**Choosing what an area shows.** The icon before an area's title opens every editor on one page, in columns: the World side's views, tools and paint, then the Terrain side's generator. The line under them says what the one under the pointer is for, and whether another area shows it already. An empty area holds the same page.

## New in 0.4.1

**The whole map at once.** **Edit ▸ Whole map** runs the terrain graph's filters over the whole map: erosion, thermal erosion, rivers (their beds in gravel), lakes, smoothing, sharpening, terraces, strata, and scaling the relief around the sea. Each opens with its settings and the map before and after, runs in the background with its progress, and is one step to undo; the map's paint, its levels and what was sculpted in 3D stay.

**Smoother.** The map glides on when a drag is let go, the zoom glides, and the views no longer stall while the map's images and the 3D tiles are made: on a map of 5,888 blocks, zooming went from 32 ms of work a frame to 6. The 3D view keeps going a little after a turn, flies softly, and draws itself smaller while it moves, sharp again when it stops.

**Faster.** Exporting a map takes about a fifth less work and writes the same files byte for byte; 3D sculpting with a large brush, and rebuilding the 3D view where it was sculpted, take less time; erosion in the terrain generator and in Edit ▸ Whole map takes about a quarter less work and makes the same terrain to the bit.

**Typefaces.** Settings ▸ Typeface sets the interface in the system's own typeface or in Inter, Geist, IBM Plex Sans or Manrope, built in, each shown in itself.

**Developer mode.** Settings ▸ Developer adds a Developer panel to every area's menu: frames a second, the time of each frame, the memory and the processor, in live charts, with what Massif keeps and why it draws again.

**Ten scenes behind the home screen**, and in the light theme each at dawn. The new map's preview stands beside its settings, as tall as they are, and what is written over the picture reads over any part of it; the links under the settings say what they open.

**Accessibility.** Settings ▸ Accessibility: high contrast (black or white text, stronger muted text, lines and edges, links underlined), reduced motion (views stop where they are let go, pictures and cards stay still) and the size of the whole interface, 100 to 150 %. On macOS the first two follow the system's own settings unless set otherwise.

## New in 0.4.0

**Walk your map as a player.** **Player**, in 3D Sculpt, comes down into the map where the view looks, with Minecraft's body, eyes and field of view: a drag or the arrow keys look around, a click lends it the mouse as in the game (Esc gives it back), W A S D walk, Space jumps — twice quickly, flies —, Q sneaks or goes down, Shift runs, and walking into a step of one block jumps onto it. Water slows you and lets you swim; under it the world turns blue and dim. The buttons sculpt at the crosshair.

**3D Sculpt, grown up.** Sculpt, Smooth and Paint share the brush of every tool (custom images too, as set in Brush Settings); a stroke adds a layer at a time and works toward the eye at cliffs and edges. Matter pushed out keeps the blocks it came from — a sand wall gives a sand ledge, the ground under it keeps its own —, Paint repaints sculpted matter, Smooth rounds the land off without wearing it down. Brushes sculpt up to 64 blocks around in 3D.

**Noise painting.** Paint lays several terrains or blocks at once, mixed by a noise, as Axiom's noise painter does: patches, veins, blobs, cells or stripes of simplex, Perlin or value noise, with their size, detail, stretch and direction; the blocks in order, each with its share of the ground; presets, patterns saved under a name, a preview under the brush, and **Paint the whole map** in the background.

**Ground covers and custom objects.** Grass, flowers, ferns, snow or any blocks of your own, painted and grown over the ground; trees, rocks and ruins from `.schem`, `.schematic`, `.bp`, `.litematic` and `.nbt` files, imported or dropped on the window, painted where they stand.

**Automatic biomes and paint that make sense.** Sand is a desert only where sand is all around, beaches are by the water, a WorldPainter map all of one terrain is read from the shape of its land. Over the whole map they run in the background with their progress; with a brush, the land is read once a tile and kept, so brushes of any size stay quick — and no longer crash.

**Settings in four sections**: the theme and the accent colour; sounds made by Massif as they play (off, essential, or all), which leave the speakers and headphones free when silent; the system's notifications when a long task ends; a memory limit, with what is in use.

**A quieter interface.** Neutral greys and one accent, grouped controls, pill buttons, slim sliders, tool icons drawn for what they do to the land, cards with short animations, messages at the bottom right, a help page (F1). The home screen shows night scenes, dithered in a palette of their own.

**The 3D view up close.** Rivers stepping down show their falls, every block reads by its own shade and edges, deep water reflects the sky; in Slope, Height and Biome, a column's walls take its colour.

**A new README**: a banner of the home screen's night scenes, animated figures, and a guide, a page on terrain graphs and one on how Massif works, in `docs/`.

**Files.** The map is a `.massif` file's icon in the Finder and in Windows Explorer; tall WorldPainter worlds keep their height; sculpted matter keeps its blocks in the file (older files still open).

## Earlier

- **0.3.1**: a home screen; maps up to 2032 blocks tall; rivers laid out from their source, finding their own way down; a 3D Sculpt editor; height maps in and out; the node graph handled as in Gaea or Blender; a new look.
- **0.3**: rivers by hand and a Rivers node that traces them in their valleys; a Map node that reads the map you have, with the **Paint this map** and **Rivers on this map** templates; climate (temperature, humidity, biomes) and tectonic plates in the generator; terrains of any size; conversion over the map; patterns of terrains; maps of 60,000 × 50,000 blocks.
- **0.2**: export for every version from 1.7.10 to 26.3; Auto biome and Auto paint; a brush option for the way slopes face; floating islands and overhangs from the generator.
- **0.1**: the map editor — levels of rock and pits, 3D sculpting, WorldPainter's brushes and controls, the game's real blocks, the whole map in 3D, `.world` import — and the node-based terrain generator.

## Downloads

| System | File |
|---|---|
| Windows 10/11 (64-bit) | `Massif-…-windows-x86_64.zip`: unzip, run `Massif.exe` |
| macOS 11+ (Apple silicon and Intel) | `Massif-…-macos.dmg` (or `.zip`): drag Massif to Applications |
| Linux (x86-64) | `Massif-…-linux-x86_64.tar.gz`: extract, run `massif` |

The command-line tool (`massif-cli`: new, info, verify, import, export) comes with the Windows and Linux builds, and separately for macOS.

These builds are not signed by Apple or Microsoft:

- **macOS**: the first time, right-click Massif → Open, then Open again (or run `xattr -dr com.apple.quarantine /Applications/Massif.app`).
- **Windows**: if SmartScreen stops it, choose More info → Run anyway.

The real Minecraft blocks are read from the game installed on the computer (official launcher, CurseForge, Prism); nothing from the game is included.
