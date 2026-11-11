Massif is a map editor for Minecraft Java worlds in the spirit of WorldPainter, with a node-based terrain generator in the spirit of Gaea, written in Rust.

## In this version

- **World**, the map editor: a multi-level surface (ground, solid and carve levels), 3D sculpting, WorldPainter's brushes and controls, the game's real blocks, the whole map in 3D at block resolution, WorldPainter `.world` import, Minecraft export.
- **Terrain**, the generator: more than fifty nodes (landforms, primitives, adjustments, erosion and other simulations, masks, colours); eight landscapes to start from (alpine range, archipelago, karst towers, river valley, desert canyon, volcanic island, dune sea, rolling hills); a viewport with a sun you turn, a sky that follows it, volumetric clouds, haze and a sea; images saved up to 8K; builds up to 16,384 × 16,384, converted to voxels as real blocks.

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
