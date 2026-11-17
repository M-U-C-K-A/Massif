# Getting started
Massif makes Minecraft maps. Shape the land, paint it, decorate it, then write it to a Minecraft world.

## The home screen
The home screen opens with Massif. On the left are the ways in: **New map**, **From a height map**, **From a real place**, **Open a map…** (a `.massif` file), **Import from WorldPainter…** (a `.world` file), and the maps opened lately. In the middle are the settings of a new map, with a preview of the whole map.
- **Size**: width × depth in blocks, in steps of 128. Type any number: when you press Enter or leave the field, it becomes the nearest step (5000 becomes 4992), and a note under it says so.
- **Height**: Minecraft's own (−64 to 320) or taller, up to 2032 blocks.
- **Land**: a landform (flat, plains, hills, mountains, islands) or a template of the terrain generator, and how the ground is painted.
- **Water**: its level, water or lava, and beaches along the shores.
- **Seed**: the same seed always gives the same land.
Press **Create map** (or `Enter`). A `.massif`, a `.world` or a height map dropped on the window opens too.

**Samples**, on the left, opens a small map made from a template of the terrain generator (Alpine range, Archipelago, Floating mountains), its relief built in a few seconds, to explore at once.

## The tour
The first time a map is open, a short tour of six steps shows the window: the World and Terrain sides, the tools, Brush Settings, the 2D map, the 3D view, saving and exporting. **Next** and **Back** (or the arrow keys) move through it, **Skip tour** or `Esc` ends it. **Help ▸ Take the tour** shows it again.

## A real place
**From a real place** on the home screen, or **File ▸ New World from a Real Place…**, makes a world of the relief of a place on the Earth; **Edit ▸ Relief of a Real Place…** puts one into the map open, into the selection by its weight or over the whole map. Its land is painted again from the new relief, the old map's painted terrains, biomes, covers and objects taken off (**Terrains ▸ Keep** keeps them and writes only heights and water). Over the whole map, **Change it** under **Map** makes the map another size in the same step (whole tiles of 128 blocks, up to 32,768 a side, around its middle; what is painted outside the new size goes); one undo brings the old map and size back.
- **Source**: **Terrain Tiles**, the heights of the whole Earth, about 30 m apart, ocean floors included, downloaded as needed; or **A file** you downloaded: a GeoTIFF (its metres a pixel read from it), a 16-bit PNG or a raw `.r16` / `.r32` (say how many metres a pixel covers and what black and white stand for).
- **Place**: search a place by name (press `Enter`), or type its latitude and longitude ("45.83, 6.86"), or its bounds. Drag the small map to move, scroll to zoom; the orange outline is the map.
- **Size and scale**: the new world's width and depth (up to 32,768 blocks a side, about two minutes to make), and **Blocks a metre** (1: a block is a metre; 0.1: ten metres).
- **Heights**: **Exaggeration** makes the relief taller; **Sea level** is where the sea stands, and **Real height there** the real height that stands at it (0: the sea's; higher for a place inland, its valleys then filled). **Range** and **Lowest**, **Highest**: the heights the ground keeps to; **Squeeze into the range** brings mountains too tall (and seas too deep) into it rather than cutting them off.
The picture under the map shows the ground it makes and the heights it reaches. Gaps, spikes and pits of the data are filled from the heights around them; sand lies only along the water, snow only above the snow line of the latitude. Tiles are kept on disk: a place seen once works offline. The ground is painted by the automatic paint; in the map it is one step to undo. Elevation: Terrain Tiles by Mapzen on AWS Open Data (SRTM, GMTED2010, ETOPO1, 3DEP, EU-DEM and others; **All the sources** in the dialog lists them); search: Nominatim, © OpenStreetMap contributors.

The picture behind the home screen changes each time Massif opens, the ones not shown yet first; the button at the bottom right shows another.

## Saving and exporting
- `⌘S` saves the map as a `.massif` file. The Finder and Explorer show the map as the file's picture.
- `⌘E` exports to Minecraft: the saves folder, the world's name, the version of the game, its rules, its border and data packs.
- From 1.18.2 to 26.3, the chunks, `level.dat` and the world's folders are written as that version writes them itself: it opens the world without converting it or asking for a backup. Blocks and biomes the version does not have yet become the closest ones it has (a cherry log a birch log, a pale garden a dark forest); renamed blocks take the version's name (short grass is grass before 1.20.3).
- The water is written as the game leaves it: flowing water at the edges of the steps and falls of rivers, so that no pool rises and nothing overflows when a player changes a block beside the water. Blocks of objects standing in water are waterlogged.
- **Test Export…** writes only the 3 × 3 tiles around the middle of the map view to a separate world, to check the result in the game quickly.

## Autosave
- While the map has changes not saved, Massif keeps a copy of it every few minutes (**Settings ▸ Autosave**: on or off, every 1 to 15 minutes), in a folder of its own, never over your file. It is written in the background: the map stays smooth, even a huge one.
- Should Massif close without saving (a crash, a forced quit, a power cut), the next start offers **Restore unsaved changes?**: the map's name and when its copy was taken. **Restore** opens it with changes to save; **Discard** deletes the copy; **Later** keeps it for the next start.
- Should Massif run into an error while you work, it writes a copy of the map at once and says so: **Save As…** keeps the map under a name of your own, **Carry On** goes back to work, **Quit** keeps the copy for the next start.
- Saving never harms the file already there: a full disk, a folder that cannot be written or a file held by OneDrive or an antivirus leaves it as it was, and the message says what to do.
- Saving the map, or closing it, removes its copy. At most 5 copies are kept.

## Save points
- **File ▸ Save Point…** keeps the map as it is now under a name, to come back to it. The **Versions** panel lists the points with a picture and their date.
- **Restore** puts the map back as it was then, as one step to undo. **Compare** shows on the 2D map where the land is higher or lower (accent) or painted differently (grey).
- In a point's **…** menu, **Open as a copy** opens it as a new map; **Open read-only** only shows it. **Rename…** and **Delete…** too.
- The points are kept in the `.massif` file and share its unchanged tiles: a point costs only what changed since. Older versions open the file as the map alone.

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
- `⌘Q` asks first when the map has changes that are not saved, as **File ▸ Quit** does, and so does quitting from the Dock or the app switcher (`⌘Tab`, then `Q`): Massif comes to the front to ask. A logout, a restart or a shut-down waits for the answer: **Save** or **Don't Save**, and it goes on once Massif has quit; **Cancel**, and it stops.

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

## The 2D map
The buttons in the map's header pick what it shows (**Terrain**, **Height**, **Slope**, **Biome**, **Relief**) and what goes over it: contour lines, **Block steps**, the tile and chunk grid. With **Block steps** on (the default), close up the map is shaded block by block, each at the height the game will give it and compared with its neighbour on the side the light comes from, as the game's own maps are: every step of a block shows as a sharp line, lit where it faces the light and in shadow at its foot, and the contour lines follow the blocks. Off, the shading follows the smooth shape of the ground. Zoomed out, where a pixel spans several blocks, the shading is the smooth one either way.

## Messages
What Massif did, and what it does in the background (saving, exporting, automatic tools over the whole map), shows at the bottom right of the window. Long tasks have a **Cancel** button.

# Sculpting
## The tools
- **Raise / Lower**: left button raises, right button lowers.
- **Smooth**: rounds the land off.
- **Flatten**: levels the land to the height where the stroke starts. In Tool Settings, **Both ways** (the default), **Only raise** (fills what is under the height, leaves what is over it) or **Only lower** (the other way round); **Apply theme** repaints the land it flattens with the terrain its new height calls for.
- **Set height**: sets the land to a height. `Alt`-click on the map picks it.
- **Roughen**: adds or removes roughness.
- **Road**: click points on the map: a smooth road runs through them. Drag a point to move it, drag the road to add one, right-click or `Delete` to take one off; `Enter` builds it, `Esc` drops it. In Tool Settings: its **Width**, **Banks**, **Steepest slope** (it cuts and fills to climb no steeper), **Surface** (a path, gravel, cobblestone, stone bricks, a terrain or the Paint tool's noise) and **Bridges over water** (a deck a block over the water, on a level named Bridges; without them the road is built up out of the water). It keeps to the selection; the brush filters choose where its surface is painted. One step to undo.

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

# Selections
## Selecting an area
The **Select** tools mark the part of the map the other tools change: once something is selected, every brush, a fill, the 3D tools on the map, **Auto biome**, **Auto paint**, noise painting and **Edit ▸ Whole map** act only inside it, and fade along its edge where it is feathered. A dashed outline, its dashes marching (still with **Reduce motion**), shows it on the map.
- **Rectangle** and **Ellipse**: drag across the map. **Lasso**: draw around an area; let go and it closes. **Selection brush**: paint it with the brush (right button: take it out); its soft edge feathers it.
- Shift adds to the selection, Alt takes out of it, both together keep only what both hold; the **New · Add · Subtract · Intersect** buttons set what a shape does with no key held. A click without dragging deselects.
- **Feather** fades new shapes over that many blocks on either side of their edge (up to 64); **Feather it** softens the selection there is.
- **Select by** picks the columns of the whole map with a terrain (the one picked in Terrain or Blocks), between two heights, or on slopes steeper or gentler than an angle, as the brush filters read them.
- **All**, **Invert** and **Deselect** are in the tool settings and in the Edit menu (⌘A, ⇧⌘I, ⌘D).

Each change of the selection is a step to undo. The selection is saved in the map's file (versions before 0.6 open the file without it). **Export** can keep to it: **Only the selection** writes only the tiles of 128 × 128 blocks it reaches, to try a part of the map in the game.

# Painting
## Terrains and biomes
**Paint** gives the ground a terrain: pick it in the Terrain palette. To blend two terrains into each other instead of meeting on a line, take the **Gradient brush**: beside Paint in the Paint row of the Tools panel (a swatch fading out, with a dot until you try it), or **Gradient** among the ways to paint at the top of Paint terrain's Tool Settings (**One terrain**, **Noise**, **Gradient**). **Biome** paints biomes the same way: pick one in the Biomes palette. The right button takes a biome back to the automatic one, and a terrain back to the map's own: the terrain a new map was filled with (one covering nearly all of an older map), shown under **Right button** in Tool Settings, where **Change** chooses another. On a map whose terrains came from its height, or with **By height**, it puts back what the height calls for: grass, sand by the water, rock and snow up high.

## Conquest Reforged and other mods

- Where Conquest Reforged is installed, the Blocks panel offers it: **Use** reads its instance in one step, its mod, its list of blocks for WorldPainter (`conquest_wp.csv`) and its resource packs, over Minecraft 1.20.1, the version it plays.
- **Blocks ▸ Mods ▸ Instance…** reads any modpack's instance the same way; **Add mods…** takes jars of mods with blocks (Conquest Reforged, Chipped, Macaw's…), resource packs (`.zip`) and lists of blocks made for WorldPainter (`.csv`) one by one. Click a chip to take it out.
- A mod's blocks show in its own creative tabs (Conquest Reforged's 36), with their textures; **All shapes** adds each block's slabs, stairs, arches and other shapes after it. A whole block paints as a terrain, any other as a ground cover; tall plants grow in two halves, and plants grow on the mod's own grass, moss and soils.
- **Export** picks the version the mod is made for (1.20.1 for Conquest Reforged) and its instance's saves, and warns in red when the version or the mod does not match. Blocks are written by their own ids and states: open the world with the same mods in the game.

## Automatic biomes and paint
- **Auto biome** reads the land and gives it biomes: water, rivers and beaches, lowland, hills, mountains, peaks, cliffs, from its water, height, slope and ground. Sand at the water's edge is a beach; sand far from the water is a desert only where sand is all around, not a patch in a plain.
- **Auto paint** paints the ground as its biome calls for: a terrain for its ground, one for its cliffs, one under its water.
A map filled with one terrain other than grass (a WorldPainter map all of sand, for instance) is read from the shape of its land: its fill says nothing about it.

Hovered over the map, they show what the brush would change, in its new colour; what already has what the land calls for (painted by an earlier stroke, say) is hatched, and a bar under the map names what the columns become and how much stays as it is. Biomes show on the map in its **Biome** mode.

Both work with the brush, or over the whole map at once: **Whole map** in the tool settings, or **Edit ▸ Auto biome: whole map** and **Auto paint: whole map**. Over the whole map they run in the background, with their progress, and are one step to undo. **Rules…** sets the heights and slopes the land is read with, the biome of each kind of land and the terrains of each biome.

Hovered over the map, the brush shows in colour what would change.

## The whole map at once
**Edit ▸ Whole map** runs one of the terrain graph's filters over the whole map: **Erosion** (rain carries soil from the heights to the valleys), **Thermal erosion** (loose rock slides where a slope is too steep to hold it), **Rivers** (traced where the water of the map runs, dug into the valleys, their beds in gravel, falling from pool to pool down steep valleys), **Lakes** (water fills the hollows up to where they spill), **Smooth**, **Sharpen**, **Terraces**, **Strata**, and **Scale and raise** (the relief scaled around the sea level, then moved up or down). Each opens with its settings — those of its node in the graph — and the map small, before and after, made again as they change. **Apply to the whole map** runs it in the background, with its progress and a **Cancel**; it is one step to undo. The ground's paint, its levels and what was sculpted in 3D stay as they are; maps over 4096 blocks across are worked out a sample every few blocks, the change spread over the blocks between, so they keep their detail.

## Caves
**Edit ▸ Caves…** carves caves into the rock, over the whole map or the selection: **Tunnels and chambers** (winding tunnels that branch, chambers with pillars), **Ravines** (long, deep, narrow cuts) and **Caverns** (large halls). **Density** sets how many, **Size** how wide and long, **Highest** and **Lowest** the heights they keep to (with a roof of 5 blocks under the ground), **Open to the surface** the share of tunnels and ravines that reach the sky. **Fill with** puts a lake of water or lava in the caverns up to a height. The same **seed** carves the same caves. **Carve** runs in the background and is one step to undo; the caves are 3D matter, as dug in 3D Sculpt: they show in the 3D views and are exported as air, their lakes as water or lava.

## Noise painting
**Noise** (in the Paint tool's settings) paints with several terrains or blocks at once, mixed by a noise, as Axiom's noise painter does: patches, veins, blobs, cells or stripes, of simplex, Perlin or value noise, with its size, detail (octaves, lacunarity, gain), stretch and direction. The blocks are a list in order — neighbours meet on the ground — each with its share of the ground; drag one to move it, click its name to change it, **Add a block** to add one. Presets are a start, and a pattern can be saved under a name. Hovered over the map, the brush shows what it would paint; **Paint the whole map** lays it everywhere the brush options let it, in the background.

## Gradient brush
**Gradient brush** (in the Paint tools) blends two terrains over their edge instead of a hard line: paint the base terrains first, then drag it over the boundary — rock, then a rim, then grass. A gradient is terrains in **bands** of so many blocks, A → B → C, with a **transition** between each two: a **Hard cut**, **Noise** (a ragged edge; its intensity and scale) or **Blobs** (of every size, denser and denser; their density and size), and a **midpoint** where the two are half and half. A band can be any terrain, or any block of the game or of its mods.
![gradient]
**Across an edge**, each column takes the band its distance to the boundary says: the bands before the **edge** lie on the side of the first terrain, the others on the side of the last; past the gradient's ends the ground stays as it is. **Wander** widens and narrows the bands along the edge. In a selection it runs in from the selection's edge. **From the centre** lays it from the brush's centre to its rim: a patch with rings. The right button paints it the other way round. It keeps to the selection, filters, intensity and soft edges, one step to undo, the same ground for the same stroke.
**Edit…** opens its editor: **Presets** to start from; drag the stops under the bar to move blocks between neighbours, click between them to add one, drag a diamond to move a midpoint and the marker over the bar to move the edge; click a band or a transition to change it. **Save** keeps it among your gradients and in the map; **Export…** and **Import…** share it as a `.massifpack`, and **File ▸ Export Pack…** offers the map's gradients.

# Decor
## Ground covers
**Ground cover** paints what grows over the ground: grass, tall grass, flowers, ferns, mushrooms, snow, seagrass, or any blocks of your own, with their density and how high they grow. **Frost** and **Lava** come with every new map, as in WorldPainter: Frost lays snow and turns the top of the water to ice, Lava turns the water into lava. Any cover can do either: **The water** in its settings (As it is, Frozen, Lava). Older maps find them in Packs.

## Layers
How every layer fits together (kinds, the slots of a block, masks, the lock) is in the guide's Layers page. The **Layers** panel (under Terrain; or any area's chooser, in Paint) lists everything painted over the ground, as WorldPainter's layers: the map's ground covers, its object layers and the terrain mixes saved for noise painting. Click one to paint with it (its settings open in Tool Settings), double-click to rename it, click its eye to hide it on the map and in the 3D view (the export writes it all the same), and right-click for Settings… (a dialog of their own), Duplicate, Move up and down, Lock and Remove. The panel lists them by the slot of a block they paint: terrain mixes, ground covers, trees and objects, under the trees, then the tree sizes and the annotations. A **locked** ground cover or tree layer keeps its blocks: no brush paints another layer of its slot over them, nor rubs them out, until it is unlocked (the lock shows on the right of a row pointed at). **New cover**, **Import objects…** and **New mix** make new ones. At the bottom, the fifteen colours of the annotations: a click paints with one.

## Base layers, sub-canopy and tree sizes
A block has one plant layer and one tree layer: painting another over it takes its place. A tree layer can be a **sub-canopy** layer instead (**Canopy ▸ Sub-canopy** in its settings): it is painted beside the trees, not in their place, and its objects (hazel, holly, bushes) grow under their canopy, in the air the trees leave. The Bushes and Azalea bushes of Packs are sub-canopy layers. **Tree sizes** (Decor group, and four swatches in the Layers panel) marks where bushes, small, medium or large trees grow: paint the sizes, then one species over everything, and each spot takes its objects of the size marked. A layer's objects are split by height into four sizes, a quarter each; every kind of tree of the packs comes in the four sizes (its smaller ones, of weight 0, stand only where their size is painted). How every layer works together: see Layers in the guide.

## Annotations
**Annotations** (Paint group) marks the map in colour to plan it, as WorldPainter's annotations: where a town goes, a road, a river to dig. They show on the map in every mode, whatever the level, and are never exported. The right button rubs them out; the pen button in the map's header hides them.

## Packs
**Packs** (the second tab of the Layers panel, or an area of its own) holds ready-made things to paint, as WorldPainter Modernised's Packs panel did: sixteen kinds of trees of the tree pack (tall and slender spruces, firs, larches, giant redwoods, big and pale oaks, birches, cherries, acacias, willows, rainforest giants, palms, dead trees, bushes, azaleas), each in eight shapes; presets of flowers, plants and water plants; and mixes of blocks for grass, soil, stone, sand, cliffs in beds, snow and volcanic rock. A click adds one to the map — an object layer, a ground cover or a terrain — and picks it to paint; a dot marks those the map has already. A search field filters them.

### Sharing packs
**File ▸ Export Pack…** makes a `.massifpack` file of the map's layers, to share: tick the object layers, ground covers and terrain mixes it holds, and give it a name, an author, a description and a picture (drawn from what it holds unless you choose one). **File ▸ Import Pack…**, the **Import…** button of the Packs panel, a file dropped on the window, or a double-click on it in the Finder or Explorer adds a pack to **Your packs**, at the top of the Packs panel, with its picture and author; its things paint like Massif's own. A name you have already asks to replace that pack or keep both under another name. The **⋯** menu of a pack saves a copy of it, or removes it from your packs (maps keep the layers they took from it). A pack holds blocks and settings only, never anything that runs; Massif checks every part of it and refuses a damaged, oversized or foreign file.

## Custom objects
**Objects** places trees, rocks or ruins from `.schem`, `.schematic`, `.bp`, `.litematic` and `.nbt` files: import them, or drop them on the window, then paint where they stand, how far apart and how many.

# Water
- **River** lays a river from a click to another: its bed, its banks and its water. Its bed is flat and wide on gentle ground and a narrow V on steep ground, its banks sloping down to it as a valley; down steep slopes it falls from pool to pool, a plunge pool under each fall; its shallows and the inside of its bends are sand, and into the sea its channel goes on under the water, fading into the sea floor.
- **Flood** fills the hollow you click with water up to where it would spill over its rim, at any height; the **Water brush** lays water as high as where each stroke starts (the water's surface there, or a block over the ground). With **From the click** off (Tool Settings), both use the **Water level** you set; **Limit the water's height** (off by default) keeps them under a height of your choice. When nothing fills, the message says why. **Drain** empties a body of water.
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
- In the **Terrain** look every block shows its own texture, from the game and from the mods and resource packs read in the Blocks panel, with what grows on it and the shade in its corners; far off, where a block is smaller than a pixel, it blends into the colour of its texture. **Settings ▸ Graphics ▸ Game textures in the 3D views** turns it off (Low draws a flat colour a block).

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
The **Terrain** side builds land from nodes, as in Gaea: generators (mountains, dunes, noise…), filters (erosion, rivers, lakes, terraces…), masks and outputs. Press `Tab` to add a node: type part of its name (in any language, or its letters in order), choose with ↑ ↓ and add it with Enter; the nodes added last come first. Drag from an output to an input to link them. `V` views the selected node in the viewport, `V` again goes back to the graph's output.

## The viewport
The viewport shows the graph's output in 3D, with its sun, clouds, haze and sea (**Render**). When nothing feeds the Height output, it says so: **Start from this map** adds a Map node, to work from the map that is open.

## Converting
**Convert to voxels** writes the graph to the map: into its ground, or over it (**over the map**: what the graph digs and raises goes into levels of their own, and the map stays as it was). Templates give a whole graph at once.

# WorldPainter worlds
**Import from WorldPainter…** opens a `.world` file: its dimensions, heights (tall worlds keep their height), terrains, water, biomes and custom terrains. WorldPainter's **Beaches** come in as it draws them: grass with patches of sand, gravel and clay.

Its layers come too. Forests (Deciduous, Pine, Jungle, Swamp) and the trees of WorldPainter Modernised's tree pack become object layers of the same trees, as dense as painted. Custom object layers become object layers, their objects read from the world itself (Sponge and MCEdit schematics, structures, Bo2) with their offset, frequency and random turn. Plant layers become ground covers with the same plants in the same shares. **Frost**, **Lava**, **Void** (columns of nothing) and **Annotations** come in as Massif's. Caverns, chasms and caves are carved where they were painted, the more the stronger. Each tunnel layer becomes a Carve level (a floating one, a Solid level) with its floor and roof where WorldPainter puts them. A custom ground cover becomes a terrain of its blocks over the terrain under it, the land raised by its thickness. A combined layer brings the layers it holds, at their factors, and its terrain and biome. The import report says what became what. Underground pockets and rivers are not imported yet: the report lists them with the tiles they covered.

# Settings
**Settings…** (`⌘,`: in the **Massif** menu on a Mac, in **View** on Windows and Linux, or the home screen's sidebar) has ten sections.
- **Appearance**: the theme, the accent colour, the loader shown while Massif works (a grid of dots in one of eight patterns), the tool icons (Line or Classic), whether the home screen's picture moves, and the crosshair of the player's view (a dot, a cross, a circle, Minecraft's plus or none, its size and opacity, with a preview).
- **Typeface**: what the interface is set in — the system's own typeface, or Inter, Geist, IBM Plex Sans or Manrope, built into Massif. Each is shown in itself; a click sets the whole interface in it.
- **Sounds**: little sounds for what happens, made by Massif as it plays them. **Essential**: tasks done, saving, errors. **All**: the interface too (choices, switches, windows, workspaces). Each can be heard from there. The sound output is held only while a sound plays: headphones are free to go to another device.
- **Notifications**: the system tells when a long task ends (an export, an import, a conversion, a pass over the whole map): never, while Massif is in the background, or always.
- **Memory**: a limit for what Massif keeps decoded to be quick (the map's tiles, the map view's images), and what is in use now. Past it, what was used least is put away, compressed.
- **Autosave**: a copy of the map while it has changes not saved, every 1, 2, 5, 10 or 15 minutes; or none.
- **Updates**: **Check for new versions** (on by default): once a day at most, Massif asks GitHub whether a newer version is out, sending nothing about you or your maps. If one is, a pill in the top bar (and a line above Settings on the home screen) opens its notes and a button to its page, in your browser; Massif never downloads or installs anything itself. **Skip this version** stops telling of that one.
- **Graphics**: **Low**, **Medium** or **High** (or **Custom**, any mix): the 3D views' render scale and smoothing, fog, water reflections, the game's textures in the 3D views, how much of the map the 3D view keeps on the graphics card (a larger map is drawn coarser, and much faster), the Terrain viewport's light and clouds, the map view's images on the graphics card, and the frames a second of an idle window. Changes apply at once. The first start picks a preset for the graphics card; when the 3D views stay slow, a note offers one step lower.
- **Developer**: developer mode offers a **Developer** panel in every area's menu: frames a second and the time of each, the memory the system counts, the processor's time, in live charts; what is kept decoded, the map, its tasks and the graphics card. **Frames a second** keeps the window in step with the screen (**Vsync**, the default) or lets it draw up to 60, 120, 144, 240 frames a second or **Unlimited**; **Draw every frame** keeps it drawing when nothing changes, to measure.

# Reporting a problem
**Help ▸ Report a Problem…** makes one `.zip` file to send us on Discord when Massif crashes or is slow. Nothing is sent from Massif: you save it and send it yourself.
- It holds what happened in your words, Massif's version, the computer and its graphics card, the settings that change how Massif draws, the map's size, the last minute of the **Developer** panel's figures, and the logs of the last two starts. The map itself goes in only if **Include the map** is on.
- Every file is listed, and **Show** opens it as it will be written. Your home folder is written `~`, and other accounts' names are left out.
- **Copy Summary** puts a few lines on the clipboard for your message.
- For slowness, **Start Recording**, do the slow thing, then **Stop** at the top of the window: each frame and what took its time go in the report.
- After Massif closed without saving, the dialog that offers your map back has **Report the Problem…** too.

## If Massif does not start
Massif writes every step of its start in `startup.log` (on Windows in `%LOCALAPPDATA%\Massif`, on macOS in `~/Library/Logs/Massif`, on Linux in `~/.local/state/massif`), the start before in `startup.previous.log`. Once Massif starts again, **Help ▸ Report a Problem…** puts them in the report; if it never starts, send those two files. A graphics API whose driver got stuck is set aside in `graphics.txt` beside them: delete it to try them all again.

# Shortcuts
Massif's own keys, as they are out of the box. **Settings ▸ Keyboard** changes any of them: click a shortcut, press the new keys (another action's keys are asked about first: Replace or Cancel), right-click one to remove it. It starts from **Massif** or **WorldPainter**'s keys (`⌘Y` redo, `⌘` `+` `−` `0` zoom the map), resets one action or all, and exports or imports a keymap file. Shortcuts wait while a text field is typed in, but those with `⌘` typing has no use for, as Save. The mouse keeps its own.

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
| `⌘A` `⌘D` `⇧⌘I` | Select the whole map, deselect, invert the selection |
| `Shift` / `Alt` + drag | Add to the selection / take out of it (both: intersect) |
| `Esc` | Cancel the stroke or the selection being drawn; walking, give the mouse back |
| `W` `A` `S` `D`, `Space`, `Shift`, `Ctrl` | Walk, jump (twice: fly), sneak (flying: down), sprint, with **Player** in 3D Sculpt |
| `Ctrl Space` | Maximise the area under the pointer |
| `Tab` | Add a node (terrain graph) |
| `V` | View the selected node (terrain graph) |
| `⌘+` `⌘−` `⌘0` | Larger, smaller interface; back to its size |
| `F1` | This help |
