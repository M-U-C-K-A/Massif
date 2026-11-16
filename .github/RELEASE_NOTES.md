Massif is a map editor for Minecraft Java worlds in the spirit of WorldPainter, with a node-based terrain generator in the spirit of Gaea, written in Rust.

## New in 0.6.0

**Autosave: a crash no longer loses your work.** While the map has changes not saved, Massif now keeps a copy of it every few minutes, in a folder of its own (never over your file). Should Massif close without saving them, from a crash, a forced quit or a power cut, it offers the next time it starts to restore the copy: the map's name and when the copy was taken, then **Restore** or **Discard**. Saving the map, or closing it, removes its copy, and at most five are kept. The copy is written in the background and the map stays smooth while it is, even a map of 44,544 × 33,536 blocks. **Settings ▸ Autosave** turns it off or sets how often: every 1, 2, 5 (the default), 10 or 15 minutes.

## New in 0.5.2

**Massif starts on slow computers.** On some Windows computers (an AMD Radeon RX 6400, for one) Massif 0.5.1 never showed its window, whatever graphics API it tried: it waited, before drawing its first picture, for the WorldPainter brushes and the game's textures to finish loading, which can take minutes on a slow computer with many brushes. It then blamed the graphics driver and set Vulkan, DirectX 12 and OpenGL aside one after the other. The home screen now comes at once: its picture, the new map's preview and the dates of the recent maps follow a moment later, and the loads no longer hold up the window. Massif sets a graphics API aside only when the graphics driver itself is stuck, and says so; the APIs 0.5.1 set aside by mistake are tried again on the next start. `startup.log` names the part of the window being laid out, should a start still be slow.

**Packs: trees, plants and mixes ready to paint.** As in WorldPainter Modernised, a **Packs** tab beside the Layers panel's list holds the sixteen kinds of trees of the tree pack (tall and slender spruces, firs, larches, giant redwoods, big and pale oaks, birches, cherries, acacias, willows, rainforest giants, palms, dead trees, bushes and azaleas, eight of each), twenty-two presets of flowers, plants and water plants, and thirty-nine mixes of blocks for grass, soil, stone, sand, snow and volcanic rock, with cliff strata laid in horizontal beds. A click adds one to the map and picks it to paint; a search finds one by name. Exports for older versions of the game turn the newer woods into the closest older ones, their leaves still persistent.

**A Layers panel.** WorldPainter's custom layers have been in Massif as ground covers (plants, flowers, any blocks), object layers (trees and rocks from schematics) and terrain mixes (noise painting), but their lists were tucked inside the settings of their tools. The new **Layers** panel, under Terrain, lists them all: click one to paint with it, double-click to rename it, click its eye to hide it on the map and in the 3D view (the export still writes it), right-click to duplicate or remove it, and add new ones from its sections. Layouts saved by earlier versions get the panel under their Terrain panel.

**The menus only in the Mac's menu bar.** On a Mac, File, Edit, View and Help are now only in the menu bar at the top of the screen, where 0.5.1 put them: the window's top bar no longer shows them a second time, and keeps its buttons (new, open, save, export, undo, redo and the tools). On Windows and Linux the menus stay in the window.

**Quitting from the Dock asks first.** On a Mac, **Quit** in Massif's menu in the Dock, Q in the app switcher (⌘Tab), and a logout, a restart or a shut-down no longer drop the changes to the map that are not saved: Massif comes to the front and asks first, as **Quit Massif** (⌘Q) does. A logout, a restart or a shut-down waits for the answer: Save or Don't Save, and it goes on once Massif has quit; Cancel, and it stops there. With nothing to save, Massif quits at once, as before.

**A faster export.** Exporting a map to a Minecraft world takes about a quarter less work: a map of 2,116 tiles that took 35 seconds of processor time now takes 25, and a little over 3 seconds instead of nearly 5 on a ten-core Mac. The world written is the same, byte for byte.

**Faster WorldPainter imports and map tiles.** Importing a `.world` takes less than half as long as before (a map of 5,888 blocks a side: 0.33 s instead of 0.88 s), and unpacking the map's tiles, which painting, drawing the map, the picture saved in the file and the export all do, a quarter less. Imported maps and saved files are exactly the same as before.

**A huge map's 3D view built twice as fast again.** On a very large map (tens of thousands of tiles), the 3D view takes half the work it took in 0.5.1 to build, and less than 0.5.0 did: a map of 44,544 × 33,536 blocks went from about 340 to 180 seconds of processor time, under a minute on a ten-core Mac, so the view gets sharp sooner and the rest of the app stays smooth while it builds. The 3D views no longer make their own look pay for the player's Minecraft look either, which weighed on some graphics cards (AMD among them) even when the player's view was off.

**Quicker brushes on large strokes.** Smooth takes about 30 % less work a dab with a large brush, and brushes limited by slope, by hollows and bumps, or by height take 55 %, 20 % and 10 % less; Raise, Lower and the paint brushes a little less too. What they write is exactly the same as before.

**Flatten one way or both, as in WorldPainter.** Flatten's settings now choose which way it moves the land: **Both ways** as before, **Only raise** to fill hollows up to the height and leave the hills, or **Only lower** to cut the hills down and leave the hollows. **Apply theme** repaints what it flattens with the terrain its new height calls for, sand by the sea, grass, rock and snow higher up.

## New in 0.5.1

**The player's view looks like Minecraft with Axiom.** With **Player** on in 3D Sculpt, the view shows the game's own blocks with their textures (read from the Minecraft installed on the computer), lit as the game lights them, corners shaded as its smooth lighting does, under its blue sky with the square sun, its clouds and its fog, deep water darker toward the floor. The blocks the brush takes are marked in Axiom's lavender and the block under the cursor is outlined. The cursor free, it works as Axiom's editor: left drag looks around, the right button uses the tool under the cursor (Alt: its inverse), the middle one picks the block for Paint, Ctrl turns about a point or moves the view, Ctrl + middle sizes the brush, and the wheel sets the flight speed, also shown in the bar along the bottom with what each button and key does. A click takes the mouse as in the game: the right button builds, the left one breaks. The keys are the game's: Shift sneaks and goes down, Ctrl sprints.

**Height limits you can read.** Brush Settings draws the map from the side, cut from west to east through its highest point, with the sea in blue and the heights on a scale beside it. The land at the heights the brush acts on is in the accent, only between the limits' lines, and each limit is a line across it with its height, dragged up or down (Shift: finely). The picture keeps to the heights of the map, so a limit beyond them waits at its edge instead of squeezing the land into a corner, and the picture and its settings make one group.

**File, Edit, View and Help in the Mac's menu bar.** On a Mac, Massif's menus are now in the menu bar at the top of the screen too, as in every Mac app: the same items as the window's menus, with their shortcuts, greyed out when they cannot be used, and kept in step with them. **About Massif**, **Settings…** (⌘,) and **Quit Massif** (⌘Q) are in the **Massif** menu, where Mac apps keep them, and Help has the system's search of the menus. ⌘Q now asks first when the map has changes that are not saved, as File ▸ Quit does, instead of quitting at once. The keys work as before, in the map and in text fields alike; the window's menus show the shortcuts of their items too.

**Fixed.** The terrain generator's viewport no longer turns the land black: looked at from high above, the mist over it came out as 0 × ∞, which the graphics card paints black. A number typed in (the size of a new map, a height, an ore band) counts once it is done, with Enter or by leaving it: typing 1024 no longer stopped at 1, jumped to the least value allowed, and grew from there into a map of tens of thousands of blocks. The menus of the top bar (File, Edit, View, Help) take a click over a hit as roomy as a system menu bar's, not just over their word.

**Ores: taken out with a key, kept to biomes.** In Edit ▸ Ores, Delete (Backspace on a Mac) takes the ore picked out of the map's ores, as the new **Remove** does, and ⌘Z (Ctrl+Z) puts it back; the **+** at the end of the ores adds back one taken out, as it was. The details under the cut are the picked ore's, its colour down their side: the band picked is ringed in the accent as its candle is, the band whose candle the pointer is over is lit, and pointing at a band there lights its candle. An ore can now be laid **only in** some biomes, or in every biome **except** some, ticked on and off in a menu with a search, as Brush Settings picks what a brush acts on: the export, the cut and the amounts follow, and a funnel marks the ore in the strip. Maps saved before keep their ores, and an export without a biome choice is the same as before.

**No more rows missing after a conversion.** Convert to voxels writes the terrain into the map it was started on. One that ended after another map was opened (a new one, say) used to put its tiles into that map, and the two never matched: black rows of missing land were left between them. It now stops and says so, and one started in another dimension waits until that dimension is shown again. Auto paint, Auto biome, Noise paint and the graph's filters over the whole map behave the same way. Saving no longer hands its file to a map opened before the save ended, so the next save of that map can't overwrite the first one.

**The 3D views with OpenGL.** On a graphics card that only OpenGL drives (an Intel HD Graphics 4600, for one), opening a 3D view, a template or a height map stopped Massif with "Downlevel flags VIEW_FORMATS are required". The 3D views, 3D Sculpt and the terrain generator's viewport now draw there, the same images as elsewhere, and saving the viewport's image no longer gives up while a slow card draws it.

**A start that gets past a stuck graphics driver.** On some Windows computers (an AMD Radeon card with Vulkan, for one) Massif stayed in the background with no window: the driver never finished its first picture. The window is now shown before that first picture, which some drivers need. If a start still stays stuck, Massif notices after 12 seconds, offers to start again right away with DirectX 12 (then OpenGL), and keeps to it on that computer; a start that died without a word is noticed by the next one. `startup.log` follows the first pictures step by step, writes down a crash with the file it came from, and says which graphics API was asked for; `WGPU_BACKEND` is read in quotes too. See *If Massif does not start* in the guide.

## New in 0.5.0

**Ores, as Minecraft lays them.** The export now lays ores in the rock — Minecraft adds none to the map's finished chunks — following the game's own veins and heights since 1.18, or those of older versions. **Edit ▸ Ores** shows the map cut through with the ores exactly where they will be and a candle for each band of each ore: drag its ends, its body or its side to move the heights where veins start, where they are the most common and how large they are; beside it, the ore expected at each height in blocks, on a log scale or as a share of the ground. Saved with the map; the export can leave them out.

**Survival check.** **Edit ▸ Analyse the map** counts every block of the map as the export writes it and reads it as a survival player would: in red what breaks survival (diamond blocks or ores lying around, bedrock above the floor, holes into the void, no wood), in amber what is missing (ores, with a way to Edit ▸ Ores; lava for a Nether portal, grass for animals, trees near the spawn), the way from wood to the Nether, what holds up, and how to fix it. Tabs for the relief (heights, steps a player walks, climbs or falls from, water), the blocks and ores, and the biomes; a spot shown on the map in a click; the report copied as Markdown. A few seconds for a map of 5,888 blocks a side, the ores counted where the export lays them.

**A truer terrain viewport.** Cliffs are rock — lumps, grain and beds drawn in the world's own space — instead of faceted, stretched stripes; each point sees as much of the sky as the relief around it leaves it; haze and a low mist give depth at any map size; forests show their crowns and grass its drier patches. While the view moves it is drawn at half the size, so it moves as smoothly as before. The karst towers (and the floating mountains' towers) have sheer fluted walls, rounded shoulders and broad green tops, where they used to end in a spire.

**Choosing what an area shows.** The icon before an area's title opens every editor on one page, in columns: the World side's views, tools and paint, then the Terrain side's generator. The line under them says what the one under the pointer is for, and whether another area shows it already. An empty area holds the same page.

**Fixed.** The height limits of the brush options are in whole blocks, as the game has the land: a column at 63.2 is at 63 (maps from WorldPainter keep fractions of blocks), and the height tools stop on a sharp limit — flattening toward 63.8 the land at or below 63 brings it to 63, no longer 63.1. A turned square or image brush acts on all of its shape, its corners too. The strength drawn under the brush is lighter, keeps to the outline block by block close up, no longer draws a diamond inside square brushes, and can be switched off (Brush Settings ▸ Strength on the map). The line under the map gives the height in whole blocks and the biome under the pointer.

**Height limits, redrawn.** Brush Settings shows the heights of the map itself, as bars from its lowest to its highest land with the sea marked, under the band the brush acts in: drag its ends, or set them in the fields below, between or outside them; it says how much of the map the band covers.

**Split from any corner.** Every corner of an area splits it when dragged into it, not only the top right one: the new area opens on that corner's side, and while dragging, the line of the split and the side it opens on show.

**A start that says what goes wrong.** Massif writes down every step of its start in `startup.log` (in `%LOCALAPPDATA%\Massif` on Windows, `~/Library/Logs/Massif` on macOS), with any error, the one before kept beside it, and nothing sent anywhere. When it cannot start or stops on an error, a message says so instead of the process staying in the background with no window; on Windows the window is shown after 3 seconds even when the graphics card is slow to draw its first picture. See *If Massif does not start* in the guide.

**A big map stays smooth.** The 3D view builds a whole map away from the frames, on threads of its own, and puts what it built on the graphics card a little at a time: on a map of 44,544 × 33,536 blocks, a frame went from 29 ms of work to 11 ms while it builds.

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
