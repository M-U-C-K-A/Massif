Massif is a map editor for Minecraft Java worlds in the spirit of WorldPainter, with a node-based terrain generator in the spirit of Gaea, written in Rust.

## New in 0.3.1

**A home screen.** Massif opens on it, not on a map made up for it: on the left, a new map, a map from a height map, a map of your own to open, a WorldPainter world to import, and the maps opened lately; in the middle, the settings of a new map with its preview, laid on ridges drawn in dots that twinkle now and then. A file dropped on the window opens. **File ▸ New** (⌘N) comes back to it.

**Maps taller than the game.** A new map can rise to 512, 1024 or 2032 — the highest Minecraft allows — and go as deep as −2032. The export adds a data pack that gives the world that height.

**Rivers laid out, not drawn.** Click the source: the water finds its own way down, eroding a bed and filling the hollows it meets into lakes (the map shows where before the click). Click elsewhere to give it an end, reached along the valleys; drag the river to bend it; **Dig river** or Enter digs it. On real WorldPainter maps, the rivers and lakes already there are kept and joined.

**3D Sculpt.** Sculpting in 3D has its own editor and workspace: one **Sculpt** tool (the left button adds matter, the right one digs) and **Smooth**, any brush — custom images too — laid on the slope under the cursor without shaking. Shift + drag moves the view, Alt + wheel sizes the brush. The 3D View only shows the world.

**Height maps in and out.** A new map from a 16-bit PNG (Gaea, World Machine…), the map's relief exported as one, a **File input** node that brings a height map, a `.world` or a `.massif` into a graph, and a graph's terrain saved as a height map.

**The node graph handled as in Gaea or Blender.** Several nodes selected (Shift, ⌘ or a box), moved, duplicated, copied and pasted with their wires, deleted at once; wires cut with a right-click; the wheel zooms, right- or middle-drag moves; ⌘Z undoes changes to the graph and conversions.

**A new look.** Panels close together with a header strip each, number fields filled up to their value (drag them, or click to type), controls grouped as one, an accent colour to choose in **Settings** (⌘,), the tools grouped by theme, one-line explanations. In 3D, a daylight sky and deep water that reflects it; lava worlds with opaque, glowing lava and scorched shores.

**Auto paint and Auto biome** show what they would change under the brush before painting, cover the whole core of the brush and say what they did.

On a Mac, `tools/install-macos.sh` builds Massif from the sources and installs it in Applications.

## Earlier

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
