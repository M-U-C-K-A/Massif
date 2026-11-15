# Getting started
Massif makes Minecraft maps. Shape the land, paint it, decorate it, then write it to a Minecraft world.

## The home screen
The home screen opens with Massif. On the left are the ways in: **New map**, **From a height map**, **Open a map…** (a `.massif` file), **Import from WorldPainter…** (a `.world` file), and the maps opened lately. In the middle are the settings of a new map, with a preview of the whole map.
- **Size**: width × depth in blocks, in steps of 128.
- **Height**: Minecraft's own (−64 to 320) or taller, up to 2032 blocks.
- **Land**: a landform (flat, plains, hills, mountains, islands) or a template of the terrain generator, and how the ground is painted.
- **Water**: its level, water or lava, and beaches along the shores.
- **Seed**: the same seed always gives the same land.
Press **Create map** (or `Enter`). A `.massif`, a `.world` or a height map dropped on the window opens too.

The picture behind the home screen changes each time Massif opens, the ones not shown yet first; the button at the bottom right shows another.

## Saving and exporting
- `⌘S` saves the map as a `.massif` file. The Finder and Explorer show the map as the file's picture.
- `⌘E` exports to Minecraft: the saves folder, the world's name, the version of the game, its rules, its border and data packs.
- **Test Export…** writes only the 3 × 3 tiles around the middle of the map view to a separate world, to check the result in the game quickly.

## Survival check
**Edit ▸ Analyse the map…** checks the whole map for a survival game, every block counted as the export writes it: the layers of the terrains, the rock and the bedrock under them, the water, what the covers grow and the objects. In red, what breaks survival: riches lying around (diamond blocks, ores at the surface, far more ore than Minecraft makes), bedrock above the floor, blocks no player can break, holes into the void, no wood. In amber, what is missing: ores (turned off, or no rock at their heights: **Set up the ores** opens Edit ▸ Ores), lava for a Nether portal, water, grass for animals, trees near the spawn. Then the way through a game, from wood to the Nether, what holds up, and **How to fix it**.
- **Relief**: the heights players stand at, the steps between columns (walked, climbed, falls that hurt, falls that kill), the water, overhangs and caves.
- **Blocks**: the ores, the blocks at the surface and in the ground, plants, objects. **Biomes**: the biomes as the export gives them.
- **Show on map** glides the map to the spot, the nearest to the spawn. **Copy report** copies it all as Markdown; the **…** menu saves it.
The check runs in the background — a few seconds for a map of 5,888 blocks a side — and comes back as it was while the map is the same.

## Ores
**Edit ▸ Ores…** sets the ores the export lays in the rock — Minecraft adds none to the map's finished chunks —, as the game lays them since 1.18 to begin with.
- Across the top, each ore with what the map will hold of it: click one to pick it (a funnel marks one kept to some biomes); the **+** at the end adds back an ore taken out. The map, small, at the left: click or drag on it to choose where to cut the map.
- On the left, the map cut through there, the ores exactly where the export will put them, and a **candle** for each band of an ore: the wick from the lowest height a vein starts at to the highest, the body where veins are the most common, its width the size of a vein. Drag a candle's ends or its body's edges to move those heights, its body to move the whole band, its side to make veins larger; the wheel over a body sets how many veins a chunk gets. Drag elsewhere to look along the cut.
- On the right, the ore expected at each height of the map: **Blocks**, **Log**, or **Share** of the ground there.
- Below, the picked ore's details: its biomes, its bands in numbers, **Add a band**, **Laid** to turn that ore on or off, and **Remove**. The band picked is ringed in the accent, as its candle is; the band whose candle is pointed at is lit, and pointing at a band lights its candle.
- **Delete** (Backspace on a Mac) takes the picked ore out; **⌘Z** (Ctrl+Z) puts back what was taken out, an ore or a band. Not while a number is typed.
- **Biomes**: **Only in** lays the ore only in the biomes ticked (none ticked: nowhere), **Except in** never in those ticked; **Add** opens the biomes, with a search, and a chip's × takes one off. The export, the cut and the amounts follow; the other ores stay where they were.
**Minecraft 1.18+** and **Before 1.18** start from the game's own; **Richness** multiplies every band; **Lay ores** turns them all off. Ores only take the place of stone, granite, diorite and andesite (deepslate and tuff get the deepslate ores), never of a block in the open. The settings are saved with the map; the World tab of the export can leave them out of one export.

# The window
## Menus
**File**, **Edit**, **View** and **Help** hold every command, each item with its shortcut beside it, greyed out when it cannot be used (what acts on the map while the home screen is shown, everything while a dialog is open).
- On a Mac they are in the menu bar at the top of the screen, as in every Mac app; the window's top bar keeps its buttons. **About Massif**, **Settings…** (`⌘,`) and **Quit Massif** (`⌘Q`) are in the **Massif** menu, and **Help** has the system's search of the menus.
- On Windows and Linux they are at the top left of the window.
- `⌘Q` asks first when the map has changes that are not saved, as **File ▸ Quit** does.

## Areas
The window is made of areas, as in Blender. Each area shows one editor: the map, the 3D view, tools, settings, brushes, palettes, levels or information.
- The icon before an area's title opens every editor on one page, in columns: the World side's views, tools and paint, then the Terrain side's generator. The line under them says what the one under the pointer is for, and whether another area shows it already. An empty area holds the same page.
- The **…** menu of an area splits it, joins it with its neighbour, maximises it or closes it.
- Drag any corner of an area into it to split it where you let go: the new area opens on that corner's side (across: left or right; up or down: above or below).
- Drag an area by its title onto another to move it there, onto its centre to swap them. Drag the gaps between areas to resize them.
- `Ctrl Space` maximises the area under the pointer, and gives it back its place.

## Workspaces
The tabs at the top are workspaces: arrangements of areas for a kind of work. The two sides of Massif are switched at the top left: **World**, the map editor, and **Terrain**, the terrain generator.
- `+` adds a copy of the workspace shown.
- Right-click a tab: **Reset** puts it back as it was at first, **Duplicate** copies it, **Delete** removes it.

## Moving around
The map moves under the hand: let go of a drag while moving and it glides on, slowing down; the zoom buttons, the zoom keys and **Fit the whole map** glide there in a moment. While a view moves, the others put off the work they do in the background, so that it keeps up, and pick it up as soon as it stops. **Settings ▸ Developer** shows the frames a second and the time of each, live.

## Messages
What Massif did, and what it does in the background (saving, exporting, automatic tools over the whole map), shows at the bottom right of the window. Long tasks have a **Cancel** button.

# Sculpting
## The tools
- **Raise / Lower**: left button raises, right button lowers.
- **Smooth**: rounds the land off.
- **Flatten**: levels the land to the height where the stroke starts.
- **Set height**: sets the land to a height. `Alt`-click on the map picks it.
- **Roughen**: adds or removes roughness.

## Brushes
A brush has a shape, a size, an intensity and a rotation. The shapes are the built-in brushes, terrain stamps (whole mountains, dunes, craters) and your own images.
- Scroll: size (`Shift`: one block at a time). `−` and `+` too.
- `1` … `9`, `0`: intensity 10 % … 100 %.
- `[` `]` or `⌘` + scroll: turn the brush.

## Where the brush acts
The options under the brush settings limit where a brush acts, as in WorldPainter:
- **Height limits**: only at or above a height, at or below one, between two or beyond them. The range is the map's own: from its bottom (−64 for a Minecraft map, or the bottom its file says) to its top. The picture shows the map from the side, cut from west to east through its highest point, with the sea in blue; the land at the heights the brush acts on, between the limits' lines, is in the accent. Drag a limit's line up or down (Shift: finely), or type the heights; the line under the settings says how much of the map the brush acts on.
- **What is on the map**: only on, or except on, some terrains, biomes, water or dry land.
- **Slope** and **Facing**: only on slopes steeper or gentler than an angle, or facing one way.

# Painting
## Terrains and biomes
**Paint** gives the ground a terrain: pick it in the Terrain palette. **Biome** paints biomes the same way: pick one in the Biomes palette. The right button paints back what the height calls for (terrains) or the automatic biome (biomes).

## Automatic biomes and paint
- **Auto biome** reads the land and gives it biomes: water, rivers and beaches, lowland, hills, mountains, peaks, cliffs, from its water, height, slope and ground. Sand at the water's edge is a beach; sand far from the water is a desert only where sand is all around, not a patch in a plain.
- **Auto paint** paints the ground as its biome calls for: a terrain for its ground, one for its cliffs, one under its water.
A map filled with one terrain other than grass (a WorldPainter map all of sand, for instance) is read from the shape of its land: its fill says nothing about it.

Both work with the brush, or over the whole map at once: **Whole map** in the tool settings, or **Edit ▸ Auto biome: whole map** and **Auto paint: whole map**. Over the whole map they run in the background, with their progress, and are one step to undo. **Rules…** sets the heights and slopes the land is read with, the biome of each kind of land and the terrains of each biome.

Hovered over the map, the brush shows in colour what would change.

## The whole map at once
**Edit ▸ Whole map** runs one of the terrain graph's filters over the whole map: **Erosion** (rain carries soil from the heights to the valleys), **Thermal erosion** (loose rock slides where a slope is too steep to hold it), **Rivers** (traced where the water of the map runs, dug into the valleys, their beds in gravel), **Lakes** (water fills the hollows up to where they spill), **Smooth**, **Sharpen**, **Terraces**, **Strata**, and **Scale and raise** (the relief scaled around the sea level, then moved up or down). Each opens with its settings — those of its node in the graph — and the map small, before and after, made again as they change. **Apply to the whole map** runs it in the background, with its progress and a **Cancel**; it is one step to undo. The ground's paint, its levels and what was sculpted in 3D stay as they are; maps over 4096 blocks across are worked out a sample every few blocks, the change spread over the blocks between, so they keep their detail.

## Noise painting
**Noise** (in the Paint tool's settings) paints with several terrains or blocks at once, mixed by a noise, as Axiom's noise painter does: patches, veins, blobs, cells or stripes, of simplex, Perlin or value noise, with its size, detail (octaves, lacunarity, gain), stretch and direction. The blocks are a list in order — neighbours meet on the ground — each with its share of the ground; drag one to move it, click its name to change it, **Add a block** to add one. Presets are a start, and a pattern can be saved under a name. Hovered over the map, the brush shows what it would paint; **Paint the whole map** lays it everywhere the brush options let it, in the background.

# Decor
## Ground covers
**Ground cover** paints what grows over the ground: grass, tall grass, flowers, ferns, mushrooms, snow, seagrass, or any blocks of your own, with their density and how high they grow.

## Custom objects
**Objects** places trees, rocks or ruins from `.schem`, `.schematic`, `.bp`, `.litematic` and `.nbt` files: import them, or drop them on the window, then paint where they stand, how far apart and how many.

# Water
- **River** lays a river from a click to another: its bed, its banks and its water.
- **Flood** fills a basin with water from where you click up to a level; **Drain** empties it.
- **Water brush** paints water at a level.

# Levels
Levels lay rock over the ground or dig into it, each in its own layer that can be shown, hidden, reordered and deleted.
- **Solid** adds rock: overhangs, arches, floating islands.
- **Carve** removes it: caves, tunnels, pits.
**Stamp slab** adds to the active level; the right button, or **Erase slab**, takes away.

# 3D
## The 3D view
- Left drag turns the view, right drag moves it.
- Scroll comes closer and goes back; `Shift` + scroll moves the view, `Alt` + scroll flies toward the point under the cursor.
- `W` `A` `S` `D` or the arrows fly over the map, `Q` and `E` go down and up (`Shift`: faster).
- Double-click looks at the point under the cursor.
- A turn or a move let go while moving goes on a little and dies away; flying with the keys starts and stops softly; a double-click glides to the point.
- While the view moves it is drawn at a lower resolution, sharp again the moment it stops.

## 3D Sculpt
**Build**, **Dig**, **Smooth** and **Paint** work on the blocks in 3D, with the brush of every tool: its shape, size, intensity and rotation are set in **Brush Settings**, shown beside the 3D Sculpt view (`⌘` + scroll changes its size there too). Left button sculpts where the cursor points, right button inverts; held down, it keeps adding or digging. In 3D a brush sculpts 64 blocks around at most (a larger one works as 64). Matter pushed out keeps the blocks it came from (a sand wall gives a sand ledge), and the ground under it keeps its own; **Paint** repaints sculpted matter too. Converting a terrain graph into the ground later drops what was sculpted on the tiles it rewrites (one step to undo).

## Player
**Player**, in the 3D Sculpt toolbar, walks in the view as in Minecraft with Axiom, coming down where the middle of the view looks. It looks like the game: the textures of the game's own blocks (from the Minecraft installed here), its light and shaded corners, its sky, sun, clouds and fog; the blocks the brush takes are marked in Axiom's lavender and the block under the cursor is outlined. The bar along the bottom lists what the buttons and keys do, with the flight speed.
- The cursor free, as in Axiom's editor: left drag (or the arrow keys) looks around, right uses the tool under the cursor (`Alt` + right: its inverse), middle picks the block under the cursor for **Paint**; `Ctrl` (`⌘`) + left turns about the point under the cursor, `Ctrl` + right moves the view, `Ctrl` + middle sizes the brush; the wheel sets the flight speed.
- Click the view to lend it the mouse, as in the game (`Esc` gives it back): it looks around as it moves, right uses the tool at the crosshair (builds), left its inverse (breaks).
- `W` `A` `S` `D` walk, `Ctrl` (or `W` twice) sprints.
- `Space` jumps; twice quickly, flies: `Space` goes up, `Shift` (or `Q`) down; twice again, or down to the ground, lands.
- `Shift` (or `Q`) sneaks: crouched, you do not step off an edge.
- Walking into a step of one block jumps onto it (Minecraft's Auto-Jump).
Water slows you and lets you swim, and under it the world turns blue and dim, as in the game; a fall off the map puts you back where you last stood. Turn **Player** off to get the orbit camera back.

# Terrain graphs
## Nodes
The **Terrain** side builds land from nodes, as in Gaea: generators (mountains, dunes, noise…), filters (erosion, rivers, lakes, terraces…), masks and outputs. Press `Tab` to add a node, drag from an output to an input to link them. `V` views the selected node in the viewport, `V` again goes back to the graph's output.

## The viewport
The viewport shows the graph's output in 3D, with its sun, clouds, haze and sea (**Render**). When nothing feeds the Height output, it says so: **Start from this map** adds a Map node, to work from the map that is open.

## Converting
**Convert to voxels** writes the graph to the map: into its ground, or over it (**over the map**: what the graph digs and raises goes into levels of their own, and the map stays as it was). Templates give a whole graph at once.

# WorldPainter worlds
**Import from WorldPainter…** opens a `.world` file: its dimensions, heights (tall worlds keep their height), terrains, water, biomes and custom terrains. WorldPainter's **Beaches** come in as it draws them: grass with patches of sand, gravel and clay. Layers Massif does not have yet are listed in the import report.

# Settings
**Settings…** (`⌘,`: in the **Massif** menu on a Mac, in **View** on Windows and Linux, or the home screen's sidebar) has six sections.
- **Appearance**: the theme, the accent colour, and whether the home screen's picture moves.
- **Typeface**: what the interface is set in — the system's own typeface, or Inter, Geist, IBM Plex Sans or Manrope, built into Massif. Each is shown in itself; a click sets the whole interface in it.
- **Sounds**: little sounds for what happens, made by Massif as it plays them. **Essential**: tasks done, saving, errors. **All**: the interface too (choices, switches, windows, workspaces). Each can be heard from there. The sound output is held only while a sound plays: headphones are free to go to another device.
- **Notifications**: the system tells when a long task ends (an export, an import, a conversion, a pass over the whole map): never, while Massif is in the background, or always.
- **Memory**: a limit for what Massif keeps decoded to be quick (the map's tiles, the map view's images), and what is in use now. Past it, what was used least is put away, compressed.
- **Developer**: developer mode offers a **Developer** panel in every area's menu: frames a second and the time of each, the memory the system counts, the processor's time, in live charts; what is kept decoded, the map, its tasks and the graphics card.

# Shortcuts
| `⌘N` `⌘O` `⌘S` `⌘E` | New, open, save, export |
| `⌘Z` / `⇧⌘Z` | Undo / redo |
| Left / right button | The tool / its inverse (also `Alt` + left) |
| Scroll | Brush size (`Shift`: one block at a time) |
| `⌘` + scroll | Turn the brush |
| `Ctrl` or `Alt` + scroll, pinch | Zoom the map |
| Middle button, `Space` + drag | Move the map |
| `−` `+` | Brush size |
| `1` … `9`, `0` | Intensity 10 % … 100 % |
| `[` `]` | Turn the brush |
| `R` `L` | Turn the light of the map |
| `⌥H` `⌥A` `⌥S` `⌥P` `⌥R` `⌥F` `⌥Y` | Height, Flatten, Smooth, Paint, Spray, Flood, Eyedropper |
| `Esc` | Cancel the stroke being drawn; walking, give the mouse back |
| `W` `A` `S` `D`, `Space`, `Shift`, `Ctrl` | Walk, jump (twice: fly), sneak (flying: down), sprint, with **Player** in 3D Sculpt |
| `Ctrl Space` | Maximise the area under the pointer |
| `Tab` | Add a node (terrain graph) |
| `V` | View the selected node (terrain graph) |
| `F1` | This help |
