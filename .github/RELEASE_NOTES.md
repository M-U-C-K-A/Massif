Massif is a map editor for Minecraft Java worlds in the spirit of WorldPainter, with a node-based terrain generator in the spirit of Gaea, written in Rust.

## In this version

- **World**, the map editor: a multi-level surface (ground, solid and carve levels), 3D sculpting, WorldPainter's brushes and controls, the game's real blocks, the whole map in 3D at block resolution, WorldPainter `.world` import, Minecraft export for versions 1.7.10 to 26.3.
- **Terrain**, the generator: about sixty nodes (landforms, tectonic plates, primitives, adjustments, erosion, rivers and other simulations, masks, climate, colours); rock over the void (floating islands, overhanging cliffs) that becomes levels of the map; ten landscapes to start from (alpine range, archipelago, karst towers, floating mountains, river valley, desert canyon, volcanic island, dune sea, rolling hills, continents) and two templates that work on the map you have; a viewport with a sun you turn, a sky that follows it, volumetric clouds, haze and a sea; images saved up to 8K; builds up to 16,384 × 16,384, converted to voxels as real blocks.

- **New in 0.2**: export for the version you play, from 1.7.10 to 26.3; Auto biome and Auto paint, by rules you can change; a brush option for the way slopes face (the sunny side, the shade); a brush preview that shows where it presses hardest; rock over the void in the generator (floating islands, overhangs) that becomes levels of the map; stone towers and floating mountains among the landscapes to start from.

- **New in 0.3**: rivers drawn by hand (River tool: a course of clicks, water that only runs down, banks easing into the land, branches and tributaries, up to 400 blocks wide); a Rivers node rebuilt from the ground up — rivers traced in the bottoms of their valleys from the sea to their sources, with tributaries, growing downstream, dug at every block with banks, water that only runs down and lakes in the hollows they cross; a Map node that reads the map in 3D (its levels of rock, its pits, what was sculpted) to work on an existing map, with two templates for it: **Paint this map** and **Rivers on this map**; climate in the generator — Temperature, Humidity (wind, rain shadow), Biomes, and a Biome output that writes the game's biomes and paints the ground to suit; Tectonic plates you move by hand, and a **Continents** template; a terrain Size in blocks (the map, or 1k to 32k: larger than the map, converting grows the world), always converted at full detail; conversion over the map that leaves it as it is; patterns of terrains to paint with (patches, veins, blobs, cells, stripes); maps of 60,000 × 50,000 blocks, shown whole in 3D.

- **New in 0.3.1**: the node graph handled as in Gaea or Blender — several nodes selected (Shift, ⌘, or a box), moved together, duplicated, copied and pasted with their wires, deleted at once, wires cut with a right-click; a mouse wheel zooms, right- or middle-drag or Space moves around; ⌘Z undoes changes to the graph and conversions; a line of hints along the bottom. Rivers on real maps: the water already on the map (rivers and lakes from WorldPainter) is kept and joined, rivers over low coastal plains keep their water, no more stray lakes in every hollow. The River tool draws by dragging, and **River source** makes a river in one click: its water finds its own way down, eroding a bed, filling the hollows it meets into lakes, with its flow, width, depth, erosion and lakes to set — the map shows where it would go before the click. Height maps in and out: a new world from a 16-bit PNG (Gaea, World Machine…), the map's relief exported as one, a Height map node in graphs and a graph's terrain saved as a height map. A Welcome screen with the ways in.

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
