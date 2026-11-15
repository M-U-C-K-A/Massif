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

# The window
## Areas
The window is made of areas, as in Blender. Each area shows one editor: the map, the 3D view, tools, settings, brushes, palettes, levels or information.
- The **…** menu of an area splits it, joins it with its neighbour, maximises it or closes it.
- Drag the top-right corner of an area into it to split it where you let go.
- Drag an area by its title onto another to move it there, onto its centre to swap them. Drag the gaps between areas to resize them.
- `Ctrl Space` maximises the area under the pointer, and gives it back its place.

## Workspaces
The tabs at the top are workspaces: arrangements of areas for a kind of work. The two sides of Massif are switched at the top left: **World**, the map editor, and **Terrain**, the terrain generator.
- `+` adds a copy of the workspace shown.
- Right-click a tab: **Reset** puts it back as it was at first, **Duplicate** copies it, **Delete** removes it.

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
- **Height limits**: only at or above a height, at or below one, between two or beyond them. The range is the map's own: from its bottom (−64 for a Minecraft map, or the bottom its file says) to its top. Drag the dashed lines in the picture, or type the heights.
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

## Patterns
A pattern paints with several terrains at once: stripes, patches or noise, at a scale and an angle.

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

## 3D Sculpt
**Build**, **Dig**, **Smooth** and **Paint** work on the blocks in 3D, with the brush of every tool: its shape, size, intensity and rotation are set in **Brush Settings**, shown beside the 3D Sculpt view (`⌘` + scroll changes its size there too). Left button sculpts where the cursor points, right button inverts; held down, it keeps adding or digging. In 3D a brush sculpts 64 blocks around at most (a larger one works as 64). Matter pushed out keeps the blocks it came from (a sand wall gives a sand ledge), and the ground under it keeps its own; **Paint** repaints sculpted matter too. Converting a terrain graph into the ground later drops what was sculpted on the tiles it rewrites (one step to undo).

## Player
**Player**, in the 3D Sculpt toolbar, walks in the view as in Minecraft, coming down where the middle of the view looks. Click the view to lend it the mouse (`Esc` gives it back), then:
- The mouse looks around; the buttons sculpt at the crosshair.
- `W` `A` `S` `D` walk, `Shift` (or `W` twice) runs.
- `Space` jumps; twice quickly, flies: `Space` goes up, `Q` down; twice again, or down to the ground, lands.
- `Q` sneaks: crouched, you do not step off an edge.
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
**Massif ▸ Settings** (`⌘,`, or the home screen's sidebar) has four sections.
- **Appearance**: the theme, the accent colour, and whether the home screen's picture moves.
- **Sounds**: little sounds for what happens, made by Massif as it plays them. **Essential**: tasks done, saving, errors. **All**: the interface too (choices, switches, windows, workspaces). Each can be heard from there. The sound output is held only while a sound plays: headphones are free to go to another device.
- **Notifications**: the system tells when a long task ends (an export, an import, a conversion, a pass over the whole map): never, while Massif is in the background, or always.
- **Memory**: a limit for what Massif keeps decoded to be quick (the map's tiles, the map view's images), and what is in use now. Past it, what was used least is put away, compressed.

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
| `W` `A` `S` `D`, `Space`, `Q` | Walk, jump (twice: fly), sneak, with **Player** in 3D Sculpt |
| `Ctrl Space` | Maximise the area under the pointer |
| `Tab` | Add a node (terrain graph) |
| `V` | View the selected node (terrain graph) |
| `F1` | This help |
