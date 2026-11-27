# Using Massif

[← Back to the README](../README.md)

Massif makes Minecraft Java maps. You shape the land, paint it, grow plants and trees on it, dig caves, lay rivers, roads and villages, then write it as a world the game opens as it is. It works the way WorldPainter does (the same brushes, layers and keys, and it opens `.world` files), and adds a node-based terrain generator in the spirit of Gaea, true 3D sculpting, a path-traced renderer and a lot more.

This guide follows the app area by area, one page each, in the order you meet them: getting started, the window, the map and its tools, the 3D views, mods and blocks, importing and exporting, then saving, working together, scripting and settings. The **Terrain** side, where the node generator lives, has [a guide of its own](terrain-graphs.md). Other pages go deeper:

- [Coming from WorldPainter](coming-from-worldpainter.md): where each of its tools is in Massif, and [its scripts](coming-from-worldpainter-scripts.md).
- [Layers](layers.md): every kind of layer, its settings, and how WorldPainter's are read.
- [Structures and villages](structures.md), and [making a village pack](village-packs.md).
- [Importing a Minecraft world](import-minecraft.md).
- [Working on one map together](versioning.md).
- [Scripting](scripting.md): WorldPainter's JavaScript scripts and Massif's Lua API.
- [MassifScript](massifscript.md), the terrain graph written as text, and [its reference](massifscript-reference.md).
- [Every node of the Terrain Generator](terrain-nodes.md).
- [How Massif works inside](internals.md).

Inside the app, **Help ▸ Massif Help** (F1) holds the same guide in short, with a search.

A note on the pictures: they are rendered by the app itself (see `doc_captures_tests.rs`), in the dark theme. Lines still marked **📷 Screenshot** name a picture not taken yet, and where to take it in the app.

## Getting started

### Install

Builds for Windows, macOS (Apple silicon and Intel) and Linux are on the [releases page](https://github.com/M-U-C-K-A/Massif/releases). Download the one for your system and unpack it.

- **macOS**: move **Massif.app** to Applications. The builds are not signed: the first time, right-click Massif.app and choose **Open** (or run `xattr -dr com.apple.quarantine Massif.app` in Terminal).
- **Windows**: run `Massif.exe`. If SmartScreen asks, choose **More info ▸ Run anyway**. Massif needs no installer; it registers its file icons for your user the first time it starts (see [In the Finder and Explorer](#in-the-finder-and-explorer)).
- **Linux**: run the `massif` binary from the archive.

Massif reads its blocks, textures and structures from **your own Minecraft Java**: the launcher's install is found by itself. Nothing of Minecraft ships with Massif. Without the game, Massif still works, with flat colours in place of textures; you can point it at a client jar in **File ▸ Game & mods…** (see [Mods and resource packs](#mods-and-resource-packs)).

Massif tells you when a new version is out, once a day at most (see [New versions](#new-versions)); it never downloads or installs anything itself.

> 📷 **Screenshot:** the macOS Finder's right-click menu on Massif.app with **Open** highlighted (first launch of an unsigned build).

### Your first map

1. Start Massif. It opens on the [home screen](#the-home-screen). The very first time, it asks **Coming from WorldPainter?**: **Yes** lays the window out like WorldPainter and sets its keys; **No** keeps Massif's own layout.
2. Choose **New map**, give it a size (1,024 × 1,024 blocks is a good first map), a **Land** (try **Hills**) and a **Seed**, and press **Create map** (or Enter).
3. The map opens on the **World** side. Pick **Raise / Lower** in the **Tools** panel and paint on the map: the left button raises the ground, the right button lowers it. The wheel sizes the brush.
4. Pick **Paint terrain**, choose a terrain in the **Terrain** panel, and paint it on. **Ground cover** and **Objects** grow grass, flowers and trees where you paint them.
5. Look at it in the **3D View** (drag to turn, scroll to come closer, WASD to fly).
6. **File ▸ Save** (⌘S, Ctrl+S on Windows and Linux) keeps it as a `.massif` file.
7. **File ▸ Export to Minecraft…** (⌘E) writes it into your Minecraft saves folder. Open Minecraft, and the world is in the singleplayer list.

A short tour shows the window the first time a map is open; **Help ▸ Take the tour** shows it again.

![A new Hills map in the World side's Layout workspace, Raise / Lower in hand](images/first-map.png)

### Samples

Under the ways in, **Samples** makes one of three small maps (768 blocks a side) for you to explore at once: **Alpine range**, **Archipelago** and **Floating mountains**. None is shipped with Massif: a click builds the map from that template of the Terrain Generator (a few seconds), and it opens with nothing to save until you change it.

### Where to go next

- Coming from WorldPainter: [where each of its tools is](coming-from-worldpainter.md), and **Help ▸ Where are my WorldPainter tools?** in the app.
- Bring a world in: a WorldPainter `.world`, a Minecraft world, a height map or a real place ([Importing](#importing)).
- Make a landscape from nodes: [the Terrain Generator](terrain-graphs.md).

## The home screen

Massif opens on its home screen, **What will you build?**. **Help ▸ Home screen**, or ⌘N (File ▸ New World…), comes back to it from a map, with **Back to** *the map* (Esc) to return.

On the left are the ways in:

- **New map**: the settings of a new map in the middle (below).
- **From a height map**: a greyscale picture from Gaea, World Machine or an image editor (16-bit PNG, JPEG, `.r16` or `.r32`): its **Scale** in blocks a pixel and the heights black and white stand for (**Black is Y**, **White is Y**). See [Height maps](#height-maps).
- **From a real place**: the relief of a place on the Earth, at the scale you choose. See [Real places](#real-places).
- **Open a map…**: a Massif map (`.massif`).
- **Import from WorldPainter…**: a WorldPainter world (`.world`). See [Importing](#importing).
- **Import a Minecraft world…**: a Minecraft Java world, its folder or its region files (`.mca`). See [Importing a Minecraft world](import-minecraft.md).
- **Samples**: three small maps made from templates (see [Samples](#samples)).
- **Recent maps**: the maps opened lately, each with its picture and when it was opened. Right-click one for **Show in Finder** (**Show in Explorer** on Windows, **Show in folder** on Linux) or **Remove from the list**.
- **Settings** (⌘,), and a line naming a new version of Massif when one is out.

A `.massif`, a `.world` or a height map dropped on the window opens too.

The settings of a new map sit beside a preview of the whole map, as tall as it is (the cross marks its centre), redrawn as you change them:

- **Name your map**.
- **Size**: width × depth in blocks, in whole tiles of 128 blocks, up to 65,536. Type any number: on Enter or leaving the field, it becomes the nearest multiple and a note says so (5000 becomes 4,992). The chain between the two links them for a square map; click it to set them apart. The line under it says how long the map takes to cross on foot.
- **Height**: Minecraft's own (Y −64 to 320) or taller, in whole sections of 16 blocks, up to Y 2,032 and down to −2,032 (taller worlds come with a data pack the export writes).
- **Land**: a landform (**Flat**, **Plains**, **Hills**, **Mountains**, **Islands**) with its **Relief** height, **Features** size and **Ground** level, or a template **From the terrain generator** (alpine range, continents, archipelago…), whose map then opens on the Terrain side with its nodes.
- **Automatic surface** (grass, sand, rock and snow by height and slope) or one terrain for the whole ground.
- **Water**: its **Level**, **Lava** in place of water (opaque and glowing, its shores scorched basalt and blackstone), and **Beaches** of sand along the water.
- **Blocks**: the block palette the terrains are made of: **Vanilla**, a palette of a mod read for this map, or one of your own (see [Block palettes](#block-palettes)).
- **Seed**, with **Another seed**: the same seed always gives the same land.

**Create map** (Enter) makes it.

[![New map selected on the home screen, with dimensions, land and water settings, a live preview and Create map.](assets/workspaces/new-map.webp)](assets/workspaces/new-map.webp)

### The tour

The first time a map is open in Massif, a short tour shows the window, in six
steps: the World and Terrain sides, the tools, Brush Settings, the 2D map, the
3D view, and saving and exporting. Each step dims the window but for the part
it talks about and says a few words beside it. **Next** and **Back** (or the
arrow keys) move through it; **Skip tour** or Esc ends it at any step. With
**Don't show again** ticked, as it is at first, it does not come back on its
own; **Help ▸ Take the tour** shows it again (over a sample, from the home
screen). Massif shows no other tips or popups. With Reduce motion on, the
light jumps from one part to the next rather than gliding.

Before it, the very first start asks **Coming from WorldPainter?** **Yes**
opens the WorldPainter workspace (the map big in the middle, the tools on the
left, Terrain, Layers and Brushes on the right), sets WorldPainter's keyboard
shortcuts, and shows a tour of its own instead, eight steps from where
WorldPainter put each thing to where Massif has it, then what Massif adds.
**No** keeps Massif's layout and the tour above. **Help ▸ Where are my
WorldPainter tools?** shows that tour again. [Coming from
WorldPainter](coming-from-worldpainter.md) has the whole table.

### The home screen's background

Behind it is the background chosen in **Settings ▸ Appearance ▸ Home background**, one of four styles:

- **Ridges**, the default: ranges of mountains seen from afar, one in front of the other, under a glow of the accent low on the right, made of dithered dots. The ranges drift and breathe; the glow pulses. **Palette** colours the glow and the crests (**Theme**, the accent, by default; **Ember**, **Glacier**, **Dusk**, **Forest**), **Height** sets how tall the ranges rise (60 to 140 %), **Glow** how strong the glow is (0 to 150 %) and **Ranges** how many there are (3 to 8).
- **Waves**: smooth bands of colour rising from the foot and melting into each other. **Palette** (**Theme**, **Flamingo**, the default, **Lagoon**, **Ember**, **Glacier**, **Ink**), **Scale**, **Melt**, **Speed** and **Direction**.
- **Pixel**: the same waves in large square pixels, with a **Pixel size**.
- **Prism**: crisp columns of colour drawing a **Shape** (**Crest**, **Valley**, **Tide**, **Slant**, **Lens**), growing from the side **Direction** says and moving as **Motion** says (**Expand**, **Gather**, **Breathe**), in their own palettes (**Theme**, **Ember**, the default, **Ultraviolet**, **Petrol**, **Citron**, **Blue hour**).

The words over it always stay legible: the colours are held toward the page's own under the text, and over Waves, Pixel and Prism the new map's settings sit on a solid surface. In the light theme the colours soften into pastels; in high contrast they turn to calm greys. The background moves at 12 frames a second at most, only while the home screen is in front; with **Animate the background** off, or Reduce motion on, it stands still.

![Settings ▸ Appearance: the Home background section with Waves chosen and its live picture](images/settings-appearance.png)

## The window

Massif's window has three **sides**, switched at the top left, each with its own workspaces, remembered as you left them:

- **World**: the map editor, where you paint and shape the map. Its workspaces are **Layout** (tools on the left, the map in the middle, 3D view, brushes, terrains, layers, levels and info on the right), **Sculpting** (a big map with the brushes and the 3D view), **Painting** (palettes on both sides of the map), **3D** (the 3D view takes the stage), **3D Sculpt** (the world in 3D with its tools), **Maps** (two maps side by side, to compare display modes), **Minimal** (the tools and the map) and **WorldPainter** (laid out as WorldPainter is).
- **Terrain**: the node-based terrain generator. Its **Generator** workspace holds the four panels of the [Terrain Generator](terrain-graphs.md), Massif's only generator: a map made with the classic graph of earlier versions opens with it converted.
- **Render** (Beta): the map path-traced under a real sky and saved as images, in the **Still** and **Animation** workspaces (see [Render](#render)).

[![Annotated map editor showing a brush footprint, biomes, village and diorama preview.](assets/workspaces/map-editor.webp)](assets/workspaces/map-editor.webp)

The circle marks the brush footprint. The diorama on the right shows the village at block scale. [View full size](assets/workspaces/map-editor.webp).

### Workspaces

Workspaces are the tabs along the top: arrangements of panels. **+** duplicates the one in front; right-click a tab to **reset** it (back to how it came, a copy back to what it was copied from), duplicate it, rename it or delete it. **View ▸ Reset workspaces** puts every one back as it came. The layout is saved and comes back the next time Massif opens.

### Areas and the panel chooser

A workspace is made of **areas**, as in Blender. Each area shows one panel, and any panel can go in any area. The icon before an area's title opens the **panel chooser**: every panel on one page, in columns, with a line saying what the one under the pointer is for and whether another area shows it already. An empty area shows the same page.

| Section | Panels |
|---|---|
| **View** | **Map** (the map from above, to paint and shape), **Diorama** (Beta: the map from above as the game shows it, block by block, and its caves), **3D View** (the whole map in 3D, block by block), **3D Sculpt** (build, dig and smooth in 3D, or walk it as a player) |
| **World** | **Info** (what is under the cursor), **Versions** (the map's save points: restore, open or compare them) |
| **Tools** | **Tools** (every tool, a row for each theme), **Tool Settings** (the options of the tool in hand), **Brushes** (shapes for the brush, and stamps of terrain), **Custom Brushes** (brushes from greyscale images, yours and WorldPainter's), **Brush Settings** (size, intensity, rotation and where the brush paints) |
| **Paint** | **Terrain** (the block on top of the ground), **Layers** (ground covers, objects and terrain mixes), **Packs** (ready-made trees, plants and mixes of blocks), **Blocks** (every block of the game and its mods), **Biomes** (the game's and its mods'), **Levels** (levels that add rock over the ground or carve it away) |
| **Terrain** | **Generator Graph**, **Generator Viewer**, **Node Library**, **Node Properties** (see [the Terrain Generator](terrain-graphs.md)) |
| **Render** | **Render Viewport**, **Sun and Sky**, **Camera**, **Render Output**, **Timeline** |
| **Developer** | **Developer** (frames, memory and processor, live): with Settings ▸ Developer ▸ developer mode on |

![The panel chooser open over an area, every section in columns, the pointer over Layers](images/panel-chooser.png)

### Arranging areas

- The **⋮** menu of an area splits it, joins it with its neighbour, maximises it (Ctrl+Space) or closes it. Hold Shift for a one-click close button.
- Drag any corner of an area into it to split it where you let go: the new area opens on that corner's side.
- Drag an area by its title onto another to move it there (onto an edge: beside it), or onto its centre to swap the two. Drag the gaps between areas to resize them.
- **Float** in the ⋮ menu takes the area out of the layout into a window of its own: move it anywhere, onto another screen, size it by its edges. **Dock back** in its own ⋮ menu, or closing its window, puts it back beside the largest area. Any panel floats, the map, the 3D View and the Render Viewport included; floating panels are saved with their workspace. Where the system gives no windows of their own (some Linux desktops), a floating panel stands over the layout instead: drag its title to move it, its bottom right corner to size it, and drag its title with Shift held onto an area to dock it there.
- Each 3D View area is a view of its own, with its own mode, water and camera: split one to watch the map in two modes side by side.

![The Brushes area dragged by its title over the 3D View, the drop zone highlighted](images/area-drag.png)

### Controls of the panels

A slider is a slim track with its value after it: click the track to jump there, drag the thumb (Shift: finer), drag the value sideways, or click it to type one. A narrow Tools area keeps each row of tools on one line, its name under the pointer.

What Massif is doing in the background (saving, exporting, the automatic tools over a whole map) shows at the bottom right of the window, over the areas, with **Cancel** for long tasks. Messages appear there too, and fade.

### Menus

**Massif**, **File**, **Edit**, **View** and **Help** hold every command,
each item with its shortcut beside it. An item is greyed out when it cannot be used: what
acts on the map waits while the home screen is shown, and everything but
Quit waits while a dialog is open or a map is being opened.

On a Mac they are in the menu bar at the top of the screen, as in every Mac
app, and the window's top bar keeps only its buttons. **About Massif**,
**Settings…** (⌘,) and **Quit Massif** (⌘Q) are in the **Massif** menu,
where Mac apps keep them, and **Help** has the system's search of the menus.
**Help ▸ Join the Discord…** opens [Massif's Discord](https://discord.gg/2dP7xJb655) in your
browser: questions, maps to show, and `#bug-reports`.
⌘Q asks first when the map has changes that are not saved. On Windows and
Linux the menus sit at the top left of the window, with Ctrl for ⌘, and the
first, **Massif**, stands for the Mac's: **About Massif**, **Settings…**
(Ctrl+,), **Check for Updates…** (asks GitHub at once, whatever the daily
check, and opens Settings ▸ Updates with the answer), then **Full Screen**
(F11) and **Quit** (Ctrl+Q), which asks first as ⌘Q does.

On a Mac, quitting from the Dock (**Quit** in Massif's menu there) or from
the app switcher (⌘Tab, then Q) asks as ⌘Q does, Massif coming to the front
for it. So does a logout, a restart or a shut-down, which waits for the
answer: **Save** or **Don't Save**, and it goes on once Massif has quit;
**Cancel**, and it stops, Massif and its map as they were. The Mac waits two
minutes at most before it gives up the logout. With no changes to save,
Massif quits at once.

The menu bar shows the shortcuts of File, Edit, View and Help without taking
their keys: these still go to the window, so ⌘Z undoes in the map, in a text
field or in Edit ▸ Ores as before, and ⌘C, ⌘V and ⌘A work in text fields.

The menus hold every command, each item with its shortcut beside it (⌘ on a Mac is Ctrl on Windows and Linux):

- **Massif** (the Mac's app menu; the first menu on Windows and Linux): **About Massif**, **Settings…** (⌘,), **Check for Updates…**, **Full Screen** (F11), **Quit**.
- **File**: **New World…** (⌘N), **Open…** (⌘O), **Import WorldPainter World…**, **Import Minecraft World…**, **Merge Massif Map…**, **New World from Height Map…**, **New World from a Real Place…**, **Open Recent**; **Save** (⌘S), **Save As…**, **Save Point…**, **World Information…**, **Import Report…** (after importing a WorldPainter world), **Game & mods…**; **Share Image…**, **Export Height Map…**, **Export Region…**, **Export Selection as Schematic…**, **Export to Minecraft…** (⌘E); **Run Script…**; **Import Pack…**, **Export Pack…**.
- **Edit**: **Undo** and **Redo** (naming the step), **Select All**, **Deselect**, **Invert Selection**; **Auto biome: whole map**, **Auto paint: whole map**, **Automatic rules…**, **Smooth terrain…**, **Covers of Layers to Ground…**; **Whole map ▸** (Erosion, Thermal erosion, Rivers, Lakes, Smooth, Sharpen, Terraces, Strata, Scale and raise, then Snowfall… and Caves…), **Replace terrain…**, **Relief of a Real Place…**, **Import Mask…**, **Add Reference Image…**; **Ores…**, **Analyse the map…**.
- **View**: the theme (**Follow the system**, **Light**, **Dark**), **Fit the whole map**, **Show Annotations**, **Reset workspaces**.
- **Help**: **Massif Help** (F1), **Keyboard shortcuts**, **Take the tour**, **Where are my WorldPainter tools?**, **Report a Problem…**, **Record Performance…**, **Join the Discord…**, **Home screen**.

## The map view

The **Map** panel shows the map from above, to paint and shape. Move it with the middle button, Space + drag or the **Pan** tool; zoom with Ctrl + wheel (⌘ + wheel too on a Mac) or a trackpad pinch; **View ▸ Fit the whole map** shows all of it. The wheel alone sizes the brush, as in WorldPainter (see [Controls](#controls) to change that). The footer gives the coordinates and height under the pointer, its region file, and the zoom.

![The Map panel's header: its display modes and overlay buttons](images/map-header.png)

### Display modes and overlays

The header of the map picks what it shows:

- **Terrain**: each column in the colour of its terrain, shaded by the relief.
- **Height**: the heights in a colour scale.
- **Slope**: how steep the ground is, as the brush's slope filter measures it.
- **Biome**: the biome of each column, painted or automatic.
- **Relief**: the land shaded as a relief map.
- **True Colour**: the map as it will look in the game (see [True Colour](#true-colour)).

Picking **Paint biome** shows the Biome mode and **Paint terrain** the Terrain mode, and the view comes back when you change tools; **Settings ▸ Appearance ▸ Switch the map's view with the tool** turns that off.

The 2D map's header picks what it shows (Terrain, Height, Slope, Biome,
Relief, True Colour) and what goes over it: contour lines, **Block steps** and the grid
of chunks (16 blocks), tiles (128 blocks) and region files (512 blocks).
The region files' borders are the strong lines, each region named in its
corner as the game names its file (`r.1.-2.mca`), and the map's footer adds
the region of the column under the pointer: what to know to merge maps
region by region.

The **contour lines** are drawn as WorldPainter's: a solid dark line every 10
blocks, and a finer, fainter one halfway between. The arrow beside their
button (or a right-click on it) sets them: the **Interval** (5, 10, 20, 50, or
any number of blocks), their **Strength**, **Black** or **White** lines (white
reads better on dark ground, the Relief mode's shadows), **Crisp** edges (a
line on the blocks where the height crosses a level) or **Smooth** ones (soft,
along the shape of the ground), and the **Fine lines between**. Zoomed out,
the interval grows to a round multiple of the one set (20, 50, 100…), so that
the lines never run together, and the fine lines show only where there is
room for them. In high contrast the lines are drawn at full strength. The
settings are kept for every map.

With **Block steps** on, the default, the map close up (a pixel or
more for each block) is shaded from the height of each column's top block,
as the game's own maps are: each block is compared with its neighbour on the
side the light comes from, so a step of one block is a sharp line one block
wide, lit where it faces the light and in shadow at its foot, and the contour
lines follow the blocks. Off, the shading follows the smooth height the ground
was shaped to, softer and without the steps. A little further out, where a
pixel spans 2 × 2 or 4 × 4 blocks, the light of the hillsides is the smooth
one and each step still draws its line over it, thinner and fainter the
further out; further still, where a pixel spans 8 blocks or more, both are
the smooth shading.

### Info

The **Info** panel (World) is WorldPainter's: it shows what is under the
cursor, on the 2D map or in the 3D view, and follows it as it moves, in four
parts. **Position**: the column's coordinates, its chunk and its region file,
the levels that have it, and whether the selection takes it. **Ground**: its
height (the block one stands on), the water over it and how deep, its
terrain and its biome (painted, or the one the land calls for on automatic).
**Layers here**: every layer painted there — its ground cover, its tree or
object layer and the one under the trees, its tree size, its underground
pockets, its smoothed steps, the combined layers that may have painted it,
and its annotations — each in its colour, with its intensity where it has
one (the share of the columns a cover grows on, of the squares a tree stands
in, of the rock the pockets take). **Measure**: the results of the
[Measure tool](#measure), while there is a measure.

![The Info panel reading the column under the pointer](images/info-panel.png)

The world's own details — its name, seed, sea level, spawn and block
palette, the map's size, the memory it takes and the map's cache, its
file — are in **File ▸ World Information…**.

**Help ▸ Massif Help** (F1) opens the help: every part of Massif, topic by
topic, with a search, and the keyboard shortcuts.

### Reference images

A reference image is a picture laid over or under the 2D map to trace over,
as WorldPainter's overlay: a plan, a sketch, a real map. It only shows: it
never changes the land and is never exported. **Edit ▸ Add Reference
Image…**, or drop a picture (PNG, JPEG, WebP, BMP, TIFF or GIF) on the map, adds one where it falls; a picture
larger than the map is fitted to it, others take a block a pixel.

![A plan laid over the map at half opacity, its handles out while placing it](images/reference-image.png)

The picture button in the map's header lists them, each with an eye to show
or hide it and a lock to keep it in place; click one for its settings:
**Over the map** or **Under it** (the shaded relief then fades over it, as
much as the image is opaque), **Opacity**, its **Middle**, its **Scale** in
blocks a pixel and its **Rotation**, typed in. The images are passive:
clicks and strokes go to the tool in hand, whatever is under them. With the
**Pan** tool, or after **Edit placement** in its menu (until the **Done** pill
at the top of the map, or `Esc`), drag an image to move it, a corner to scale
it (the opposite corner stays), the knob above it to turn it (Shift: by 15°);
with Pan, dragging the map off the images still moves the view. There can be several. They are
saved with the map by the path of their files, found again beside the map
when the map and its pictures move together. Each is read once and kept on
the graphics card with mipmaps, scaled down past 8,192 pixels a side: a large
picture costs nothing a frame at any zoom.

## The tools

The **Tools** panel holds every tool of the World side, a row for each theme. Hover one for its card: a little animation of what it does, then each gesture and key. The left button uses a tool, the right button does the opposite (lowers, rubs out, takes away). Tool Settings shows the options of the tool in hand, Brush Settings its brush. The keys are those out of the box; **Settings ▸ Keyboard** changes them (see [Controls](#controls)).

| Group | Tool | What it does |
|---|---|---|
| **Sculpt terrain** | **Raise / Lower** (⌥H) | Raise the ground; right button: lower. See [Sculpting the land](#sculpting-the-land). |
| | **Smooth** | Smooth the ground. |
| | **Flatten** | Flatten to the height where the stroke starts. |
| | **Set height** | Set the ground to the target height. |
| | **Roughen** | Roughen the ground; right button: smoother. |
| | **Road** | Click points: a road runs through them, Enter builds it. See [Roads](#roads). |
| **Paint** | **Paint terrain** (⌥P) | Paint a terrain, or several mixed by a noise; right button: back to the map's own terrain. See [Painting terrain](#painting-terrain). |
| | **Gradient brush** | Paint over the edge between two terrains: they blend through the gradient's bands. |
| | **Paint biome** | Paint a biome; right button: back to automatic. |
| | **Auto paint** | Paint what the land calls for: rock on cliffs, sand on shores… See [Automatic biomes and paint](#automatic-biomes-and-paint). |
| | **Auto biome** | Set the biome the land calls for: beach, forest, peaks… |
| | **Annotations** | Mark the map in colour, to plan it: never exported. See [Annotations](#annotations). |
| **Select** | **Rectangle selection**, **Ellipse selection**, **Lasso selection**, **Selection brush** | Mark the part of the map the other tools change. See [Selections](#selections). |
| **Decor** | **Ground cover** | Paint what grows: grass, flowers, snow… See [Ground covers](#ground-covers). |
| | **Objects** | Paint where objects stand: trees, rocks… See [Custom objects](#custom-objects). |
| | **Tree sizes** | Mark where bushes, small, medium or large trees grow. See [Layers](#layers). |
| | **Underground pockets** | Paint pockets of other blocks into the rock under the ground. |
| | **Smoothing** | Paint where Smooth terrain builds the top of the land in partial blocks, or where it does not. |
| | **Combined layer** | Paint several layers at once: a terrain, a biome, a cover, trees, pockets. |
| | **Structures** | Villages and the game's structures, placed with a click. See [Structures and villages](#structures-and-villages). |
| | **Building** | One building of a village placed from a pack: select, move, turn, swap. |
| **Water** | **Water brush** | Lay water under the brush; right button: take it off. See [Water](#water). |
| | **Flood** (⌥F in the WorldPainter keys) | Fill a basin with water; on water, raise it a block; right button: lower it; Shift: drain it. |
| | **River** | Click a source: the water finds its way down. See [Rivers](#rivers). |
| **Underground** | **Layer** | Edit the map layer by layer, cut at a height (Page Up, Page Down). See [Caves](#caves). |
| | **Caves** | Paint where a cave layer carves its caves. |
| | **Cave biome** | Paint lush caves, dripstone caves or the deep dark between two heights. |
| **Levels** | **Stamp slab**, **Erase slab** | Add rock to the active level, or erase it. See [Levels](#levels). |
| **Navigate** | **Pan** | Move the map (also: middle button, Space). |
| | **Eyedropper** (⌥Y) | Pick what is under the cursor to paint with it. See [The eyedropper](#the-eyedropper). |
| | **Measure** | Measure a line's length, heights and slopes, or an area's size, volume and cover. See [Measure](#measure). |
| | **Spawn point** | Set where players appear in the world. See [The spawn point](#the-spawn-point). |

The tools new in this version are marked in the panel until you first try them. In the **3D Sculpt** panel the same tools work in 3D, with three of its own: **Sculpt**, **Dig** and **Smooth** (see [3D Sculpt](#3d-sculpt)).

![The Tools panel, the pointer over Gradient brush and its card](images/tools-panel.png)

### Cards

Hover a tool and its card shows: a little looping animation of the tool at
work on a patch of land drawn like the map — a hill rising under the brush,
flowers spreading, trees growing where they are painted, a river laid out
point by point and dug, water rising in a basin — then what it does, what
each gesture and key does with it, and a remark. A brush's card shows the
land it raises, shaded and contoured on a block of ground, turning slowly
from side to side, so that its shape is easy to make out. (As in the user's
WorldPainter fork, whose cards these are.)

## Brushes

Brushes work as in WorldPainter. A brush has a shape (one of the twelve
built-in brushes, a terrain stamp, or an image), a radius, an intensity and
a rotation:

![Brush Settings with Height limits on: the side cut of the map showing the band the brush acts on](images/brushes.png)

- **Built-in brushes**: circle or square with a Constant, Linear, Sine,
  Plateau, Spike or Dome profile.
- **Terrain stamps**: generated height-map brushes for landforms; use them
  with Raise to stamp a peak, with Lower to carve a valley.
- **Custom brushes**: greyscale images (white paints, black does not),
  stretched over the brush. Massif reads its own `brushes` folder (every
  sub-folder is a group) and WorldPainter's brushes folders, in place.

Brush operations run tile by tile in parallel, so even a brush 8,000 blocks
across stays responsive and light on memory.

The outline on the map is exact: zoomed in, it follows the blocks the brush
will change, block by block; zoomed out, a custom brush is outlined through
every pixel of its image. Inside it, the brush shows how hard it presses:
the map is lighter where it presses more, with a line where its strength
is a quarter, a half and three quarters — the rings of a soft round brush,
the peaks of a terrain stamp.

**Brush options** (in Brush Settings) limit where every brush acts, from
what the map shows:

- **Height limits**: at or above, at or below, between or beyond two
  heights, optionally fading over 4 blocks. A picture shows the map from
  the side, cut from west to east through its highest point, with the sea
  in blue and the heights on a scale beside it: the land at the heights the
  brush acts on, between the limits' lines, is in the accent. Each limit is
  a line across, dragged up or down (Shift: finely) or typed in. Under it,
  the share of the map the brush acts on.
- **Only on / except on**: terrains, biomes, water or dry land, and layers:
  ground covers (ferns, tall grass, Frost…), tree and object layers, the
  layers under the trees, underground pockets and annotations. As in
  WorldPainter, **only on** asks for every kind listed at once: a ground
  cover and a terrain (meadow fescue and dirt) means that cover on that
  terrain, nowhere else; two of one kind that cannot share a column (two
  terrains, two biomes, two ground covers) mean either. **Except on** keeps
  off any of what it lists. **Add**
  opens a list to search, by name or kind, sorted three ways: **On the
  map** (what the map holds first, then the rest), **Recent** (what the
  filters were set to lately first) or **A–Z**. The **eyedropper** beside
  it picks on the map, as WorldPainter's "select on map": click it, then a
  spot of the 2D map or the 3D view; what is there — its terrain, biome,
  water or land and each layer — shows by the pointer, and the one clicked
  becomes the filter. It is one shot: the brush is back in hand after, and
  Esc puts the eyedropper down.
- **Slope**: above or below an angle. Slopes are the trend of the hillside
  (a plane fitted over 9 × 9 columns), not the steps between single blocks;
  the map and 3D views' Slope mode use the same measure (in 3D, a column's
  walls take its colour too, as do they in Height and Biome).
- **Facing**: only the slopes turned a given way, give or take an angle —
  the side the sun lights, the side in the shade. A compass sets the
  direction (north up, as on the map); **Sun** and **Shade** take it from
  the light of the map. Level ground faces nowhere. Snow on the northern
  slopes, moss in the shade, bleached rock in the sun: one stroke each.
- **Curvature**: hollows or bumps, deeper than a number of blocks compared
  with the columns around.

The options are read by squares of 4 × 4 tiles in parallel, each with the
ring of columns around it its tests need, so that a brush of any size takes
little memory; slopes are fitted from sums along the rows, then down the
columns, a few additions a column rather than the 81 of the square.

### Pencil

**Pencil** (Brush Settings) draws as WorldPainter's Pencil: a hard line, at
full strength, that follows the mouse block after block, without a gap
however fast it moves, with whatever tool is in hand — terrain, a layer, a
biome, the height. One block wide, its blocks join by their sides, never
only by a corner (a "joined up" line, as roads and walls want it);
**Width** makes it 3, 5 or more blocks wide.

The wheel sizes the brush; **Alt** and the wheel turn it, as in
WorldPainter (⌘ and the wheel too on a Mac, Ctrl + Shift elsewhere), 5° a
notch, 1° with Shift; Ctrl and the wheel zoom the map.

## Selections

As in WorldPainter and image editors, a selection marks the part of the map
the tools change. The **Select** row of the Tools panel holds four tools:

![A lasso selection drawn round a hill, the Select tools' settings beside the map](images/select-tools.png)

- **Rectangle selection** and **Ellipse selection**: drag across the map.
- **Lasso selection**: draw around an area; letting go closes it.
- **Selection brush**: paint the selection with the current brush; the right
  button (or Alt) takes out of it. The brush's soft edge feathers what it
  paints.

Shift or ⌘ (Ctrl on Windows and Linux) while dragging adds the shape to the
selection, Alt or the right button takes it out, both keep only what the two
hold. With no key, the shape does what the **New · Add · Subtract ·
Intersect** buttons of Tool Settings say (New by default). To clear the
selection, right-click the map, press Esc, or press **Clear** beside those
buttons (⌘D too); in the New mode a click without dragging clears it as well.
Esc first drops a shape being drawn. The keys that add and take out can be
changed in Settings ▸ Keyboard.

Once something is selected, everything that edits the map keeps to it:
every brush (sculpting, painting, covers, objects, water, levels, the 3D tools
used on the map), the Flood fill (it stops at the selection's edge), **Auto
biome** and **Auto paint** over the whole map, noise painting over the whole
map, and **Edit ▸ Whole map** (erosion, rivers, terraces…). Nothing changes
outside it; a stroke wholly outside does nothing at all, and a short word at
the foot of the window says so once a stroke, with how to act everywhere
again (Esc with a Select tool, or Edit ▸ Deselect).

**Feathering.** A selection weighs every column from fully in to fully out.
**Feather** in Tool Settings fades new shapes over that many blocks on either
side of their edge (up to 64), and **Feather it** softens the selection there
is. A brush acts on a feathered edge as much as its weight says: raising by 10
blocks raises the middle of the edge by about 5. Paint that sets a value (a
terrain, a biome) sets it on that share of the columns, scattered.

**Select by** picks columns over the whole map, as the brush filters read
them: a terrain (the one picked in Terrain or Blocks), a band of heights, or
slopes steeper or gentler than an angle. It combines with the selection as the
mode buttons say, so "the grass between 70 and 90" is a Terrain selection,
then a Height one with Intersect.

**All**, **Invert** and **Deselect** are in Tool Settings and in the Edit menu
(⌘A, ⇧⌘I, ⌘D; Ctrl on Windows and Linux). Every change of the selection is a
step to undo.

The map shows the selection as a dashed outline whose dashes march; with
Settings ▸ Accessibility ▸ Reduce motion they stand still. The selection is
saved in the `.massif` file, in a section older versions skip. **Export**
offers **Only the selection** when there is one: only the tiles of
128 × 128 blocks the selection reaches are written, and players appear inside
it, a quick way to try a part of a large map in the game.

## Sculpting the land

The **Sculpt terrain** group shapes the height of the ground, with the brush picked in **Brushes** and set in **Brush Settings**:

- **Raise / Lower**: the left button raises the ground under the brush, the right button lowers it, by the brush's intensity and shape. With a terrain stamp from the Brushes panel, one click stamps a peak (or carves a valley with the right button).
- **Smooth**: evens out the ground, block by block. Its strength and how far it reaches are in Tool Settings.
- **Flatten**: levels the land to the height where the stroke starts (below).
- **Set height**: sets the ground to the **Target height** of Tool Settings (the eyedropper beside it takes the height under the cursor).
- **Roughen**: roughens the ground; the right button smooths it.
- **Road**: a road graded into the land (see [Roads](#roads)).

Each stroke is one step to undo. Brushes run tile by tile in parallel, so a brush thousands of blocks across stays smooth. The same tools sculpt in 3D in the 3D Sculpt panel, where Raise / Lower pushes the surface along its slope into overhangs (see [3D Sculpt](#3d-sculpt)).

[![Raise / Lower with a custom brush outline on the terrain and its height map visible at right.](assets/workspaces/raise-lower.webp)](assets/workspaces/raise-lower.webp)

With **Raise / Lower** selected, the white outline shows the brush footprint. Hold the left mouse button to raise the ground or the right button to lower it.

### Flatten

Flatten levels the land to the height where the stroke starts, as in
WorldPainter. Its options in Tool Settings:

- **Both ways** (the default) raises what is under the height and lowers
  what is over it.
- **Only raise** fills the hollows up to the height and leaves the land
  over it as it is; **Only lower** cuts the land over the height down and
  leaves the hollows.
- **Apply theme** repaints the land it flattens with the terrain its new
  height calls for (sand by the sea, grass, rock and snow higher up), as
  Paint Terrain with Alt does.

## Painting terrain

A **terrain** is what the top of the ground is made of: grass, sand, stone, a mix of blocks, or any block of the game or a mod. **Paint terrain** paints one. Tool Settings sets how:

- **One terrain**: the one shown at the top of its settings, picked there, in the **Terrain** panel, in the **Blocks** panel, or with Alt-click on the map.
- **Noise**: several terrains or blocks at once, mixed by a noise (see [Noise painting](#noise-painting)).
- **Zones**: each column takes the terrain its height, slope and water call for (below).
- **Gradient**: terrains blended across a boundary (see the Gradient brush below).

The right button takes the ground back to the map's own terrain. **Paint biome** paints a biome from the **Biomes** panel the same way, its right button back to automatic.

![The Painting workspace with Paint terrain in hand](images/painting-workspace.png)

### Picking the terrain

The terrain Paint terrain lays is shown at the top of its Tool Settings,
with its picture: a click on it opens a list to search, the terrains picked
lately first, then the terrains of the map, then every whole block of the
game (a block becomes a terrain of the map the first time it is picked).
The Terrain and Blocks panels pick it too, and Alt-click picks the one under
the cursor on the map.

**The Terrain panel's categories are yours.** Its terrains are listed by
category (Grass & Soil, Stone, a WorldPainter world's Imported…), each
folding at a click on its title. Drag a terrain onto another category, or
right-click it ▸ **Move to**, to sort it there, or into a **New category…**
named at once. Right-click a category to rename it (or double-click its
title), move it up or down, hide its terrains, or delete it (its terrains
go to Other; none is removed). **Hide from the list** takes a terrain out of
the lists without touching the map: it is listed under **Hidden**, at the
end, where **Show in the list** brings it back. The map keeps its
categories, their order and the hidden terrains.

**Blocks of your game and mods, as terrains.** Under the map's terrains,
**From your game and mods** lists the blocks that make ground, by source and
category (Minecraft · Stone, Conquest Reforged · Soil, sand and snow…), each
group folded at first: whole opaque cubes of stone, soil, wood or building,
not glass, lamps, furniture, doors, slabs or plants. Click one to paint with
it: it becomes a terrain of the map, in its group's category, and is listed
among the terrains from then on (a block already a terrain of the map is
not listed twice). **Every block** lists the others too, and a search looks
through every block. When the mods are read, a WorldPainter world's
**Imported** terrains are sorted in one click (**Sort … Imported terrains by
mod and kind**, at the top of the panel) into the categories of their
blocks; those whose block is not read stay in Imported. The Blocks panel
still lists every block, for those who want the whole catalogue.

**A WorldPainter world made of a mod's blocks.** The import report names
the mods whose blocks the world is made of and that are not read yet, and
offers to read them first (from an instance found on this computer, or
**Game & mods…**): their real colours, the sorting of the Imported terrains
and the covers of layers made the ground's own as the mod's blocks.

### Unpainting

Paint terrain's right button takes the land back to the map's own terrain:
the one a new map was filled with (**One terrain** in the new map's
settings), or, on a map made before Massif kept it, the one covering nearly
all of its dry land. Tool Settings ▸ **Right button** shows it; a click on
it opens the list of terrains and blocks to choose another, kept with the
map. On a map whose terrains came from its height, or with **By height**,
the right button puts back what the map's zones call for (see
[Zones](#zones)). In 3D Sculpt, Paint's inverse does the same.

### When the paint does not show

Some parts of the map may keep their look under a stroke. Once the stroke
ends, a word at the foot of the window says how much of it shows no change,
and why:

- **A ground cover lies over the terrain.** A cover that lays blocks over
  the ground (sand, gravel, layers: a WorldPainter map's custom ground
  covers come in as such) stays on top of whatever terrain is painted under
  it, on the map and in the game. The terrain is painted all the same.
  Rub the cover out with **Ground cover**'s right button, or hide it in the
  Layers panel to see the terrain under it.
- **A level stands over the one painted.** Pick that level in the Levels
  panel to paint it.
- **A layer is locked.** Ground cover and Objects leave a locked layer's
  blocks as they are: unlock it in the Layers panel.
- **The brush's filter** (Only on, Except on, the heights, the slopes)
  leaves the columns it does not let through.
- **The layer is hidden** on the map: it is painted, and shows once shown
  again.

The Info panel says the same of the column under the cursor, for the tool
in hand.

### Replace terrain

**Edit ▸ Replace terrain…** paints every column of one terrain with
another, over the whole map, or only in the selection when there is one
(by its weight). Choose the terrain to replace and the one it becomes, each
in a list by category with a search; **Replace** runs over the map and is
one step to undo. It is how two terrains made of the same block (a
WorldPainter world's Imported one and a mod's own) become one: the terrain
replaced stays among the map's terrains, to hide from the Terrain panel's
lists. Right-click a terrain in the Terrain panel ▸ **Replace it…** opens
the same window with that terrain chosen.

### Zones

Paint terrain's **Zones** (among **One terrain**, **Noise**, **Zones**,
**Gradient**) paints each column with the terrain its zone calls for: by the
height of its top block, its slope and the water over it. The right button
with **By height** puts back the same. The zones show under **Zones** in
Tool Settings, each with where it goes (`y 140–200 · under 50°`, `under
water, 4–6 deep`), read from the top: the first zone that takes a column
paints it, and a column no zone takes keeps its terrain. **Ragged edges**
breaks up the lines between zones, a few blocks up and down.

Presets to start from:

- **Classic**: as a new map is painted: grass, sand along the water, rock on
  cliffs, rocky ground then snow up high.
- **Plains**: grass, coarse dirt on the steeper slopes, rock on cliffs.
- **Mountain**: grass below, gravel on the steep slopes, stone on cliffs and
  above the tree line, snow on the peaks.
- **Mesa**: red sand on the flat, bands of terracotta of every colour up the
  slopes, coarse dirt on high plateaus.
- **Desert**: sand, sandstone on the slopes and under the water.
- **Taiga**: podzol, gravel along the water, coarse dirt on slopes, stone on
  cliffs, snow up high.
- **Coast**: sand along the water, gravel and stone on the sea cliffs, grass
  inland.

**Edit these zones** makes the preset the map's own (**Custom**): click a zone
to change its terrain (the same list as above), whether it is on dry land or
under water, its heights (Y on the map) or its depth under the water, and
its slope in degrees; move it up or down, take it out, or **Add a zone**.
The zones are saved with the map; the disk button saves them under a name in
the settings, to paint other maps with (**Saved**).

Under the water, nothing that only grows in the open air is ever painted —
grass, podzol, mycelium, moss, snow — whatever a zone says: the zones made
for under the water take it, or else the game's beds by depth: sand, then
clay, dirt, and gravel deep down. A lake above the sea is under water too.
Under a sea of lava, blackstone and basalt. (The export already lays dirt
rather than grass, podzol or mycelium wherever water stands on them.)

### Gradient brush

![The Gradient brush's editor on the gradient of the terraformers' sketch: moss, coarse dirt, podzol and grass on a bar with their textures, the stops and midpoints under it, the edge's marker over it, a blobby transition chosen, a patch and a strip showing the result](assets/renders/gradient-editor.jpg)

![The Gradient brush's Tool Settings beside the map](images/gradient-brush.png)

The **Gradient brush** (in the Paint tools, beside Paint terrain: a swatch
fading out, marked with a dot until it is first tried; or **Gradient** at the
top of Paint terrain's Tool Settings, beside **One terrain** and **Noise**)
blends two terrains over their edge instead of leaving a hard line. Paint the base
terrains first — patches of rock in grass, a dark pit — then drag the
Gradient brush over the boundary: rock, then a rim of other blocks, then
grass.

A gradient is a list of terrains in **bands** of so many blocks, A → B → C →
D, and between each two a **transition**:

- **Hard cut**: a clean line.
- **Noise**: a ragged edge, each terrain reaching into the other.
  **Intensity** sets how far, **Scale** the size of its fingers.
- **Blobs**: blobs of the next terrain, of every size, denser and denser until
  they join, with blobs of the first left in it. **Density** sets how many,
  **Blob size** the size of the largest.

Each transition also has a **midpoint**, as between two stops of a picture
editor's gradient: where the two terrains are half and half. Moved towards
the first terrain, the next one goes further into it; towards the next, the
first goes further out. A band can be any terrain of the map, a terrain mix,
or any block of the game or of the mods read in Blocks (Conquest Reforged's,
say): a block becomes a terrain the first time it is picked.

**Across an edge**, the brush's main way, every column takes the band its
distance to the boundary says. The bands before the **edge** lie on the side
of the gradient's first terrain, those after it on the side of its last; out
of the box the edge falls in the first transition, so the first terrain
keeps its ground and the others ring it. The boundary is found within the
brush's reach, between the gradient's first terrain and its last, as it was
before the brush painted there, and only those two terrains are painted:
its middle terrains are the transition it laid, and a terrain it does not
paint (a beach, a field) stays as it is, as does the ground under water
(the gradient stops at the water's edge). Clicking again over the same edge
paints the same thing: the brush remembers the boundary it found until the
ground is changed by something else, and a soft brush only fills in the rim
its first click left half painted. Past the gradient's ends the ground is
left as it is. **Wander** widens and narrows the bands along the edge, so
that a rim is wide here and thin there as on real ground; the edge itself
stays where it is. With a selection, the gradient runs in from the
selection's edge instead. **From the centre** lays it from the brush's centre
(its start) to its rim (its end): a patch with a core and rings. The right
button paints the gradient the other way round.

![A dark pit in grass, then ringed by andesite and gravel fading into the grass after one click of the Gradient brush](assets/renders/gradient-patch.jpg)

It keeps to the selection, the brush's filters, its intensity and soft edges,
and a stroke is one step to undo. The gradient is laid in world coordinates
from the map's seed and its own: the same stroke paints the same ground, and
strokes side by side join without a seam. A brush thousands of blocks wide
is worked out a square of 1,024 blocks at a time.

**Edit…** opens the gradient's editor. **Presets** along its top are the
built-in gradients, yours and the map's: a click starts from one. The bar
shows the bands and transitions with the blocks' real textures and their
widths in blocks. Drag a stop under the bar to move blocks from one segment
to its neighbour; click under the bar between two stops to add one there,
splitting the band; drag a transition's diamond to move its midpoint, and
the marker over the bar to move the edge. Click a segment to choose its
terrain, type its width or pick its transition and settings. A patch and a
strip along an edge show the result as it changes. **Save** keeps the
gradient among yours (in every map) and in the map, which keeps every
gradient it is painted with. **Export…** writes it as a `.massifpack` with the
terrains it needs; **Import…** reads one. Built-in gradients are a start:
Rock to grass (a gold-green rim of moss around rock), Pit to grass (two
greys around a dark pit), Deciduous forest to grass (a hard cut, a noise and
blobs over 30 blocks) and Shore. Gradients travel in packs too: **File ▸
Export Pack…** offers the map's gradients, and one picked in the Packs panel
adds its terrains to the map and paints with it.

### The eyedropper

The **Eyedropper** (⌥Y, in the Navigate group) picks what is under the cursor to paint with it, as WorldPainter's: a click lists, by the pointer, the terrain, the biome and each layer there; the one picked is put in hand with its tool (where there is only a terrain, it is picked at once). **Shift + click** takes the terrain, the height and the water at once, and goes back to the tool before. It works in the 3D views too. **Alt + click** with Paint terrain picks the terrain under the cursor without changing tools, and the small eyedropper beside a brush filter or a height field picks for that field alone.

### Measure

The **Measure** tool (in the Navigate group) measures the map, a block
being a metre as in the game. Choose **Line** or **Area** in Tool Settings.

- **Line**: drag across the map, or click from point to point and
  double-click (or press Enter) to end; the pointer is the next point
  meanwhile, and Backspace takes back the last one. A label on the line
  says its length, its height difference and its mean slope. The Info panel
  gives its **Distance** on the map, its length **Along the ground** (up
  and down the slopes), its **Height** at each end, the blocks climbed and
  come down along a path of points, its **Mean slope** and its **Steepest**
  (over 4 blocks), as a grade and in degrees, and the **profile** of the
  land under it, the water shown over the ground: point at the profile and
  the map marks the spot.
- **Area**: drag around a part of the map (a lasso), or click its corners
  and click the first one again (or press Enter) to close it. The Info
  panel gives its **Area** (in m², km² past a million), its **Perimeter**,
  its lowest, highest and mean heights, the blocks of ground **over** a
  level and the room **under** it — what levelling it would take away or
  bring — counted from its **Lowest ground**, the **Sea level** or **A
  height** of your choice (Tool Settings ▸ Volume counted from), the share
  of it under water, and what covers it: its terrains, its biomes and its
  layers, each with its share. An area of millions of blocks is read one
  column in several, and says so.

Measures work in the 3D views too: each click adds a point on the land, a
double-click or Enter ends it. A measure is never saved with the map. It
shows while the tool is in hand; **Keep on screen** leaves it on the map and
in the Info panel with the other tools. **Esc**, a right-click or **Clear**
clears it.

### The spawn point

The **Spawn point** tool (in the Navigate group), as WorldPainter's Spawn tool, sets where players appear the first time they join the world. Click the 2D map or a 3D view: the spawn point goes on the ground of that column, or on the water over it, as one step to undo. A pin marks it on the map and in the 3D views (in the accent colour while the tool is in hand); drag it on the map to move it. With the Pan tool or this one, a right-click on the map opens a menu with **Set spawn here**. The spawn point is in the Overworld; **File ▸ World Information…** shows and sets its numbers, and the export's **Spawn point** can still set another for one export.

## Noise painting

Paint terrain can lay several terrains or blocks at once, mixed by a noise
(Tool Settings ▸ **Noise**), as the noise painters of Axiom and other
editors do: grass with patches of coarse dirt and podzol, stone veined with
gravel, rooted dirt in andesite, a patchwork of fields. The noise has a
**shape** — **patches**, **veins**, **blobs**, **cells** or wavering
**stripes** —, a kind (**simplex**, **Perlin** or **value** noise), a
**size**, **octaves**, **lacunarity** and **gain** for its detail, a
**stretch** and a **direction** to draw it out into streaks or bands, soft or
sharp edges and a seed.

![Paint terrain in Noise mode: three terrains and their shares](images/noise-painting.png)

The blocks are a list, in order — neighbours in the list meet on the ground,
as in a gradient —, each with its **share of the ground**: the shares always
make 100 %, and every block covers its share whatever the noise (the pattern
learns its own spread and paints by rank). Drag a block's icon to move it in
the list; click its name to put another block in its place; **Add a block**
offers the map's terrains and every whole block of the installed game (a
block becomes a terrain the first time). Pictures show the ground it paints,
its noise, and how the noise's values spread between the blocks; hovered over
the map, the brush shows what it would paint before the first stroke.
Presets (Meadow, Forest floor, Rocky ground, Gravel paths, Snowy rocks,
Dunes, Fields) are a start, and any pattern can be saved under a name.

It paints with the brush of every tool — its shape, size (up to a radius of
4,096 blocks: type it in Brush Settings, or scroll), intensity and options —,
and **Paint the whole map** lays it over every column of the active level in
the background, where the brush options let it (only on grass, a height
band, a slope…), as one step to undo. The pattern is laid in world
coordinates: strokes side by side join without a seam, and painting a place
again gives the same blocks.

**Blend** makes the list an ordered gradient: neighbours in the list are
dithered into each other, block by block, across their shares. At 0 the
blocks meet in sharp bands (with the ragged edge **Soft edges** gives); at
100 % each block fades into the next over the whole of their shares, so that
dirt 30 %, coarse dirt 30 % and grass 40 % run from dirt through coarse dirt
to grass with no hard line, and never dirt beside grass. The dither is
seeded by the world position too: strokes still join.

The same mix is a node of the terrain graph, **Noise paint** (an output): its
noise, its blocks in order with their shares, and the presets — those built
in and those saved from the brush or from the node, one list for both, so a
mix made with the brush is laid by a graph and the other way round. Its mask
input says where it paints (dithered where the mask is between 0 and 1);
without one it paints everywhere. Outputs below it in the graph paint over
it.

## Automatic biomes and paint

Two tools of the Paint group work from what the map already is, with a
brush (and its options) or over the whole map at once:

![Auto biome in hand over the map](images/auto-biome.png)

- **Auto biome** reads the land and gives it biomes. Every column is one
  of twenty kinds of land, told from its water (deep, shallow, or with a
  bank close on both sides: a river), its height over the sea (lowland,
  hills, mountains, peaks), its slope (cliffs are rock, whatever grows on
  them) and what its ground is made of (grass, sand, red sand, snow, moss,
  mud, mycelium, podzol, stone); sand at the water's edge is a beach, and
  sand away from it a desert only where sand is all around (seven tenths of
  the dry ground within 48 blocks): a patch of sand in a plain is the
  plain. Each kind has a biome. A map filled with one terrain other than
  grass (a WorldPainter world all of sand, for instance) is read from the
  shape of its land there: its fill says nothing about it.
- **Auto paint** goes the other way: every biome has a terrain for its
  ground, one for its cliffs and one for what lies under its water. Where
  no biome is set, the ground is painted as the land it is.

Over the whole map (**Whole map** in Tool Settings, or Edit ▸ Auto biome /
Auto paint: whole map) they run in the background, a progress bar and a
Cancel button in front of the map, and their result is one undoable step;
the map is read by squares of 4 × 4 tiles and never decoded whole (a map
of 44,544 × 33,536 blocks takes about 15 seconds). Hovered over the map,
both show what they would change before anything is painted: under the
brush, the columns that would change, in their new colour, and those that
already have what the land calls for (painted by an earlier stroke, or so
from the start) hatched, so that the whole brush shows; a bar under the map
names what the columns become, with their colours, and how much of the
brush stays as it is. Biomes do not show on the map in its Terrain mode:
its Biome mode shows them. A stroke changes every column in the core of the brush
(all of it at 100% intensity, with a blended edge below), and says how many
it changed — or that the land already follows the rules.

What the land calls for is worked out a tile at a time, by squares of
tiles each read with its margin, and kept: as long as the map does not
change, the preview following the pointer and the dabs of a stroke read
each tile once (a stroke of Auto biome hands what it read back to the
preview), and a brush of any size takes little memory. The preview is
drawn a column a pixel. With a brush 4,000 blocks across on a 5,888-block
WorldPainter map, the pointer moves at 30 ms a frame and a stroke at about
100; at 300 blocks, both under 10 ms.

### Biomes of mods and data packs

Once a mod's blocks are read (Blocks panel), Massif reads the biomes the
game, its mods and its data packs define too
(`data/<namespace>/worldgen/biome/*.json`): the mods' jars, the data packs
added with **Add mods…** (a `.zip`, or a pack left as a folder), and those
an instance gives every world (its `datapacks`, `global_packs` and
`openloader` folders), as well as mods that add biomes and no block. The
**Biomes** panel lists them after the game's, under each mod's name with
how many it has; its search finds a biome by its name, its id or its
mod's name. A biome is added to the map when it is first picked, and saved
with it; the game's biomes the built-in list lacked (its Nether's and End's,
Ice Spikes, Eroded Badlands…) come the same way once the game is read.

Each has a colour on the map: its grass's (a sea's, river's or lake's
water), mixed with a colour of its own drawn from its id so that biomes of
the same grass stay apart. In the 3D view, grass and leaves take the
colours the biome gives them in the game: its own `grass_color` and
`foliage_color`, else the game's colour maps at its temperature and
rainfall, a swamp's or a dark forest's grass as the game changes it.

**Any biome by its id.** Type `namespace:biome` in the panel's search
(`terralith:moonlight_grove`), then **Use** it: for a mod or a data pack
that is not read here. It is named after its id and given a colour of its
own; the export writes it as typed.

The export writes every biome by its id into the chunks' biome palettes:
open the world with the mod or data pack that defines them installed. Auto biome's **Rules…** offers them
for each kind of land, and a terrain whose mod's profile names a biome for
its block (see *Mod profiles*) takes that biome where the biome is left on
automatic. Before 1.18, when biomes were numbers, a mod's biome is written
as plains.

**Rules…** (in Tool Settings, or Edit ▸ Automatic rules) opens both tables:
the depths, heights and slopes the land is read with, the biome of each
kind of land, the three terrains of each biome. They are kept with the
settings. Forests are told from the ground for now (podzol, coarse dirt);
trees will count when layers exist.

## Layers

What WorldPainter calls layers — its custom plant layers, custom object
layers and custom terrain mixes — are in Massif the **ground covers**, the
**object layers** and the **terrain mixes**. The **Layers** panel lists all
of the map's in one place: the **combined layers** first, then the others
by the slot of a block they paint — terrain mixes, ground covers, trees
and objects, under the trees, underground pockets, tree sizes,
annotations (it sits under the Terrain panel; any area can show it, from
Paint in the area's chooser). How they fit together is in
[Layers](layers.md).

![The Layers panel listing the map's layers](images/layers-panel.png)

- a **click** picks one to paint with, with its tool — Ground cover,
  Objects, or Paint terrain with its noise — and Tool Settings shows its
  settings, to edit;
- a **double-click** renames it;
- its **eye** hides it on the map and in the 3D view, to see the ground
  under it; the export writes it all the same, and the map remembers it;
- its **lock** (on the right of the row it is pointed at, and in its menu)
  keeps a ground cover's, a tree layer's or a pocket layer's blocks: no brush paints another
  layer of its slot over them, nor rubs them out, until it is unlocked;
- its menu (right button) opens its **Settings…** in a dialog, paints with
  it, **finds** it — the map goes to the nearest place it is painted, the 3D
  view with it when it follows the map, and **Find** again goes to the next
  place, round to the first past the last (each patch of painted tiles is
  one place; what each tile holds is kept from one Find to the next, a
  tile read again only once it changed, so it is quick on the largest
  maps) —, renames, **duplicates**, moves it **up** or **down** its section,
  locks or **removes** it;
- **New combined**, **New cover**, **Import objects…**, **New pockets**
  and **New mix** add one. A mix is
  set up in Paint terrain ▸ Noise and saved there under a name.
- under them, the fifteen colours of the **annotations**: a click paints
  with that colour.

A column has at most one cover and one object layer; painting another
over it takes the place of the first.

**Frost and Lava** are covers that come with every new map, as WorldPainter
has them. **Frost** lays snow on the ground and turns the top of the water
where it is painted to ice; **Lava** turns the water where it is painted
into lava (a lava lake in a world of water). Any cover can do either: **The
water** in its settings is **As it is**, **Frozen** or **Lava**. A map made
before has them in Packs ▸ Plants & ground cover and Water plants. The map
shows the ice white and the lava orange, and the 3D view the same.

**Annotations** (the **Annotations** tool, in the Paint group) mark the map
in colour, as WorldPainter's do — where a town goes, a road to build, a river
to dig — to plan a map. They are drawn over the map in every view mode,
whatever the level being edited, and on the land of the 3D views, and never
exported. The right button rubs them out.

- **Colours**: WorldPainter's fifteen, and any other: the **Colour** button
  of the tool's settings opens a colour picker. Picked from one of the
  fifteen, it adds a colour of the map's own (kept with the map, up to 240 of
  them, shown after the fifteen); picked from one of the map's own, it
  changes it, and the marks painted with it change too.
- **Opacity**: each colour has its own (75 % unless set otherwise), in the
  tool's settings.
- **Show or hide them all**, on the map and in 3D alike: the eye in the
  tool's settings and beside the colours of the Layers panel, the pen button
  in the map's header, or **View ▸ Show Annotations**.
- **Curves**: **Curve** at the top of the tool's settings, in place of
  **Brush**, draws annotations as curves, as Axiom's Path tool: click to
  place points, drag a point to move it, drag the curve to add a point
  there, right-click a point (or Delete, for the one picked) to take it
  off. It runs **Smooth** (a Catmull-Rom spline through every point),
  **Bézier** (the point picked shows two handles, mirrored; drag one to bend
  the curve there) or **Straight** (from point to point). It is drawn as the
  ribbon it will paint, from its **Start width** to its **End width**, in the
  colour picked. It stays a curve to edit until **Paint curve** (Enter)
  paints it, in one step to undo; from then on it is an annotation like any
  other, rubbed out the same way. Escape drops it.

The curve is a shape of its own in Massif's core (`massif_core::curve`), apart
from the annotations: the River and Road tools are meant to follow one later,
a river or a road laid along a curve drawn first.

### Base layers, sub-canopy and tree sizes

How all the layers fit together — kinds, the slots of a block, masks, order,
visibility and lock — is in [Layers](layers.md).

A block has one **base** plant layer (its ground cover) and one **base**
tree layer (its object layer): painting another over it takes the place of
the first, so two plants or two species of trees never grow on one block.
WorldPainter lets layers pile up on a block; Massif keeps them apart by
itself.

An object layer can instead be a **sub-canopy** layer (**Canopy ▸
Sub-canopy** in its settings): it is painted beside the base tree layer of a
block, not in its place, and its objects grow under that layer's trees —
hazel, holly, dogwood, bushes. A block has one sub-canopy layer too. In the
export and the 3D view, the base layer's trees stand first and the
sub-canopy's grow only in the air they leave (and in water and plants):
never through a trunk or a crown. The **Bushes** and **Azalea bushes** of
Packs ▸ Trees are sub-canopy layers. Turning a layer sub-canopy, or back to
base, takes what is painted of it along, as one step to undo. In the Layers
panel, a sub-canopy layer says so beside its count of objects.

**Tree sizes** (the **Tree sizes** tool, in the Decor group, and the four
swatches under the object layers in the Layers panel) mark where
**bushes**, **small trees**, **medium trees** and **large trees** grow. Paint
them first, then paint one species over everything — Big oaks, say — and
each spot takes that species' objects of the size marked: its smallest
where Bushes is painted, its tallest where Large trees is. A layer's objects
are split by their height into the four sizes, a quarter of them each (the
sixteen kinds of trees of Packs come in eight sizes, two to each); where a
layer has none of the size asked for, the nearest one it has stands. The
settings of an object layer show the size of each of its objects. The sizes
show over the map, in four greens, while the tool is in hand, or always with
the eye of their section in the Layers panel; the right button paints any
size again.

### Combined layers

A **combined layer** (the **Combined layer** tool, in the Decor group, and
the first section of the Layers panel) paints several layers at once, as
WorldPainter's combined layers do: an oak forest's grass, its forest
biome, its ferns and its oaks in one stroke. Its settings name the layer
of each slot it paints — **Terrain**, **Biome**, **Cover**, **Trees**,
**Sub-canopy**, **Tree size** and **Pockets**, or **None** — and the share
of the columns painted each takes: ferns over 60 % of them, say, the same
columns every time the brush goes over them. Each layer goes to its slot
as its own tool paints it, locks included, so a block still holds one
layer of each slot; changing a combined layer changes what it paints
next, not what it painted. One stroke is one step to undo. The right
button rubs out what it paints: its cover, trees, size and pockets gone,
the biome back to automatic, the terrain put back as Paint terrain's
right button puts it. Its pockets go into the rock of the ground, whatever
level is edited.

### Underground pockets

**Underground pockets** (the tool of that name, in the Decor group) lay
blobs of other blocks in the rock under the ground, as WorldPainter's layer
of that name: gravel, clay, granite, a mod's stone. Each pocket layer is
made of blocks of your own, with their weights, and takes its **share** of
the rock where it is painted (5 % to all of it), in pockets about its
**size** across, between its **lowest** and **highest** heights. The
pockets never reach into the strata of the ground's terrain — the grass
and dirt over the rock stay as they are — nor into caves or water.
**New pockets** (in Tool Settings or the Layers panel) makes one: gravel,
dirt, clay, andesite, diorite, granite or tuff, or any full block of the
game or a mod. The pockets show over the map, hatched in the layer's
colour, while the tool is in hand; the right button rubs them out. Ores
take the place of a pocket's blocks where they are rock an ore takes
(andesite, diorite, granite, tuff), as in the rest of the rock, never of
gravel or clay; the export, the survival check and the cut of the Ores
window agree block for block, so a pocket of diamond blocks is caught as
riches lying around.

### Smooth terrain

**Edit ▸ Smooth terrain…** builds the top of the land in partial blocks when
the map is exported, so that it follows its precise height rather than
whole blocks — what WorldPainter's half blocks and the plugins that used a
mod's full range of models did, without a layer to keep in step with the
terrain by hand. One switch, **Smooth the whole map**, turns it on; then
each column takes the pieces of the block painted there, and repainting
grass as sand makes the smoothing follow.

- **Layers** (eighths of a block, as snow's): the land's height between two
  blocks picks how many, in place of the top block or over it.
- **Slabs**: a half step, by the same height — by eighths for a mod's slabs
  that have them (Conquest Reforged's). A block whose slab goes by eighths
  but has no layers (Conquest's brown moss, dark mud) takes its slab where
  layers are asked for.
- **Stairs**: at the foot of a one-block step, a stair facing it; in an
  inner corner an inner stair, at an outer corner an outer one.
- **Vertical slabs**, at the foot of a cliff of two blocks or more, against
  it; **Corners**, corner pieces at the outer corners of steps.

The page lists the terrains painted on the map (**Every terrain** shows them
all), each with the pieces it may use: a box per kind, greyed where its top
block has none in the game or the mods read. Those you change are in bold;
the others take the defaults of their block, from the game (slabs and stairs
for stone, bricks and planks, layers for snow) and from the mods' profiles —
Conquest Reforged's gives its soils, sands and gravels layers and its rock
and masonry the whole range. A block no rule names takes its height in
layers or slabs, where it has them. **Back to the defaults** forgets the changes;
**Apply** is one step to undo.

The pieces are found by name, as the Blocks panel finds the shapes of a
block, among the blocks of the version exported to and of the mods read: a
mod's profile adds its suffixes as data (`_layer`, `_vertical_slab`,
`_corner_slab`…), in its own namespace or for the game's blocks (Conquest's
`grass_block_layer` for the game's grass). The mods' stonecutter recipes say
the rest: which block each shape is cut from, so that a shape a mod names
otherwise is found too — Conquest calls the game's stone Limestone and cuts it
into `limestone_slab`, its black terracotta is Black Hardened Clay with a
`black_hardened_clay_slab`. A slab by eighths from them wins over the game's
half slab (stone is smoothed by eighths of limestone slab when Conquest is
read). Only a block's own shapes count: a recipe that cuts sand into grass and
sand layers makes another block, not sand's layers. A block without a piece
stays whole. A piece only goes in the air or in still water over the top block
(in water it is waterlogged), never under an object. Under a ground cover's
plants (or snow), the land keeps its height in layers or slabs and the plants
stand on them, as on WorldPainter's layers.

The **Smoothing** brush (Decor group) paints over the map's setting: **On**
here, **Off** here, or **A slab** of your choice at the steps with its
stairs, whatever the block; the right button gives the map's setting back.
What it painted shows hatched while it is in hand. The 3D View shows the
pieces as boxes of their height (stairs three quarters high, vertical slabs
and corners half). Exports to 1.12 and older have no pieces.

### WorldPainter's layers in Massif

| WorldPainter | In Massif | From a `.world` |
|---|---|---|
| Deciduous, Pine, Jungle, Swamp | Object layers of the pack's trees: big oaks and birches; tall spruces, slender spruces and firs; rainforest giants and bushes; willows | Converted, as dense as painted (WorldPainter thins its forests by the square of their strength, and so does the import) |
| The tree pack of WorldPainter Modernised (Big oaks, Firs, Tall spruces…) | The same trees, in Packs ▸ Trees | Converted to the same object layer |
| Custom object layers | Object layers | Converted: their objects (Sponge `.schem`, MCEdit `.schematic`, structures, Bo2) are read from the world itself, with their offset, frequency, random turn and density |
| Custom plant layers | Ground covers | Converted: the same plants in the same shares, on every column painted, on valid blocks only if the layer said so |
| Frost | The Frost cover | Converted |
| Lava (flood with lava) | The Lava cover | Converted |
| Void | Columns of nothing: lower the land to the floor of the world | Converted: nothing in those columns |
| Annotations | Annotations | Converted, colour for colour |
| Biomes, custom terrains | Biomes, terrains | Converted: each custom terrain one terrain of its blocks, laid as WorldPainter lays its mix — noise as each block on its own, blobs as patches of about their size, layers by height from Y 0 (repeating and sloping as set; their variation is left out) |
| Caverns, Chasms, Caves | Cave layers (the Caves tool, Edit ▸ Whole map ▸ Caves) | Converted: a cave layer for each mix of them painted together — Caverns as chambers, Chasms as ravines, Caves as tunnels and noodles —, as dense as they were painted on average, painted where they were; closed under the ground, without lakes (set them in the layer) |
| Resources | Edit ▸ Ores | Massif lays its own ores |
| Tunnels, floating layers | Levels: Carve and Solid | Converted: a Carve level each (a Solid level for a floating one), its floor and roof where WorldPainter puts them (at a height, so deep under the ground, inverted, so high over the floor, within their limits), flooded up to its level; their noise and their floor, wall and roof materials are left out (the floor keeps the ground's terrain) |
| Custom ground cover layers | Terrains with strata | Converted: a terrain of the cover's blocks over each terrain it lies on ("Snow Block on Grass"), the land raised by the cover's thickness, or the top of the land replaced for a negative thickness; its edges are sheer |
| Combined layers | Combined layers | Converted: a combined layer of the same layers (its terrain, biome, covers, trees and pockets) at their factors, to paint on with; where it was painted, the layers it holds, each as strong as its factor, and its terrain and biome where painted at half strength or more |
| Underground pockets | Underground pockets | Converted: a pocket layer of the same blocks (its material, its mixed material, or its terrain's blocks), as frequent and as large, between the same heights |
| River | A river dug into the ground | Converted: dug where painted as WorldPainter's export digs it (its water two blocks under the ground, its bed four to seven blocks under the water, shallower towards the edges of what is painted, under water already there only its bed), joining a lake or the sea within ten blocks at its level; its bed is Massif's rivers' (gravel, sand in the shallows, clay in patches) rather than WorldPainter's mix of grass, gravel, clay and stone, with the river biome. Exported, its ground and water match WorldPainter's own export to the block in 78 columns of 80 checked, the other two a block apart |
| Populate, Read Only | — (Massif writes finished chunks, and merges none) | Left out |

The import's report says which layers became what, and lists those it
left out with the number of tiles each covered.

**WorldPainter layer files** (experimental): a custom layer exported from
WorldPainter (Export in its menu, a `.layer` file) comes in with **Import
WorldPainter layer…** at the top of the Layers panel, several at once, or
dropped on the window. Plant layers, custom object layers (Bo2, schematics,
structures, their objects read from the file), ground covers, underground
pockets and combined layers (with a custom terrain written with them)
become Massif's layers, as from a `.world`; tunnel, floating and custom
annotation layers are not carried over. A report says what each file
became and what it held that Massif left out — the settings of objects
Massif does not have (collision, leaf decay, foundations…), a ground
cover's edges and noise. The map's own layers are never changed: a layer of
the file with the name of one of the map's comes in as a layer of its own.
[Layers](layers.md) has the whole table.

## Annotations

**Annotations** (the **Annotations** tool, in the Paint group) mark the map in colour, as WorldPainter's do: where a town goes, a road to build, a river to dig. They are drawn over the map in every view mode and on the land of the 3D views, and are **never exported**. The right button rubs them out.

- **Colours**: WorldPainter's fifteen, then the map's own **annotation layers**. The **Colour** button of the tool's settings opens a colour picker: picked from one of the fifteen, it adds a colour of the map's own (kept with the map, up to 240, shown after the fifteen); picked from one of the map's own, it changes that colour, and the marks painted with it change too. Each colour paints in its own colour.
- **Annotation layers**: **+** beside Annotations in the **Layers** panel makes one, with its own name and colour, as WorldPainter's custom annotation layers. The fifteen colours take each other's place on a block; annotation layers do not: each is a layer of its own, over the colours and over the other layers, so that several mark the same blocks, as masks (biomes, tree sizes, flow lines, the land's colours). The brush and curves paint the one picked over what is there; the right button rubs out that one alone. The map and the 3D view draw each in its colour and opacity, one over the other; the **Info** panel lists them all; a filter's **Only on** finds each wherever it is. Each is listed by name under the colours, or in its palette: annotation layers are sorted into palettes with the other layers, and a palette's eye and solo show and hide them too. A click paints with one, a double-click renames it, its eye shows or hides it alone (its marks are kept), and its menu sets its colour and opacity, finds it on the map, shows it alone (Solo) or moves it to another palette, and **Remove** deletes it with its marks (one step to undo; the other layers keep their numbers). One in a palette is listed there only, not among the colours, and its palette's eye shows or hides it even with all the annotations hidden; the eye of all the annotations shows or hides the colours and the layers in no palette. As in WorldPainter, the layer you start painting with is shown, whichever eye hid it. A WorldPainter world's annotation layers import as such, overlaps kept, each in the palette it was sorted into, and so does a WorldPainter layer set **not to be exported** (a placeholder marking where something goes): it comes in as an annotation layer of its name, marked where it was painted, rather than as the plants or blocks it holds.
- **Opacity**: each colour has its own (75 % unless set), in the tool's settings.
- **Show or hide them all**, on the map and in 3D: the eye in the tool's settings and beside the colours of the Layers panel, the pen button in the map's header, or **View ▸ Show Annotations**.
- **Curves**: **Curve** at the top of the tool's settings, in place of **Brush**, draws annotations as curves, as Axiom's Path tool: click to place points, drag a point to move it, drag the curve to add a point, right-click a point (or Delete) to take it off. **Smooth** (a spline through every point), **Bézier** (two handles on the point picked) or **Straight**. It is drawn as the ribbon it will paint, from its **Start width** to its **End width**, until **Paint curve** (Enter) paints it in one step to undo. Escape drops it.
- **Import Mask** paints annotations from a colour image, each pixel taking the nearest colour (see [Import mask](#import-mask)), and scripts read and write them (see [Scripting](#scripting)).

![The Annotations tool: WorldPainter's colours, an annotation painted on the map](images/annotations-tool.png)

## Packs

**Packs** is the library of ready-made things to paint, so that a new map
can be dressed without setting anything up, as in WorldPainter Modernised.
It is the second tab of the Layers panel (**This map** · **Packs**), and an
editor of its own in the area chooser. A click on an item adds it to the
map and picks it to paint, with its tool; a click on one the map has
already (marked with a dot) picks it again. A search field filters them.

![The Packs panel and its categories](images/packs-panel.png)

| Section | Holds | Becomes |
|---|---|---|
| Trees | Tall spruces, slender spruces, firs, larches, giant redwoods, big oaks, pale oaks, birches, cherry trees, acacias, willows, rainforest giants, palm trees, dead trees, bushes, azalea bushes — eight of each, in different sizes and shapes, and eight smaller ones (bushes and small trees) that stand only where Tree sizes ask for them | An object layer (bushes and azalea bushes: a sub-canopy one) |
| Flowers | Wildflower meadow, flower mix, tulip field, sunflower field, roses and lilacs, spring flowers, pink petals, swamp flowers | A ground cover |
| Plants & ground cover | Grass and flowers, dense grass, forest ferns, forest floor, berry bushes, dry grass, desert plants, jungle undergrowth, pale garden, mushroom patches, pumpkins and melons | A ground cover |
| Water plants | Seagrass and kelp, coral garden, sugar cane | A ground cover |
| Mixes | Grass, dirt and soil, stone, sand and shore, cliff strata, snow, ice and volcanic rock, built surfaces | A terrain |

The trees are made by Massif, always the same way, as the tree pack of
WorldPainter Modernised makes them; their leaves never decay. The mixes lay
their blocks one by one, in **patches** a few blocks across, or, for the
cliff strata, in **beds**: horizontal bands that repeat upwards, each block
as thick as its share, dipping a little and wavering, down through the land
so that cliffs show them. Once added, each is a layer like any other: its
settings in Tool Settings, to change as you like.

An export for an older version of the game turns what it does not have into
the closest it has: cherry trees become birches, pale oaks dark oaks,
willows' mangrove leaves oak leaves, keeping the way their logs lie and
their leaves' persistence.

### Sharing packs

Packs can be made from a map and shared, so that the community can trade
the trees, plants and mixes they have set up.

**File ▸ Export Pack…** makes a pack of the map's layers. Its dialog lists
the map's object layers, and its ground covers and terrain mixes that are
not those every new map has; tick those the pack holds. Give it a **name**,
an **author** (remembered for the next time) and a **description**. Its
**picture** is drawn from what it holds, side by side, or **Choose…** one
(a PNG or a JPEG, fitted into 512 × 512). **Export…** writes the
`.massifpack` file.

A pack is added to **Your packs** by any of:

- **File ▸ Import Pack…**, or the **Import…** button of the Packs panel;
- dropping the file on Massif's window, on the home screen or over a map;
- opening it from the Finder or Explorer (a double-click: Massif owns the
  `.massifpack` type, as it owns `.massif`).

Your packs come first in the Packs panel, each under its picture, its name
and its author ("by Nedgar · 3 things"; its description shows over it).
Their things are tiles like Massif's own: a click adds one to the map and
picks it to paint. If the map has a layer of that name that is not the
same, the pack's comes in as "Name (Pack)", the map's left as it was. The
search finds a pack by its name or its author. Each pack's **⋯** menu has
**Export a copy…**, to pass it on, and **Remove…**; maps keep the layers
they took from a pack removed.

Importing a pack whose name one of yours has asks whether to **Replace**
it or **Keep both**, the new one under another name.

**What a pack file holds.** A `.massifpack` is a zip archive:
`manifest.json` (its format, `massif-pack`, and the version of that
format, then its name, author, description and the version of Massif that
wrote it); `content.json` (its covers, terrains, object layers and
objects, each block by its state such as `minecraft:oak_log[axis=y]`);
`objects/<n>.bin` (the blocks of each object); and `preview.png`. A newer
Massif reads every older version of the format; an older one says when a
pack needs a newer Massif.

**Safety.** A pack comes from someone else, so Massif checks all of it
before adding it, and refuses it whole when anything is wrong: a file over
64 MB, more than 2048 entries, an entry that inflates past 32 MB (160 MB in
all) or to anything but the size it says, a failed checksum, an encrypted
or unknown kind of entry, an entry named with `..`, a root, a drive or a
backslash, a block state that is not one, an object larger than 16 million
blocks or naming blocks it does not have, a picture over 1024 × 1024. A
pack holds blocks and settings only: nothing in it is ever run, and no
name in it ever becomes a path. Massif writes its own copy of each pack it
adds into its folder (Application Support, or AppData), named after the
pack.

## Ground covers

The **Ground cover** tool (Paint group) paints what grows on the ground:
grass, tall grass, flowers, ferns, mushrooms, dead bushes, snow, seagrass,
frost and lava come with every world, and **New cover** makes one of your own. A cover is
a mix of blocks with their weights — the flowers of a meadow, or any block
at all, typed or picked from the game's blocks — with its colour on the map
and the **opacity** it shows at, a **density** (the
share of the painted ground where something grows), a **height** (cane and
cactus 1 to 3 blocks), **only on soil** (grass, dirt, podzol, moss, mud:
plants pop off sand and stone in the game) and **under water** (seagrass,
coral). The right button takes a cover off. The brush, its intensity and its
options (height band, slope, only on a terrain…) apply as with any paint;
the eyedropper picks the cover under the cursor.

![A ground cover's settings: its plants and their shares](images/ground-cover.png)

Covers grow on the topmost surface of a column — the ground, a floating
island's top, the floor a carve opens — never under an overhang. Where each
plant grows is decided column by column from the world's seed, so the map,
the 3D view and the export agree. Zoomed in, the map shows the cover's
colour at its opacity in each column where something grows, and lighter
(two fifths of it) where the ground stays bare; from afar, the mean of the
two: the denser a cover grows, the more of its colour shows, and at 0 % its
painted ground stays in sight. The 3D view shows every plant in its own
colour (dandelions yellow, poppies red, cornflowers blue…). Tall plants are written in their two halves; versions of
the game without a plant get the closest they have (short grass is grass
before 1.20.3, cornflowers are blue orchids before 1.14, and before 1.13
plants are numbered, tall ones as double plants).

## Custom objects

The **Objects** tool (in Decorate, beside Ground cover) paints where objects
stand. **Import objects…** in its settings — or files dropped on the window
— reads them from:

![An object layer's settings: its objects, their sizes and how they stand](images/object-layer.png)

| File | From |
|---|---|
| `.schem` | WorldEdit and FAWE (Sponge schematics, versions 1 to 3) |
| `.schematic` | MCEdit, Schematica and old WorldEdit (blocks numbered before 1.13, turned into today's: logs and their axis, leaves, flowers, slabs, stairs…) |
| `.bp` | Axiom blueprints |
| `.litematic` | Litematica (all its regions in one object) |
| `.nbt` | Structure blocks |

Air and structure voids are left out; an object stands on its lowest layer
that is not soil or hanging plants — the foot of a trunk — and the dirt a
tree was copied with goes into the ground. Files imported together make a
layer, and every layer has its objects with their weights, a **spacing**
(the side of the squares of its grid: one object at most in each), a
**density** (the share of the squares that get one), a random **turn** and
**mirror** (their blocks follow: logs lie along the new axis, stairs and
vines face the new way), **only on soil**, **under water**, a **sink** (how
deep they go into the ground) and **replace the ground** (otherwise their
blocks only take the place of air, water and plants). The right button takes
a layer off; Add objects… reads more files into a layer.

### Growing trees

**Grow trees…**, in the Objects tool's settings and beside Add objects… in
a layer's, grows trees from a few settings rather than reading them from
files. Start from a kind (Oak, Spruce, Birch, Cherry, Palm, Willow, Acacia,
Dead tree, and two giants: Giant sequoia, about a hundred blocks tall, and
Great oak) and change what makes it:

| Setting | What it does |
|---|---|
| Height, Trunk width | The tree's size, up to 128 blocks tall and 9 across; each seed varies it a little |
| Limbs | How many limbs leave the trunk, in a spiral (in whorls for a cone) |
| Droop, Reach for the sun | How much branches bend down under their weight, and turn up to the light |
| Crown | Cone, Round, Umbrella (flat clumps), Weeping (curtains of leaves), Palm (fronds) |
| Clump size, Fullness | The clumps of leaves at the ends of the twigs; 0 % for a bare tree |
| Wood, Leaves | The blocks |
| Survival friendly | Only logs and leaves, the blocks a tree of the game has |
| More settings | The trunk's flare, how far up it goes, where the limbs start, branch levels (1 to 4), their angle, length and how crooked they grow |

The trunk flares into roots at its foot, smoothed with stairs and slabs;
thick wood is logs laid along their axis, thinner branches walls (of the
stone closest to the bark), the thinnest fences (of the planks closest to
it) that vanish into the leaves, whose clumps leave gaps the wood shows
through; every leaf is persistent and
within the game's reach of wood (a palm's fronds reach further). Nothing
floats: every block holds to the trunk. The picture shows the tree of the
seed, and those of the next four seeds under it; a click on one takes its
seed. **Add … trees to a new layer** makes an object layer of as many
variants as Trees says, each grown from the next seed; **Add to** adds them
to the layer picked. Bushes and small trees for Tree sizes are grown with
them, of weight 0, as for the packs' trees. The same settings and seed
always grow the same tree. A tree too tall to stand under the top of the
world where it is painted is left out there rather than cut, on the map,
in the 3D view and in the export alike. Growing a giant of 120 blocks takes
about 30 ms.

Where each object stands is decided from the world's seed, square by
square, where the layer is painted on the top of the land: the map draws the
objects from above with their shadows when zoomed in, the 3D view block by
block in their colours (the game's own when its blocks are loaded), and the
export writes them whole, across chunks and tiles, for every version the
blocks exist in. Block entities (chests' contents, signs' text) and entities
are not kept.

## Water

The **Water** group holds three tools, which work at any height: the sea's level is not a limit.

- **Water brush**: lays water under the brush, at the height Tool Settings says; the right button takes it off.
- **Flood**: a click fills the basin under it with water, as a bucket would, up to where it would spill; a click on water raises it a block, the right button lowers it a block, **Shift + right click** drains it whole. Hold to repeat.
- **River**: click a source, and the water finds its own way down to the sea (see [Rivers](#rivers)).

The **lava** setting of a new map, and the **Lava** ground cover, give lava in place of water. Water written to the world holds as the game holds it (see [Water in the game](#water-in-the-game)).

![Flood in hand: its Tool Settings beside the map](images/flood-tool.png)

### Flood and drain

The Water group has three tools: the **Water brush**, **Flood** and the
**River** tool. They work at any height, wherever you use them; the sea's
level is not a limit.

![A lake flooded in a hollow, seen in the 3D View](images/flood-lake.png)

**Water height.** The Water brush and Flood each take the water's height
from one choice in Tool Settings, with a line under it saying in plain
words what will happen:

For Flood, this height is the level of a new fill, on dry land: a click on
water always raises it a block, whatever the choice.

- **Where I start** (Water brush): each stroke fills to the height where
  it starts, the surface of the water there, or a block over the ground.
  **Where I click** (Flood, and what it is set to until you pick another):
  the water poured where you click runs down the slope into the hollow
  below, and fills it up to the lowest point of its rim, where it would
  spill; a lake already in it counts as its floor. Clicked in a hollow,
  that is the hollow clicked.
- **Sea level (63)**: the water is laid up to the map's sea level.
- **Fixed**: up to a level of your own, set in the **Level** field shown
  in this mode. This is the water's level, not a limit: the Water brush
  leaves ground at or above it dry, and says so. Flood, clicked on land
  as high as that level or higher, fills the hollow the water runs into
  instead, as Where I click does, and says so by the cursor (**Above y
  63: fills this hollow instead, to y 118**) and once it is done.

Over the map, a tag by the cursor says what the tool would do there:
**Fills to y 74** (the Water brush), **Fill to y 63**, **Runs down into a
hollow: fills it to y 104** (with a ring on the hollow and a dotted line
to it) or **Raise this water to y 64** (Flood), **Height limit y 80
reached**, or why the ground stays dry (**Dry here: the ground is at or
above y 62.**), so that a level is never a surprise. Settings saved by an
older version carry over: From the click on is Where I start, off is
Fixed at the level that was set, for the Water brush; Flood's water
height comes back to Where I click until you pick one.

**Flood.**

- Clicked on dry land, it puts water there: in the hollow clicked, or, on
  a slope or a hilltop, in the hollow the water runs down into, up to
  where it would spill (with Sea level or Fixed under the land clicked,
  the area clicked, wherever the land lies lower). Puddles of fewer than
  a dozen columns on the way are run past. The message says where it
  went: **Flooded 230 columns up to 104**, or **The water ran down into
  a hollow: flooded 230 columns up to 104**. Running into a lake that is
  not full, it raises it up to its rim. Where the water would run away
  (into the sea, off the map, more than 256 blocks down, or out on wide
  flat ground), it fills the nearest hollow within 64 blocks of the click
  instead (**No hollow here: filled the nearest one, 140 columns up to
  96**). Only with no hollow near does it ask first: made to stay a
  block over the spot, the water would spread wherever the land lies
  lower, and the question says over how many columns and up to what
  level; **Flood anyway** floods it (one step to undo, inside the
  selection only when there is one), **Cancel** leaves the map as it was.
  With **Warn before flooding beyond a hollow** off, it floods at once.
- Clicked on water, whatever the water height chosen, it raises that body
  of water by one block: it covers the shore a block above it, as far as
  it reaches. A lake at y 63 clicked with Fixed at y 63 goes to y 64. At
  the rim of its hollow, where a block more would spill, it asks first, as
  above. At the height limit, when it is on, it stays and says **Height
  limit y 80 reached**.
- Its **right button** lowers the body of water clicked by one block: its
  top layer goes, and the shore it uncovers stays as the ground was. When
  the last layer goes, the water is gone. **Shift + right click** drains
  the whole body at once. (Flood took over from the Drain tool: a Drain
  saved in your settings is Flood.)
- **Held down**, either button keeps going, a block at a time, at the pace
  set by **Hold speed** in Tool Settings (6 blocks a second by default):
  up to the rim of the hollow (or on past it with Warn before flooding
  beyond a hollow off), never over the height limit or the top of the
  world, down to the bottom. The 2D map and the 3D views
  follow it as it goes, and the whole press is one step to undo.
- A flood changes **67,108,864 columns at most** (a square of some 8,000
  blocks a side), all of them or none: one that would cover more, a body
  of water that large lowered or drained, or water raised a block past
  where it would spread that far, is left as it was, and the message says
  why (**Too much water for one flood: it would cover more than
  67,108,864 columns. Nothing was flooded.**). Stopped halfway, the water
  would stand cut off where it stopped, a wall of water over the land and
  the sea beyond it.

**Water brush.** It lays water under the brush at the water height chosen;
the right button takes water off.

**Height limit.** Folded under the other settings, its state in its title
(**Off**, or the limit), **Limit the water's height** is the only thing
that caps the water: every water tool stops at the **Highest** level set
there, whatever the water height, and says so when the limit is under the
spot used.

All of them keep to the selection, and each click, press or stroke is one
step to undo.

## Rivers

The **River** tool (Water group) lays a river out from its source:

![The River tool in hand: its Tool Settings beside the map](images/river-tool.png)

- **Click** on land: the river springs there, and its water finds its own
  way down — down the slope, a block at a time; a hollow it meets fills up
  to where it overflows, and it goes on from there — to the sea, into water
  already there, or off the map. Before the click, hovering shows where the
  water would go.
- **Click elsewhere**: a point more, exactly where you clicked, and the
  river ends there; each click on the land after that adds a point at the
  end, the points already laid staying where they are. The way from one
  point to the next is worked out over the land, as water would take it:
  along the valleys, climbing as little as it can, into the water it
  meets, keeping near the line between the two. The points are numbered
  in the order the river runs through them: the source blue, the end
  dark.
- **Click on the river**, or drag it, anywhere along its course to bend
  it: a point it must pass, between the two points that stretch joins.
  Drag a point (its handle, the numbered circle) to move it: it alone
  moves, by as much as the pointer, and a click on a handle without
  dragging moves nothing. **Right-click** a point to take it away: the
  others stay, the point before the end becomes the end, and the one after
  the source the source.
- **Dig river** (or Enter) digs it, one undoable step; **Cancel** (or Esc)
  drops it. The bar over the map says how long it is and where it goes.

Its settings: **Width** at the source, **Flow** (how much it grows
downstream: up to five times wider and two and a half times deeper at its
mouth; on the way, as the land draining into it grows, as water finds its
way over the ground around it: it widens over some thirty blocks where a
valley joins it, little along a stretch that gathers nothing),
**Depth**, **Erosion** (how much it wears the land beside its bed: a wider
valley), **Banks**, **Lakes** (which hollows fill: deep ones only, or every
one, or none — through the others it cuts a gorge), the terrain of its bed
and a river biome on its water.

- The course is made natural before it is shown and dug. Where it comes
  back within a width of where it was before (water filling a shallow
  hollow wanders round it), it goes straight across instead, unless that
  would climb; its runs along the grid (the ways found across hollows and
  over the land go from cell to cell) are smoothed over about its width,
  without moving up a slope; and on gentle ground (a drop of less than one
  block in twenty) it meanders: a wave across its way some ten times its
  width long and up to half its width to either side, smaller where it
  would climb out of its valley, none near its ends and the points you
  dragged it through. None of this moves it more than half its width from
  the way found, nor across itself or back on itself: a stretch between
  two points that would is left as found. Then a smooth curve runs
  through all of it, through every point you dragged it through and to
  the end you clicked: what is dug is what was shown. While a point is
  dragged its way is worked out coarsely; let go, finely, along the way
  shown rather than down another valley.
- Water runs from the source down. Its level along the river is the lowest
  the land allows so far, a
  little under the banks: it never rises downstream, whatever the bumps.
- Where the land is really steep (the river would drop seven blocks or
  more over the next 24, a grade of some 30 %), it falls from pool to pool
  instead of stepping down a block every few blocks: its water stays level
  in a pool dug into the slope, then falls three to five blocks into the
  next one, the steeper the higher, with a plunge pool dug under each
  fall, rounded out a quarter wider. On gentler ground (down a valley of
  10 or 20 % too) it steps down a block at a time, rapids. Through a lake or a river on its
  way, it keeps their level, and never falls below the water it ends in.
- Its shape follows the land. On gentle ground the river is a little
  wider, over a flat bed with rounded sides; where the land is steep (a
  grade of 12 % or more over 16 blocks either way), it is narrower and a
  quarter shallower, cut in a V. Its width wanders by about a tenth over
  some fifty blocks, as a real river's does.
- The bed is `depth` deep in the middle and rises to the water's edge;
  banks climb back to the ground smoothly, over the chosen width at least,
  and over two and a half times as far as the river lies under the land
  around it (48 blocks at most): a valley sloping down to the water, not
  the walls of a trench. The banks follow the river's level smoothed over
  some twenty blocks either way, as one valley: past a fall they ramp
  down rather than step, and no row of ledges lines them. Only the ground is lowered, but for a bank that
  rises back towards the ground it had to hold the water.
- Where the water runs slowly, on gentle ground, the bed has patches of
  clay among its gravel.
- A course that starts on a river is a branch, leaving it at its level; one
  that ends on a river or a lake is a tributary, meeting it at its level.
  Two branches that meet again make a split stream.
- The bed gets a terrain (gravel by default), the water a river biome.
  Its shallows are sand: the edges of the bed, the inside of its bends,
  where the current is slow, with a strip of beach at the foot of the bank
  there.
- Into the sea, the river does not widen: its channel goes on under the
  water, cut into the sea floor and down it, shallower and shallower until
  it fades into the floor (or after six times its width), opening out a
  third wider at most; its floor is sand, with a fan of sand where it
  ends, and the river's biome thins out into the sea's. Its water never
  goes under the sea's surface, even where it reaches the sea over a beach
  at the water's level.
- The lakes and the rivers it runs into keep their floor, their terrain
  and their water.

The water's level is a whole block per column, so it stands in steps
along the river. The 3D View and 3D Sculpt draw each step as the game
does: flowing water poured over the lower water, its surface sloping down
from the water above (a fall: a curtain of it down to the water below),
not a wall of water. The water stays in its bed: where a bank was dug lower
than the water beside it, the bank rises back towards the ground it had
to hold it, and where it cannot, the river's own water goes down to the
bank's height. Water that was on the map before the river (painted by
hand, or brought in from WorldPainter) is never lowered. Edit ▸ Whole map ▸
Rivers holds its water the same way.

### Water in the game

Minecraft leaves the water of an exported world still until a block next
to it changes: then every water block with open air beside it runs into
it. Two sources beside the same block make a new source, so a pool one
step under another rises to the upper level, then the next, until the
water overflows its banks. On a WorldPainter map with rivers and lakes of
5,888 blocks a side, nearly 400,000 blocks of water would appear that way,
157,000 of them over dry land.

The export writes the water as the game leaves it once every block of it
is updated, worked out with the game's own rules: a row of flowing water
at the edge of each step, falling water where it drops. Only where two
sources would make a new one, and a pool rise, are the sources beside it
written as flowing water instead, which over other water never spreads.
The game then finds nothing left to change: no pool rises, nothing
overflows. Water painted over land lower than itself (on a slope, say)
runs down that land in the export as it would in the game. Blocks of
custom objects that stand in still water and can hold it (fences, slabs,
stairs, leaves…) are written waterlogged, with no pocket of air. A world
of lava is written as before.

## Roads

The **Road** tool (Sculpt terrain group) lays a road through points
clicked on the 2D map and builds it into the land:

![The Road tool in hand: its Tool Settings beside the map](images/road-tool.png)

- **Click** on the map: where the road starts; each click after adds a
  point at its end. A smooth curve runs through all of them.
- **Drag a point** to move it; **drag the road** anywhere between two points
  to add one there and move it. **Right-click** a point, or press
  **Delete** (the point last taken hold of), to take it off.
- **Build road** (or Enter) builds it, one undoable step; **Cancel** (or
  Esc) drops it. The bar over the map says how long it is, the steepest it
  climbs and how much of it is on bridges.

Its settings, in Tool Settings: **Width** of the level way, **Banks** (how
far its sides ease back into the land), **Steepest slope** (in percent:
12 % climbs a block every eight), **Surface** and **Bridges over water**
with the **Deck** they are made of.

- The road follows the land where the land is gentle. Where it is steeper
  than the limit, the road cuts through the humps and fills the hollows,
  halfway between, so that it never climbs steeper than the limit. Its two
  ends meet the land as it is.
- Across its width the road is level; past it, the banks blend back into
  the land.
- **Surface**: a trodden path, a gravel mix, a cobblestone mix, stone
  bricks or a wooden deck (added to the map's terrains when a road is built
  with them), any terrain of the map, the noise of the Paint tool, or the
  ground left as it is. Nothing grows on the road (its ground cover goes),
  and no object stands on it or along its edges.
- **Over water**, with bridges, the road crosses on a deck one block thick,
  a block above the water's surface, laid on a level of its own named
  **Bridges** (added with the first bridge); the water and the bed stay under
  it, and the road climbs down to the deck's height on either shore.
  Without bridges, the road is built up a block over the water.
- A **selection** holds the road: only what is selected changes, faded along
  a feathered edge. The **brush filters** choose where its surface is
  painted (only on grass, not above a height…); the grading ignores them.

## Levels

The map's ground is a height for each column. **Levels** add rock over it or carve rock out of it, between a bottom and a top of their own, for what a single height cannot hold: overhangs, arches, floating islands, tunnels, pits. The **Levels** panel lists them:

- **World** at the top, with the map's **Name**, **Seed**, **Sea level** and **Spawn**.
- The **Ground**, then each level, with its kind, its name and how many tiles it covers (double-click to rename). **Solid** adds rock, **Carve** removes it; a higher level wins over a lower one. The buttons add a **Solid** or a **Carve** level; each level has **Show** / **Hide**, **Move up**, **Move down** and **Delete**.
- Click a level to make it the **active level**.

The **Levels** tools paint on the active level, which must be a Solid or a Carve one: **Stamp slab** adds to it (the right button erases), **Erase slab** erases it (the right button adds). Tool Settings sets the slab's **Thickness** and where its **Centre** stands (**Above ground**, by an **Offset**, negative underground; or **At height**), and what a stroke edits: the level's **Surface**, its **Top** or its **Bottom**. Levels are best seen in the 3D view. Paint terrain and the other paint tools paint the active level's top when it is not the Ground. The Terrain Generator writes floating islands and overhangs as Solid levels of their own, and WorldPainter's tunnel and floating layers import as Carve and Solid levels.

![The Levels panel: Solid, Carve and the Ground](images/levels-panel.png)

## Caves

Caves are a layer painted on the map, carved as Minecraft carves them since
1.18 wherever it is painted. A **cave layer** says what its caves are:

![Edit ▸ Whole map ▸ Caves…: chambers, tunnels, noodles and ravines](images/caves-dialog.png)

- **Chambers**: large rooms with thin pillars of rock left standing in
  them, the game's "cheese" caves.
- **Tunnels**: long winding tunnels, its "spaghetti" caves.
- **Noodles**: thin tunnels, many of them, in half of the rock.
- **Ravines**: long, deep, narrow cuts with steep walls, narrower toward
  their floor.

Each has its **Density** (the share of the rock it takes, from none to the
most), its **Size** (1×: as the game carves them) and the heights it keeps
to (**Highest**, **Lowest**); it thins out toward the ends of its heights
rather than stop flat. The caves keep a **Roof** of rock under the ground's
surface, except where they **open to the sky**: the share of dry land where
they may break through, never within three blocks of water, so that no lake
or sea pours into them.

**Aquifers** fill the caves with water, and with lava deep down, at levels
of their own, as the game's: the rock is cut into zones some fifty blocks
across, each dry or holding water up to its own level (**Lakes**: the share
of zones that hold some; **Lava lakes**: the share of those under **Lava
under** that hold lava), and where zones of different levels meet, a wall
of rock stays between them. **Lava floor** fills every cave under a height
with lava, as the game's under y −54. Water and lava never run once the
game updates a block beside them: no fluid lies beside the open air or over
a cave, the caves keep a block under the floor of any lake or sea next to
them, and the export makes rock of the few blocks that would still let a
fluid run (a carve's room or a sculpted hollow beside a lake, say).

The **Caves** tool (Underground group) paints a layer where its caves go;
the right button rubs it out. Its settings, in Tool Settings, change the
caves wherever the layer is painted, at any time: nothing is carved for
good. **New cave layer** makes one as the game's. A layer's edges fade:
caves thin out across the last few blocks painted.

**Edit ▸ Whole map ▸ Caves…** paints a layer — a new one, or one of the
map's with its settings changed — over the whole map, or over the
selection by its weight, as one step to undo. Before anything is done it
pictures the caves of two tiles in the middle of the map: a plan of the
caves at a height you choose, and a cut through the land, their water and
lava in their colours.

Caves are worked out as the map is drawn and exported, from the layer, the
ground and the seed alone, a tile at a time (36 ms of processor time a
tile, 5.6 s for a map of 4,096 blocks a side on a ten-core Mac): they take
no room in the file, and the last tiles worked out are kept in memory
(384 MB at most). The 2D map and the 3D views show the caves open to the
sky; **Underground** on the 2D map and **X-ray** in the 3D views show the
others. 3D Sculpt digs and builds in them as anywhere: a cube of 16³ blocks
sculpted starts from the caves there, with their water and lava, and keeps
them as they were when it was sculpted.

### Layer by layer

The **Layer** tool, first in the Underground group, edits the map block by
block, one layer at a time — the way to fix a cave by hand, open an
entrance, wall off a lake or dig a mine.

![The Layer tool: the map cut at a height, shown in the map's header](images/layer-tool.png)

- **Seeing a layer.** With the tool in hand the 2D map is cut at a height:
  the **Underground** button in the map's header turns the cut on and off,
  its slider and field set the height, and **PageUp** / **PageDown** move it
  a layer (with **Shift**, 8 layers). The map shows that layer: rock in grey
  (darker under y 0), the soil's colour within four blocks of the ground's
  top and a block's own colour where one was filled, caves dark (darker the
  deeper their floor lies), water blue, lava orange, cave biomes tinted, the
  land above the cut faded. Each tile is composed once at every height and
  kept while the map stays cut, so moving through the layers redraws only
  the picture.
- **Editing it.** **Dig** makes air, **Fill** fills with the column's rock
  or with the terrain chosen in the Terrain panel, **Water** and **Lava**
  pour still water or lava, **Biome** paints a cave biome. **Layers under**
  and **Layers over** reach that many layers around the cut; with **Round**
  each dab is a ball (an ellipsoid over a brush that is not round), so a
  round brush digs a ball, otherwise a cylinder. The brush's edge is hard,
  block by block, and the selection limits it as any brush. The right
  button fills back with the column's rock. Each stroke is one step to
  undo.
- **With the terrain graph.** Caves made by a graph's Cave System and
  volumes show on the layer like painted ones. What is edited here is kept
  when the graph is applied again: the blocks the graph carved before
  follow its new settings, the blocks edited by hand stay as they are.

### Cave biomes

The **Cave biome** tool (Underground group) paints **Lush caves**,
**Dripstone caves** or the **Deep dark** between two heights (**Highest**,
**Lowest**): the game keeps a biome for every cell of 4 × 4 × 4 blocks, and
so does the painting, so a biome can lie under another or under the
surface's. The export writes them as the game's cave biomes, and decorates
their caves as the game does:

- **Lush caves**: moss over the floor and the ceiling, carpets of moss,
  grass, azaleas and flowering azaleas, cave vines hanging with glow
  berries, spore blossoms, rooted dirt with hanging roots, and small pools
  of water lined with clay, where big and small dripleaves stand.
- **Dripstone caves**: dripstone blocks in patches over floors, walls and
  ceilings, stalactites hanging and stalagmites standing, of pointed
  dripstone thick at their base and thin at their tip (merged where they
  meet).
- **Deep dark**: sculk over the floor, the walls and the ceiling, sculk
  veins clinging to the rock around, sculk sensors, shriekers (which can
  call the Warden, as the game's own) and catalysts, each with its block
  entity.
- Other caves: glow lichen here and there on walls and ceilings.

What a block of a cave's surface gets follows its place: a floor, a wall or
a ceiling, how flat the floor is, how much room there is above or below. The
decoration is the same every time, whatever part of the map is exported.
Sculk needs 1.19 or later; older versions get what they have.

### X-ray in the 3D views

**X-ray** in the header of the 3D View and of 3D Sculpt (the scanning eye)
draws the floors and ceilings of the caves under the ground through it,
see-through, warm near the surface and cold deep down: where the cave
layers carve, how deep, how the networks run. Walking or flying in the land
(the Player, its Ghost) shows every cave as it is, to go into.

### Objects and their density in the 3D views

**Objects** in the header of the 3D views (the tree) shows or hides the
trees, bushes and rocks the object layers place; the structures placed stay.
**Density** (the grid) colours the land by how many objects its layers will
place around each column, counted over cells of 8 blocks and shown as
objects per 100 blocks², from few (blue) to many (red), with its scale at
the bottom left. Land where nothing is placed keeps its colours, and the
objects step aside while the density shows, so that how thick each forest,
thicket or scree will be can be seen before exporting.
### Underground on the 2D map

**Underground** in the 2D map's header (the split square) cuts the map
through at a height: the rock in greys (deepslate darker), the caves dark,
lit by the relief of their floor and tinted by their cave biome, their
water and lava in their colours, the land over the cut faded. Set the
height beside it.

## Structures and villages

The **Structures** tool (Decor group) places villages and the game's own structures, read from your own Minecraft and mods and assembled the way the game does. In Tool Settings, pick what to place: under **Villages**, a theme (the game's plains, desert, savanna, taiga, snowy, or a builder's pack added with **Add a pack…**); under **The game's structures**, a pillager outpost, the desert and jungle temples, a witch hut, an igloo, a shipwreck, ocean ruins, an Ancient City, Trial Chambers, a stronghold, trail ruins, a bastion remnant. Then click the map to place it there, or, for a village, drag a rectangle that it fills. With a selection, **Fill the selection** builds a village in it.

Click a structure placed to select it: drag it to move it, drag its corners to resize a village, change its theme, seed, roads, walls and density in Tool Settings; the right button takes it away. Villages lay their roads into the land, level their plots and can be walled.

The **Building** tool changes one building of a village placed from a pack: a click selects it (its outline, handles and name on the map, its picture, file and size in Tool Settings), a drag moves it and lays a path from its door to the nearest road, **R** turns it a quarter, **Delete** takes it away (a farm with its field), and Tool Settings puts another building of the pack in its place from a menu of pictures seen from above, draws another of its type, or puts it back as drawn. Each change is one step to undo and is kept with the village when it is built again.

The whole story, with every setting, is in [Structures and villages](structures.md); [Village packs](village-packs.md) says how to make a pack of your own builds.

![The Structures tool in hand: its Tool Settings beside the map](images/structures-tool.png)

![The Building tool: one house of a pack village selected, its Tool Settings beside the map](images/building-tool.png)

## The whole map at once

**Edit ▸ Whole map** runs one of the terrain graph's filters over the whole
map, without opening the graph:

![Edit ▸ Whole map ▸ Erosion…: its settings and the map before and after](images/erosion-dialog.png)

- **Erosion**: rain runs down the slopes and carries soil from the heights
  to the valleys — gullies, worn flanks, fans of silt (the Erosion node:
  rain, erosion, deposition, capacity, evaporation, brush, seed).
- **Thermal erosion**: loose rock slides wherever a slope is steeper than
  it can hold — scree at the feet of cliffs, softer ridges.
- **Rivers**: traced where the water of the whole map runs, dug into the
  bottoms of the valleys, wider downstream, with their lakes, keeping to
  the water already on the map. They are shaped as the River tool's: down
  steep valleys they fall from pool to pool, narrower and in a V; on gentle
  ground they run wider over a flat bed; their width wanders a little; their
  banks open out into a valley where they lie deep under the land; and into
  the sea their channel goes on under the water, fading into the floor.
  Under their water the bed is gravel, sand in the shallows (under one
  block of water) and in the channels under the sea; the banks keep
  their ground, and so does a bed the water leaves dry. Where a river runs
  is set by how much land drains there: a share of the map's land, or on a
  very large map, whose valleys each gather only a small share of it, a
  catchment of some 900 blocks a side with the default setting (200 to
  4,000 from many rivers to few). **Width** is the width of the greatest
  river; the others are narrower as they gather less water, down to a
  quarter of it, and a tributary that would be narrower still is left out.
  The water holds as the game holds it, block by block: the bed under a
  step stays under the water above it (no dam of land across a river), a
  block of land left alone in the water goes under it, and a lake stands
  at most a block over the water beside it. A lake covers a 64th of the
  land at most (four times more with **Lakes** all the way up, four times
  less near none): a wider hollow holds one in its bottom.
- **Lakes**: water fills the hollows up to where they spill, block by block:
  every lake stands at or under its rim on every side, in one level. No
  lake covers more than a 64th of the land (or 64 blocks a side): the
  water of a wider hollow stands lower, a lake in its bottom (a hollow
  nearly flat all over stays whole). Its
  floor is dug into a bowl, a block deeper every three blocks from its
  shore, down to 2 to 10 blocks as it is larger; sand under its shallows,
  gravel deeper, a sand beach where its shore is flat (with **Fill**, their
  ground rises instead).
- **Smooth**, **Sharpen**, **Terraces** (flat steps over the whole height
  of the map) and **Strata** (ledges of rock along the slopes).
- **Scale and raise**: the relief scaled around the sea level (twice as
  high, half as high…), then moved up or down by a number of blocks.

Each opens in a window with its settings — those of its node — and the map
small, before and after, made again a moment after they change. **Apply
to the whole map** works on a copy of the map in the background, with its
progress and a Cancel, and puts the result in place as one step to undo.
The surface of the map is read as the Map node reads it, the filter changes
it, and only the change goes back into the ground: its paint stays (but for
the rivers' beds), and so do the rock of its other levels and what was
sculpted in 3D over it. Up to 4096 blocks across, every block is read; a
larger map is read a sample every few blocks and the change is spread over
the blocks between, so the ground keeps its own detail. On a WorldPainter
map of 5,888 blocks a side, erosion takes about twenty seconds, the others
two or three, terraces and smoothing under half a second.

### Snowfall

**Edit ▸ Whole map ▸ Snowfall…** lays snow over the map by a small
simulation, as the wind and the slopes would leave it:

![Edit ▸ Whole map ▸ Snowfall…: its settings and the snow it would lay](images/snowfall-dialog.png)

- **Intensity**: how much snow falls, in blocks, before it moves.
- **Lowest**, **Highest** and **Fade**: the band of heights it falls on,
  thinning out over so many blocks past each end.
- **Deepest**: the most snow anywhere, in snow layers (8 make a block).
- **Accumulation**: how much of the snow settles where it falls; the rest
  drifts with the wind.
- **Steepest slope**: slopes steeper than this shed their snow, which slides
  to their feet.
- **Wind ▸ Strength** (how far it carries snow, in blocks; 0 for none) and
  **From** (0° north, 90° east): the wind strips the slopes facing it and the
  open ground, and drops the snow in the lee of ridges and in hollows, where
  drifts gather.
- **Iterations**: passes of the simulation; more give smoother drifts.
- **Only on Frost and snow covers**: snow only where Frost or a cover of snow
  layers is painted.

The picture beside the settings shows the map from above with the snow it
would lay, made again as they change, with the share of the land under snow
and the deepest drift. **Let it snow** lays it in the background, one step to
undo, within the selection when there is one: each column rises by the snow's
depth in eighths of a block — Minecraft's snow layers, 1 to 8, then whole
snow blocks under them —, takes the Snow terrain, and is smoothed (whatever
the map's Smooth terrain switch says) so that the export writes the layers. A
Frost or snow cover painted there is taken off the column, its snow now part
of the ground; the water and the ice of Frost stay as they are.

## The 3D views

**3D View** shows the whole map in 3D, block by block, with the game's textures. Put it in any area; each 3D View area has its own mode, water and camera.

- Left drag turns the view, right drag moves it. Scroll flies toward the land under the cursor and back; Alt + scroll comes closer around the point the view turns about, Shift + scroll moves the view. Double-click looks at the point under the cursor.
- **W A S D** or the arrows fly over the map, **Q** and **E** go down and up (Shift: faster).
- The header picks the mode (**Terrain**, **Height**, **Slope**, **Biome**, **Relief**, **True Colour**), **Show water**, **X-ray** (the caves under the ground drawn through it, warm near the surface, cold deep down), **Objects** (the trees, bushes and rocks of the layers shown or hidden) and **Density** (the land coloured by how many objects its layers will place, from few in blue to many in red), and buttons to look at the middle of the map view or show the whole map.
- In the **Terrain** look every block shows its texture from the game and from the mods and resource packs read in the Blocks panel. **Settings ▸ Graphics ▸ Game textures in the 3D views** turns it off.
- Large maps are drawn coarser far away and every block near the eyes; **Settings ▸ Graphics** sets how much the views load (see [Settings](#settings)).

The **3D Sculpt** panel is a 3D view with the tools (below), and its **Player** mode walks the land as in the game. The **Diorama** shows the map from straight above as the game would. The **Render** side path-traces it.

![The World side's 3D workspace: the 3D View over the map](images/view3d.png)

![The 3D View with X-ray on: the caves glowing through the ground](images/xray.png)

## 3D Sculpt

[![Annotated 3D Sculpt workspace with brush settings, levels, and slope and height maps.](assets/workspaces/3d-sculpt.webp)](assets/workspaces/3d-sculpt.webp)

Use the brush settings to adjust the stroke and the slope and height maps to read the surrounding ground. [View full size](assets/workspaces/3d-sculpt.webp).

![3D Sculpt's Player mode, the keys listed along the bottom](images/player-ghost.png)

Sculpting in 3D has its own editor, **3D Sculpt** (its workspace sits next
to 3D; the 3D View's **Sculpt** button opens it): a 3D view that works with
the tool chosen in the **Tools** panel, as the map does, with its Tool
Settings and the brush the map uses: the one picked in the **Brushes**
panel, its shape (custom images too, turned as they are), size, intensity
and rotation those of **Brush Settings**. A brush picked there, or a size
set there, is 3D Sculpt's at once. Its workspace shows the Tools panel,
Tool Settings and Brush Settings on the left, the map, the Brushes panel
and the levels on the right; the top of the view names the tool in hand,
the brush with its size and intensity (a click on it shows the Brushes
panel, outlined a moment, and brings it into the workspace if it is not
there), and says what the buttons do. (A 3D Sculpt workspace laid out by an
earlier version gets these panels once; one given the Tools panel by hand
is left as it is. 3D Sculpt had a brush of its own before 0.4: it gives way
to the shared one.)

- **Raise / Lower** sculpts in 3D: it pushes the surface out (the left
  button) or in (the right one) under the brush, along its slope, a layer at
  a time — at its **Speed** in Tool Settings, in blocks a second at full
  strength —, the brush's shape setting how far each point goes: a click
  adds a bump, holding grows it, and on a cliff it builds outward, into
  overhangs, or digs caves into it.
- **Smooth** smooths the columns under the brush exactly as it does on
  the map — the same kernel, strength and passes, the brush's shape (a
  stamp too) only shaping its falloff —: a stroke in 3D and one on the map
  over the same place leave the same heights. (Smooth3d, from its
  shortcut, still melts sculpted matter in 3D, Taubin's smoothing.)
- **Paint terrain** paints the terrain chosen in Terrain or Blocks, or a
  noise of several, on the columns under the brush and on the matter
  sculpted there; the right button puts back the map's own terrain.
- The map's other brushes — Flatten, Set height, Roughen, the Gradient
  brush, Paint biome, Auto paint and Auto biome, ground covers, objects,
  tree sizes, pockets, combined layers, the Water brush, the slabs — act on
  the columns around the point under the cursor, as they do on the map;
  the Eyedropper and Flood act where
  the click points. A tool of the map only — the selections, Pan, Road,
  River, annotations — says so at the top of the view, where the left
  button then turns the view and the right one moves it.
- The brush faces the way the land faces over its whole area — the mean
  of the faces between ground and air within its radius (up to 16 blocks),
  each counting for its area, less toward the rim —, not the face of the
  block under the cursor: on a slope of one-block steps the treads and the
  risers add up to the slope, within a few degrees (2.4° at most on a
  slope of 22°, where the face under the cursor was 52° off), and it eases
  from one frame to the next as the cursor moves. Raise / Lower works
  straight up on slopes up to 25°, on the same columns as the map's Raise,
  and along the surface from 40° (cliffs, cave walls and ceilings, in
  Ghost too), blending between the two.
- The brush of the tools that act on columns (Paint terrain, Smooth and
  the map's brushes) shows exactly the columns it takes: its outline, as wide in
  blocks as on the map, laid on the land column by column, and in the
  player's view the blocks of those columns marked at any height, cliffs
  included. The tools that shape in 3D show the brush laid along the slope.
- Shift + drag moves the view, Alt + drag (or the middle button) turns it.
  The wheel flies toward the land under the cursor, down to a few blocks
  over it on a map of any size, and the view turns about that point
  from then on; Alt + wheel brings the view closer around what it turns
  about; Shift + wheel moves forward over the land; ⌘ (Ctrl) + wheel
  sizes the brush. In 3D a brush
  sculpts 64 blocks around at most: the work of a dab grows with the cube
  of its size — a dab of Raise / Lower takes about 18 ms at 64, 50 ms at
  96 and 145 ms at 128, smoothing sculpted matter 32, 200 and 310 ms —, so
  a larger brush works as 64 for the tools that shape or paint matter
  (Raise / Lower, Paint terrain), the top of the view showing its size in
  the accent and Brush Settings saying so under **Size**; the map's
  brushes go as wide as on the map.
- **Player** walks in the view as in Minecraft with Axiom, to see and
  sculpt the land at a player's height: it comes down where the middle of
  the view looks, with Minecraft's body (0.6 × 1.8 blocks, its eyes at
  1.62) and field of view.
  - It looks like the game: the textures of the game's own blocks (read
    from the Minecraft installed on the computer, as the Blocks panel
    does), the game's light — tops full, the sides along z and x darker,
    the corners blocks close in shaded as its smooth lighting does —, its
    blue sky with the square sun, its clouds at 192 and its fog, into which
    far land fades. Deep water darkens its floor a light level a block.
    The blocks the brush takes are marked in Axiom's lavender, brighter
    along the edge of what it takes, and the block under the cursor (or the
    crosshair) is outlined as the game does. Without Minecraft installed,
    the blocks keep their colours. In Height, Slope, Biome and Relief, the
    view keeps its own look.
  - The cursor free, it works as Axiom's editor: the left button looks
    around (as do the arrow keys), the right one uses the tool under the
    cursor (Alt + right: its inverse, Dig for Sculpt), the middle one picks
    the block under the cursor for Paint; Ctrl (⌘) + left turns about the
    point under the cursor, Ctrl + right moves the view, Ctrl + middle
    sizes the brush, and the wheel sets the flight speed (10 to 800 %,
    also in the bar along the bottom, which lists what the buttons and keys
    do).
  - A click lends the view the mouse, as in the game: it then looks around
    as it moves, the crosshair aims, the right button uses the tool
    (builds) and the left one its inverse (breaks); Esc gives the mouse
    back. The click that takes the mouse sculpts nothing.
  - The game's keys: W A S D walk, Ctrl (or W twice) sprints, Space jumps
    — held, again on landing —, twice quickly flies (Space up, Shift or Q
    down; twice again, or flying down to the ground, lands), Shift (or Q)
    sneaks, and a crouched player does not step off an edge; walking into a
    step of one block jumps onto it (Minecraft's Auto-Jump). Turning or
    moving the view with Ctrl flies.
  - **Ghost**, chosen at the bottom right of the view (**Walking**,
    **Flying**, **Ghost**) or with G (its key can be changed in Settings ▸
    Keyboard), flies through everything, as the game's spectator mode does:
    into caves, under overhangs, along tunnels, to sculpt or paint there.
    Space goes up, Shift (or Q) down, Ctrl faster, at the flight speed. With
    the eyes in the ground, the view sees through it — its edges darkened,
    **In the ground** said in the bar — to the air beyond and what stands in
    it, a cave's walls and floor, and the cursor and the brush work on that
    surface, the one shown. G again, or Flying, flies out of Ghost; inside
    the ground, the player comes up to the first air above.
  - Water slows and lets swim, and under it the world turns blue and dim,
    as in the game; matter sculpted over the player lifts them, and a fall
    off the map puts them back where they last stood. Player off, the
    orbit camera comes back from the player's eyes.

Sculpted matter is kept by cubes of 16 blocks, each standing for its whole
cube. A brush on the map that moves the ground under it takes it along: in
the columns the brush moves, what was sculpted keeps its shape and rides on
the ground, up or down by the whole blocks the ground moved there, and the
ground the brush moved shows around it (one step to undo with the brush's).
An overhang stays an overhang under a light Raise, Lower or Smooth beside
it, on the map or in 3D Sculpt. Strokes in 3D keep only the cubes they
change, and a map opened lets go of cubes that hold nothing of their own —
versions before made them over the whole reach of the brush, and the ground
changed under them later did not show there —, saying how many.

The brush is laid on the slope under the cursor, read from the matter
around smoothed over five blocks, so it glides over the steps of the terrain
without shaking, and a custom brush keeps its shape on a hillside. Where
that smoothed slope and the face pointed at disagree — at the foot of a
cliff, on an edge, in a hollow — the face wins, so that the brush always
works toward the eye, never into the matter or along it. The ordinary 3D View only shows the world
(its wheel works the same).

Up close, every block reads by a shade of its own and a fine line along its
edges (fading out a few dozen blocks away), and water stepping down — the
levels of a river coming down a slope — shows its falls.

How sculpted matter is stored is in [internals](internals.md#sculpted-volumes).

## Diorama

The **Diorama** (Beta) is a view of the map from straight above, as the
game would show it from the sky: block by block, with the game's textures
(and those of the mods and resource packs read in the Blocks panel), the
trees, plants, houses and other objects the export would place, the water
over the shallows showing their floor, and a light from the north-west
that makes slopes, cliffs and valleys read, each tree and building casting
its shadow to the south-east. It is an editor like the map and the 3D View,
in the **View** section of an area's chooser: put it beside the 2D map to
paint on one and see the result on the other, float it, or give it the
whole window. It follows the map's edits: the tiles changed are built
again, the old ones shown until then.

![The Diorama over a village, every block textured](images/diorama.png)

![The Diorama with Caves on: the cave floors under a height](images/diorama-caves.png)

- Drag to pan, the wheel (or a pinch) to zoom, down to the blocks,
  **Space** to come back to the middle of the map.
- **R** turns the view a quarter clockwise (**Shift + R** back): north,
  east, south or west up. The little **N** at the bottom right points north.
- **Caves** (the button in the area's header, or **C**) shows what lies
  under a height: the floors of the caves, ravines and cave lakes, in their
  own blocks, darker the further under it they lie, and the rock near
  black. **Page Up** and **Page Down** move the height a block (**Shift**: 8
  blocks); the settings have a slider. Above the land it shows the surface.
- The strip along the top says what is shown, the block under the pointer
  (x, y, z: y is the top of the block seen), how many chunks are in view and
  which way is up; its gear holds the settings: Surface or Caves, the
  height, which way is up, and how strong the relief is. The strip along the
  bottom lists the keys.

The tiles are built by the exporter itself, in the background, nearest the
view first, so what shows is what an export would write. Of each column
only what can be seen from above is kept: where something lies under air or
water, with what stands on it. A map of 1024 × 1024 blocks is built in
about a quarter of a second on an M4, and a frame takes about 2 ms of the
graphics card. From afar each tile is a small image; up close every block
is drawn with its texture. The floors of the tiles seen least recently are
let go of past 384 MB, and built again when looked at.

## True Colour

> 📷 **Screenshot:** the same part of a map side by side in two Map areas (the **Maps** workspace): Terrain mode on the left, True Colour on the right, with a resource pack read.

**True Colour** shows the map as it will look in the game once exported,
rather than in Massif's own colours: each block's top is the colour of its
texture, and grass, leaves, vines and water take the tint the game gives
them in the biome they stand in, painted or automatic, read from the game's
colour maps by the biome's temperature and downfall (or the colours the
biome names itself: badlands, swamps, warm oceans). A River biome painted
blue in Biome mode has green grass here, as in the game. Plants of covers
and the blocks of trees show in their own colours too, and so do mods'
blocks, from their textures. The 3D views have the same mode: their
textures tinted the same way.

The colours come from the blocks read in the **Blocks** panel: the game's,
then each mod, then each resource pack added there, in their order, the last
pack winning as a pack higher in the game's list does (right-click a pack's
chip to read it earlier or later). As in the game, every pack goes over every
mod, whatever the order they were added in: a pack that retextures or
recolours a mod's blocks (ArdaCraft's over Conquest Reforged's grasses and
leaves) shows its own look, its textures, models and colour maps. A pack's `colormap/grass.png` and
`foliage.png` replace the game's, and packs made for OptiFine (or MCPatcher)
give blocks colour maps of their own: `optifine/colormap/*.properties` with
their `blocks=`, read by temperature and downfall (`format=vanilla`), by
biome and height (`format=grid`, a column a biome by OptiFine's numbers, a
row a block of height from `yOffset`) or as one colour (`format=fixed`),
and `water.png`, `pine.png`, `birch.png`, `swampgrass.png`,
`swampfoliage.png`, and `palette.block.*` in `color.properties`. Packs left
as folders in an instance's `resourcepacks` are read too. A note at the
bottom left of the map names whose colours it shows. The Render side tints
its blocks with the same maps.

The view is drawn tile by tile in the background, as the other modes, and
again where the map is edited, or when other blocks or packs are read.

## Mods and resource packs

Massif paints with every block of your game, and of the mods and resource packs you give it. **File ▸ Game & mods…** (also **Game & mods…** at the top of the Blocks panel) chooses them:

- **Minecraft**: the version of the game read, found among the launcher's installs. **Choose…** reads **another version's client jar**.
- **Mods**: **Add mods…** takes the jars of mods with blocks (from the `mods` folder of your instance), the resource packs (`.zip`) that give them their textures, and the lists of blocks made for WorldPainter (`.csv`) that give their states: their blocks are painted and exported like the game's. **Instance…** takes a modpack's instance at once (CurseForge, Prism, Modrinth…): its mods with blocks, its resource packs and its lists of blocks, over the version of the game it plays.
- Each mod or pack is a chip: right-click it to **Read earlier** or **Read later (wins)**, as a pack higher in the game's list wins, or **Remove** it.

Once read, a mod's blocks are in the **Blocks** panel and among the terrains, its biomes in the **Biomes** panel (see [Biomes of mods and data packs](#biomes-of-mods-and-data-packs)), its textures in the 3D views and in [True Colour](#true-colour), and its structures among the Structures tool's. A resource pack read changes the textures and colours Massif shows, over the mods' own. The export writes mods' blocks by their ids: open the world with those mods installed. The export dialog warns when the version chosen does not suit the map's mods.

![File ▸ Game & mods…: the version of Minecraft found, mods and instances](images/game-and-mods.png)

### Block palettes

A block palette says what the terrains of a map are made of: the grass,
rock, sand, gravel and snow the generator lays by height and slope (and the
basalt and blackstone along a sea of lava), the other natural terrains (dirt,
podzol, moss, mud, clay, ice…), and the rock under them, where caves are
carved and ores laid. Lakes, Rivers, the automatic paint and the brushes
paint the same terrains, so their beaches and beds follow the palette too.
A terrain a palette leaves out is the game's own; terrains of your own are
never touched.

![The block palettes window](images/palettes.png)

**Vanilla**, the game's blocks, comes with Massif. A mod may have a palette
of its blocks too, offered only once that mod's blocks are read (see
[Blocks from mods](#blocks-from-mods)): the menus list Vanilla, the palettes
of the mods read for this map, and your own. Massif knows no mod in its
code: a mod's palette is data, a *mod profile* shipped with Massif
(`crates/massif-core/assets/mods/<namespace>/`: the mod's name, the version
of the game it is made for, and its palette), and any mod can have one.

Conquest Reforged's profile, for one, has a palette made from the lists
Conquest writes for WorldPainter (`conquest_wp.csv`): its grass blocks,
loamy dirt and brown earth, forest floors for podzol, sphagnum moss, muds,
gray and mossy limestones, small stones and boulders on rocky ground, tan
sand on the beaches, gravels and pebbles in river and lake beds, snow over
icy limestone, glacier ice and weathered basalt, several blocks sharing
each surface in patches. The rock under it stays the game's stone, so the
ores are the game's.

**Choosing one.** **Blocks** in the New map settings picks the palette of a
new map; its preview shows it, and the line under the settings names the
version it is for: the one its mod's profile names. When a palette uses
the blocks of a mod not read yet but installed in an instance here,
**Create map** (and **Blocks** in the Info panel) reads that instance as the
Blocks panel's **Use** does. Later, **Blocks** in the Info panel (World)
makes the whole map of another palette at once, in one step to undo: nothing
is painted again, the terrains are only made of other blocks. The map keeps
its palette in its file.

**Your own.** **Edit palettes…** at the bottom of either menu opens the
palettes. Vanilla and the palettes of the mods read are shown as they are: **Duplicate** makes one of
your own from any of them, **From this map** one of the blocks of the open
map's terrains. Pick a terrain (the generator's first, then the others, a
dot marking those the palette sets), then each of its layers from the
surface down: how deep it goes, whether its blocks lie in patches and how
large, and its blocks, each with a weight (its share is shown beside it).
Click a layer to pick it, then a block in the grid under it (the game's and
the mods' read in Blocks, with a search) or type any id with its properties
(`mod:block[axis=x]`), for a block of a mod or resource pack not read here.
**+ Layer** adds one under the others, **Back to the game's** leaves the
terrain to the game. **Rock under the terrains** sets the rock above and
under Y 0. Your palettes are kept with Massif's settings; one changed since
it was given to a map is given again by picking it in **Blocks**.

### Blocks from mods

Massif paints with the blocks of any mod as with the game's: Conquest
Reforged's 17,000 blocks, Chipped, Macaw's and the like, made for Forge,
NeoForge, Fabric or Quilt. No mod is treated apart: what Massif reads of
one (its jar, its instance, its list for WorldPainter) it reads of all.

> 📷 **Screenshot:** the Blocks panel with a mod's blocks listed under its name, the search finding one, **All shapes** on.

**Game & mods.** The Blocks panel lists the blocks to paint with; which
game and mods they come from is set in **File ▸ Game & mods…**, also opened
by the button at the top of the Blocks panel: the version of Minecraft
(found beside each instance and wherever the official launcher, CurseForge
on any drive, the Modrinth App, Prism, MultiMC, ATLauncher or GDLauncher
keep it; **Choose…** for another jar), the mods and the instances. The
Terrain panel is the map's own terrains.

**In one step, from the modpack's instance.** Massif looks through the
instances installed here (CurseForge, Prism, MultiMC, Modrinth or the
official launcher, on any drive) for mods with blocks, whatever the mods.
Those whose blocks are not read yet are listed in **File ▸ Game & mods…**
(also the button at the top of the Blocks panel) under
**Mods with blocks in your Minecraft instances**: each instance on a row,
with its loader, the version of the game it plays and how many mods with
blocks it has, and **Use** reads it. Click a row to unfold its mods, one a
line, by name: the name the mod gives itself (in `mods.toml`,
`fabric.mod.json` or `mcmod.info`), else its jar's file name without its
versions and loader, then its id and how many blocks it has; hover a mod
for its jar's file name. Once there are many, a search above the list finds
instances and mods by name, id or file name, and unfolds the instances on
the mods it finds. Instances with lists of blocks for WorldPainter
come first. **Instance…** reads any other modpack: choose its folder (the one with
`mods` in it, or the instance's own). Reading an instance:

- reads its mods that have blocks (the others, Sodium, Iris and the like,
  are left out), its list of blocks made for WorldPainter (see below), and
  the resource packs it has turned on in the game;
- reads the blocks its KubeJS scripts make, as a mod named KubeJS: those
  its startup scripts register (`StartupEvents.registry('block', …)`, or
  `onEvent('block.registry', …)` before KubeJS 6), with the name, type
  (a cube, a slab, stairs, a fence, a wall, a crop) and textures the script
  gives them (`displayName`, `textureAll`, `texture('up', …)`, `model`), the
  textures from its `kubejs/assets` folder, as KubeJS itself makes them when
  the game starts; and the states, models and textures written in that
  folder, which win over what Massif makes of a script. Massif reads the
  scripts, it does not run them: a block whose id is built by code (in a
  loop, from a variable) is not known;
- reads them over the version of the game the instance plays, when that version is installed here;
  otherwise over the version read now, and the panel says so.

**One file at a time.** **Add mods…** takes the jars of mods (from the
`mods` folder of an instance), resource packs (`.zip`, read after the mods:
a pack's textures replace its mod's) and lists of blocks for WorldPainter
(`.csv`). Each shows as a chip; clicking it takes it out. They are
remembered, and read again each time Massif starts.

**Connected textures.** Fusion draws a texture across blocks
(continuous), by position (random) or joined to the blocks beside it
(connecting), from a sheet of tiles that its `.png.mcmeta` describes;
OptiFine (and Continuity, which reads its files) does the same from
`optifine/ctm/**/*.properties`: `method=ctm` (47 tiles), `ctm_compact`,
`horizontal`, `vertical`, `repeat` and `random`, for the textures
(`matchTiles`) or blocks (`matchBlocks`) they name, on the faces they name.
The Blocks panel shows each block with the tile of a block alone, at the
size of a block. In the True Colour view, close up, each top shows the tiles
its neighbours give it: a block joins those of the same block at the same
height beside it.

**The blocks.** A mod's blocks come after the game's, in its own creative
tabs when it gives them (`assets/<namespace>/groups/*.txt`: Conquest
Reforged's 36, Cobble and Brick, Advanced Masonry, Roofing…), with their real textures and names, else by
family. A family that a block's name does not tell is read from how the
block is drawn: a cross of two planes, a crop or leaves make a plant (a
mod's `amaranth`, `cattail`, `barley`), planks or a log make wood (its
fences, doors, signs), bricks, cobble or stone a building block. A plant
drawn with a model of its own (Regions Unexplored's duckweed, clover, leaf
litter, ivy, tassels) is told by the game's tags of plants the mod puts it
in (flowers, saplings, leaves, crops, what trees grow through, what is
climbed), a pot of it is not. A mod's blocks of ground (`silt_grass_block`,
`peat_dirt`, `volcanic_ash`, and those not quite whole, `peat_dirt_path`,
`silt_farmland`) go with the soils. Whole blocks, plants and the ground's
blocks show at first; **All shapes** adds every other shape (a
mod's tabs list each block once, and its shapes follow it): slabs, stairs,
arches, capitals, vertical slabs, fences, doors… The search looks through
every block and shape, ticked or not. A whole
block becomes a terrain and any other (a plant, a shape) a ground cover, as
the game's do. The terrains of a mod mix, lay in patterns and beds, and
serve the automatic paint like any other. A mod's tall plants (its lilacs,
peonies…) grow in their two halves, and its plants grow
on its own ground: its grass, moss, forest floors, dirts, soils, silts,
muds and peat count as soil.

**Any block by its id.** For a block nothing in the panel finds, type its id
in the field under the search, with its properties if it has any:
`regions_unexplored:redwood_log[axis=x]`. **As terrain** paints it as the
ground, **As cover** grows it over the ground. A block among those read
keeps its textures; another is written as typed and drawn in grey, and the
game needs its mod to have it. A mistyped id or property says what is wrong,
and a property a mod's list for WorldPainter does not give is refused.

**A mod's list for WorldPainter.** Some mods write a list of their blocks
for WorldPainter, in WorldPainter's custom-materials format (`<mod>_wp.csv`,
which WorldPainter reads from the `materials` folder of its settings:
Conquest Reforged writes `conquest_wp.csv`), and some, beside it, the list
of every block the game registers (`blocks-pls.txt`). Massif reads them for
any mod, from the instance or from WorldPainter's folder, so a WorldPainter
user finds the same blocks: the list gives every property of every block
and the values it takes; the registry's list tells the real blocks from
block files of the jar the game never registers (79 in Conquest's), which
Massif leaves out. Objects (schematics) made of a mod's shaped blocks keep their states valid when they are
turned or mirrored: an arch lying north–south turns east–west, left and
right shapes swap in a mirror.

**Mod profiles: a palette for any mod.** What Massif knows of a mod beyond
its jar is data, one folder per mod in `crates/massif-core/assets/mods/`,
named after the namespace of its blocks. Massif ships profiles for Conquest
Reforged (`conquest`), Biomes O' Plenty (`biomesoplenty`), Regions
Unexplored (`regions_unexplored`) and Oh The Biomes We've Gone
(`biomeswevegone`); a profile is used only once its mod's blocks are read.
Anyone can write one for another mod, with no code:

- `profile.json`: the mod's name as its jar gives it, its namespace, and the
  version of the game it is made for:

  ```json
  { "name": "Regions Unexplored", "namespace": "regions_unexplored", "minecraft": "1.21.1" }
  ```

- `biomes` in `profile.json` (optional): the biome each of the mod's blocks
  calls for. A terrain made of that block (from the Blocks panel) takes it
  where the biome is left on automatic: **Auto biome**, the map's Biome
  mode and the export give it that biome, as a desert's sand gives the
  desert. Regions Unexplored's peat mud is its bayou, its silt podzol its
  autumnal maple forest:

  ```json
  { "name": "Regions Unexplored", "namespace": "regions_unexplored", "minecraft": "1.21.1",
    "biomes": { "regions_unexplored:peat_mud": "regions_unexplored:bayou" } }
  ```

- `palette.json` (optional): a block palette of the mod's blocks, the
  game's mixed in as you like. `terrains` maps a built-in terrain (`Grass`,
  `Dirt`, `Coarse Dirt`, `Podzol`, `Moss`, `Mud`, `Clay`, `Rocky`, `Sand`,
  `Red Sand`, `Gravel`, `Snow`, `Ice`, `Basalt`…) to its layers from the
  surface down: blocks with their weights, a depth range in blocks, and
  `patches`, how wide the patches of its blocks are. `colors` gives each
  mod block's colour, for the map before the game is read. The easiest way
  to write one: make the palette in **Edit palettes…** with the mod read,
  then copy it from Massif's settings.

  ```json
  { "name": "Regions Unexplored", "colors": { "regions_unexplored:peat_mud": [67, 56, 45] },
    "terrains": { "Mud": { "strata": [ { "blocks": [["minecraft:mud", 2], ["regions_unexplored:peat_mud", 2]],
                                         "depth": [2, 4], "patches": 4 } ] } } }
  ```

Rebuild Massif and the profile is in. `MASSIF_MOD_JARS=<the mod's jar>
cargo test -p massif-assets mod_profiles_match_their_mods` checks it against
the mod: its name, every block a whole block with its texture and a state the
mod gives, and the colours (it prints the right ones). The gradient brush's
presets need nothing per mod: they paint the built-in terrains, which a
mod's palette makes of its blocks.

**Exporting for the mod.** A mod is made for one version of the game:
Conquest Reforged's for Minecraft 1.20.1, as its jar says. When the map uses
a mod's blocks, **Export** picks that version and the instance's `saves` folder,
and lists each mod under **Mods** with how many of its blocks the map
uses. It warns, in red, when another version is chosen (the game may not
load the mod's blocks; **Export for** that version puts it back), when a block
comes from a mod Massif has not read, and when a state is none of the
game's by the mod's list. Blocks are written by their own ids and states
(`conquest:travertine_brick_small_arch[direction=eastwest,waterlogged=false]`),
so open the world with the same mods in the game. Before Minecraft 1.13,
where blocks are numbered, a mod's blocks are written as stone.

**Checked against the game.** A map painted with 150 of Conquest
Reforged's blocks (terrains, mixes, patterns, plants, and objects turned
every way) is exported for 1.20.1 and read back with an independent reader:
every block state written is one of the 726,881 the game registers.
Conquest Reforged's 222 MB jar is read with Minecraft 1.20.1 in about half
a second, and its blocks take about 70 MB of memory.

## Real blocks

Massif reads the blocks of the game from your own installation: the client
jar of a released version, found where the official launcher, CurseForge,
Prism or the Modrinth App keep them (on any drive, on Windows, macOS and
Linux), or chosen by hand. Nothing of the game is
copied into Massif or the repository; the jar is read in place, in a
fraction of a second, every time Massif starts.

![The Blocks panel](images/blocks-panel.png)

- Every block state file gives a block; its model, followed through its
  parents, gives the textures of its top, sides and bottom; grass, leaves
  and water are tinted as in the game (plains colours).
- The blocks and terrains of the open world take the real colours of their
  textures, on the map and in the 3D view.
- The **Terrain** panel draws each terrain with its main block's textures.
- The **Blocks** panel lists every block of the game by family (soil,
  stone, ores, wood, plants, building blocks, coloured blocks, Nether,
  End), with a search field and the version to read from. Clicking a whole
  block makes it a terrain of the world (grass, podzol and mycelium over
  dirt, as in the game; other blocks a few deep over the natural rock) and
  paints it. Clicking any other block (a flower, grass, a carpet, coral…)
  makes it a ground cover that grows it, and paints that.
- **Mods** in the Blocks panel adds the blocks of mods: see
  [Blocks from mods](#blocks-from-mods).

## Importing

Massif brings in what you already have:

- **A WorldPainter world** (`.world`): **File ▸ Import WorldPainter World…**, the home screen, or drop it on the window. Its terrains, heights, water, biomes, layers (covers, trees, custom objects, pockets, annotations, tunnels and floating layers), selections and settings come in; **File ▸ Import Report…** says what came in, what was left out, and offers to make the ground's own the covers of layer blocks some maps (Conquest Reforged's) lay. Where each of WorldPainter's tools went: [Coming from WorldPainter](coming-from-worldpainter.md); its layers: [WorldPainter's layers in Massif](#worldpainters-layers-in-massif).
- **A Minecraft Java world**: **File ▸ Import Minecraft World…**: its land, water, biomes and plants, and what stands on it, from any version Massif exports. See [Importing a Minecraft world](import-minecraft.md).
- **A height map**: a greyscale picture made into a new world, or exported back. See [Height maps](#height-maps).
- **A real place**: the relief of a place on the Earth, as a new world or into the map. See [Real places](#real-places).
- **Masks**: images painted onto the map as annotations, a layer, a terrain, a biome, water or the height. See [Import mask](#import-mask).
- **Reference images**: a picture over or under the map to trace over, never exported. See [Reference images](#reference-images).
- **A copy or a region of this map**, edited elsewhere: **File ▸ Merge Massif Map…**. See [Working together](#working-together).
- **Packs**: **File ▸ Import Pack…** adds a `.massifpack` someone shared. See [Packs](#packs).

![The Import Report of a WorldPainter world](images/import-report.png)

## Height maps

- **File ▸ New World from Height Map…** makes a world of a greyscale image
  — a 16-bit PNG from Gaea, World Machine or an image editor (JPEG, and raw
  square `.r16` / `.r32` are read too): what height black and white stand
  for, how many blocks a pixel covers, the sea's level; terrains are painted
  from height and slope. The home screen offers it too.
- **File ▸ Share Image…** draws pictures of the map ready to post on
  Discord or Reddit, and shows them as a carousel to pick from (the arrows,
  the thumbnails or ← →): **Diorama**, the whole map as a block with its
  sides, on a soft background; **Relief**, a low view of the map's most
  dramatic relief, the sun low and to the side; **Map**, the map from above
  in the colours the game gives its blocks (in its terrains' colours when
  the game's blocks are not read),
  with a scale and a north arrow; **Map and relief**, both side by side. A
  band along the bottom gives the map's name, its size and the version of
  Minecraft, with Massif's mark; it can be left out, light or dark. The
  pictures are 16:9 (1920 × 1080), square (1080 × 1080) or 4K, drawn in a
  few seconds from a copy of the map; **Copy** puts the one picked on the
  clipboard, **Save PNG…** saves it.
- **File ▸ Export Height Map…** saves the map's surface (with its levels and
  what was sculpted) as a 16-bit PNG, black the bottom of the world, white
  its top.
- In a graph, the **File input** node brings in a file as relief: a height
  map (stretched over the terrain, black and white set to any height), a
  WorldPainter `.world` or a `.massif` (fitted to the terrain, or read at
  its own place). Its Height, Water and Colour outputs work like the Map
  node's, and the file is read again when it changes. The arrow beside
  the Generator Viewer's **Apply to map** saves what a graph makes as a 16-bit PNG height
  map (**Height map: Save…**), a block a sample, without writing it into
  the map.

![The home screen's From a height map page with a 16-bit PNG chosen](images/from-height-map.png)

## Real places

A world, or a part of one, from the relief of a real place on the Earth.

> 📷 **Screenshot:** File ▸ New World from a Real Place… with a mountain range found by name, the orange outline of the map on the small map, and the picture of the ground it makes below.

- **File ▸ New World from a Real Place…** (also **From a real place** on the
  home screen) makes a new world of it; **Edit ▸ Relief of a Real Place…**
  writes it into the map open, into the selection (by its weight, fading
  along a feathered edge) or over the whole map. Either writes the
  heights, puts the sea under the sea level (and takes it off the land
  above it) and paints the ground with the automatic paint; in the map it
  is one step to undo.
- **Terrains.** Into the map, the **Map** section's **Terrains** says
  what becomes of the paint. **Repaint** (the default) paints the new land
  from the real place, as a new world is (sand along the water only, rock
  on cliffs, snow above the snow line), and takes off the biomes, ground
  covers and object layers painted there, so the old map's sand pits and
  biomes do not lie at random over the new relief. **Keep** writes only
  the heights and the water, the terrains and everything painted staying.
- **Blocks.** The block palette, as in the New map dialog: Vanilla, your
  own palettes, and those of the mods whose blocks are read (Conquest
  Reforged's, say). A new world is made of it. Into the map it starts as
  the map's palette; another one switches the map's palette with the
  relief, its terrains made of that palette's blocks all over the map, in
  the same step to undo.
- **Map size.** Over the whole map (no selection), the dialog's **Map**
  section says plainly that the map's land is replaced, and its **Size**
  can be kept or changed: **Change it** takes a new width and depth in
  blocks, made whole tiles of 128 once typed, up to 32,768 a side (the
  chain between them keeps them equal until it is clicked, as in every
  size of a map: New map, a new world of a real place). The
  map is cut or grown around its middle, and the place's heights fill all
  of it, new ground included. The other levels and the sculpted ground
  (and, with **Keep**, the painted biomes, covers and objects) stay where
  the new size covers the map, and go outside it. A note gives the size, where it starts and
  about how much memory it takes, and warns of a map that will be slow
  (over 16,384 blocks a side) or that may not fit in the computer's
  memory. Size and relief are one step: undo puts the map back as it was,
  size and all. (In the Terrain Generator, the graph's **Terrain size**
  in Node Properties sets how much of the world the terrain covers.)
- **Source.** **Terrain Tiles** are the heights of the whole Earth, about
  30 m apart (finer where better data exists), ocean floors included,
  downloaded as needed. **A file** reads one you downloaded: a GeoTIFF
  (16-bit or 32-bit, its pixel size read from its tags), a 16-bit PNG or
  a raw `.r16` / `.r32`, for which you say how many metres a pixel covers
  and the heights black and white stand for. A file is centred on the map.
- **Place.** Type a place's name and press Enter to search it (with
  OpenStreetMap's Nominatim; only when you search), or type its latitude
  and longitude (`45.83, 6.86`, `33.9S 18.4E`), or its north, south,
  west and east bounds. The small map beside shows the Earth's relief,
  the area the map covers outlined in orange: drag it to move, scroll to
  zoom.
- **Scale.** **Blocks a metre** is the horizontal scale: 1 makes a block
  a metre, 0.1 ten metres. A new world has the **Width** and **Depth**
  you give it, up to 32,768 blocks a side, the area following (rounded up
  to whole tiles of 128 blocks, each read from the data); in a map, the
  area is the map's (or the selection's) and the scale sets how much of
  the Earth it covers. A world of 16,384 blocks a side takes about half a
  minute and 650 MB of memory to make, one of 32,768 about two minutes
  and 600 MB; past some 30 km the data is read a little coarser (at most
  1,024 tiles), its pixels then larger than a block.
- **Clean data.** Heights no place on the Earth has (the gaps of the data)
  and lone spikes and pits a pixel wide are dropped, filled from the
  heights around them, meeting them with no step; a ridge or a gully as
  narrow stays. A tile the server does not send is asked for again, then
  made from the tile of a coarser zoom over it, bent at its edges to meet
  its neighbours; the status bar and the log say how many tiles could not
  be had.
- **Coasts.** The data near the sea comes from sources of every
  resolution, and swings either side of 0 m along a coast: the coastline
  follows a smooth line through it, not the data's squares, so low coasts
  and lagoons have no square pools. Where the data's fine part sits in
  rectangles over a coarse fill (as along the coasts of Sweden), the fill
  near them takes their coast, and the seams along the rectangles are
  smoothed away. The sea floor deepens from the shore as far as the
  bathymetry says, smoothed over its own cells so that no rectangle of its
  sources shows (some 30 m deep at most where nothing says, far from where
  something does). A lake the data gives flat over the sea's level is
  water at its own level, its floor deepening from its shores.
- **Paint and biomes.** The ground is painted from the real place: sand
  along the water only (a beach of about 30 m, no more than 12 blocks),
  rock on steep slopes, snow above the snow line of the latitude (about
  3,000 m in the Alps, 5,000 m in the tropics), not by the heights of
  blocks that the scale and the exaggeration move. The biomes follow: the
  sea is ocean (deep ocean where deep), warm in the tropics and cold toward
  the poles; lakes and rivers are river; the sand along the water beach.
- **Heights.** At an **Exaggeration** of 1 the relief keeps the same scale
  upward as across. **Sea level** is the map's, and **Real height there**
  the real height standing at it (0, the sea's; for a place inland, its
  valley floor, the land under it then flooded). The ground keeps between
  **Lowest** and **Highest** (Minecraft's −64 to 320, a taller world's, or
  the map's own); **Squeeze into the range** brings what would go past it
  into it, above and under the sea apart, so the summit stands at the top
  and the sea where it is, rather than cutting it off.
- **Preview.** The picture under the small map shows the ground it makes,
  and the line under the heights the real heights it reads and the
  heights of blocks they become.
- **Downloads.** Tiles come from Amazon's open data, a few at a time, in
  the background with their progress and a Cancel, and are kept on disk
  (`~/Library/Caches/Massif`, `%LOCALAPPDATA%\Massif\cache`,
  `~/.cache/massif`; up to 2 GB, the oldest dropped first): a place seen
  once works offline. Massif sends nothing about you, only its name and
  version. Offline, a place not seen before cannot be had, and Massif says
  so rather than writing a hole.
- **Credits.** Elevation: Terrain Tiles by Mapzen (Tilezen), on AWS Open
  Data. ArcticDEM terrain data DEM(s) were created from DigitalGlobe, Inc.,
  imagery and funded under National Science Foundation awards 1043681,
  1559691, and 1542736; Australia terrain data © Commonwealth of Australia
  (Geoscience Australia) 2017; Austria terrain data © offene Daten
  Österreichs – Digitales Geländemodell (DGM) Österreich; Canada terrain
  data contains information licensed under the Open Government Licence –
  Canada; Europe terrain data produced using Copernicus data and
  information funded by the European Union - EU-DEM layers; Global ETOPO1
  terrain data U.S. National Oceanic and Atmospheric Administration;
  Mexico terrain data source: INEGI, Continental relief, 2016; New Zealand
  terrain data Copyright 2011 Crown copyright (c) Land Information New
  Zealand and the New Zealand Government (All rights reserved); Norway
  terrain data © Kartverket; United Kingdom terrain data © Environment
  Agency copyright and/or database right 2015. All rights reserved; United
  States 3DEP (formerly NED) and global GMTED2010 and SRTM terrain data
  courtesy of the U.S. Geological Survey. Place search: Nominatim, ©
  OpenStreetMap contributors (ODbL). When you share a map made from them,
  credit them too.

## Import mask

**Edit ▸ Import mask…** paints an image onto the map, as WorldPainter's
Import mask does: a plan drawn in an image editor, a forest mask from
another tool, a biome map, a heightmap of a part of the map. Massif reads
PNG (8 or 16 bits, grey, colour or palette, with or without alpha), TIFF
(8, 16 or 32-bit integers or floats, in strips or tiles), BMP (1 to 32
bits) and JPEG. The image is read once in the background, for the range of
its values and a picture of it, then the dialog opens.

![Edit ▸ Import Mask… with two pictures: what the first paints, its levels and threshold](images/import-mask.png)

**What it paints**:

- **Annotations**: a colour image gives each pixel the nearest of the 15
  colours of the annotations (**Nearest of the 15**), or **One colour**
  wherever the image is on; a grey image paints the colour picked.
- **Layer**: any ground cover, tree layer (as trees or under the trees),
  layer of pockets, or a tree size.
- **Terrain**, **Biome**: the one picked.
- **Water**: water up to the surface set (the sea level to start with).
- **Height**: the ground, as a heightmap: black gives the **Height of
  black** (the offset), white the **Height of white**, the greys those
  between (the scale is the difference). It works on the map as it is,
  where a new world from a height map makes a new one.

**Where it lands**: **At the world's origin** puts its top left corner on
x 0, z 0, a block a pixel; **Fit to the map** makes it as large as the map
lets it, in its middle (an image of the map's size lands on it, block for
block). **Scale** and **Offset** move it from there: the scale about the
image's middle, from an eighth of the scale that fits the map to four times
it (100 % is a block a pixel), the offsets a block a step from where the
image just touches the map's west or north edge to where it touches its
east or south one; any value typed in is kept exactly. Under them, the map
small, the image over it where it lands, coloured as it will paint: drag
the image there to move it, or use the arrow keys, a block a press (with
Shift, 16).
Larger than a block a pixel, the greys between pixels are blended; smaller,
each block takes the average of its pixels (colours: the middle one).

**How its values are read**: **On from a threshold** paints where the grey
is at or over the threshold; **Strength, scattered** paints that share of
the columns (a grey of 30 % about 30 % of them), as a spray does, the same
columns every time. The **Range** runs from black to white, or from the
image's lowest value to its highest (floating point images start there).
**Invert** turns it over; **Black leaves the map as it is** skips black
pixels (and a JPEG's near-blacks), so that only what is drawn counts.
Transparent pixels always leave the map as it is.

**With what is there**: **Replace** lets the image alone decide where it
lies, rubbing out the layer where it is off (a terrain stays, a column
always has one); **Add** and **Max** paint where it is on and keep what was
painted (for a layer, which is in a column or not, the two are the same);
**Min** keeps the layer only where it was painted and the image is on. For
the height, they add to the ground, raise it to the image or lower it to
the image. Only the tiles the map has are painted, within the selection
when there is one; a **locked** layer keeps its columns.

**Several images** go in at once: pick them together, or **Add images…** in
the dialog. Each has its tab, with what it paints and how; they all land in
the same place, and **Import** puts them all in as one step to undo, the
heightmaps first so that the masks after them read the new ground. The ✕
beside the tabs leaves one out.

**Blend the heights' edges over** (shown once an image paints the height)
fades a heightmap of a part of the map into the land around it: over that
many blocks in from the image's edges the ground goes smoothly from its own
height to the image's, so that there is no cliff at the seam (16 blocks to
start with; 0 for none).

**Import** runs in the background, with its progress and a **Cancel**, and
is one step to undo. The image streams through: a band of 128 rows of the
map at a time, the next rows decoded meanwhile, never the whole image in
memory (JPEG and compressed BMP excepted, decoded whole at 8 bits). On an
M4, a 16,384 × 16,384 greyscale PNG onto a map of the same size is read in
1.7 s and painted in 2.2 to 2.6 s, the whole process below 0.5 GB.

**Keep the image as a reference image** (on to start with) leaves the image
on the 2D map as a [reference image](#reference-images), exactly where it
lands (its middle, its scale), at the opacity set, so that you see the
source under what it painted. It is kept with the map and never exported;
the map's Reference images button shows, fades, moves or removes it like
any other. With several images, the one shown when **Import** is pressed
stays.

### The mask window

Where the window has room (1100 × 640 points or more), Import mask opens as
the **mask window**: the map large on the left, the settings on the right,
**Import** and **Cancel** under them. **Smaller** goes back to the dialog,
and **Mask window** in the dialog comes back to it.

Over the map, what it shows:

- **Over the map**: the image in the colour it paints, over the map.
- **Mask**: the image alone, in grey as its values are read (levels,
  midtones, blur and threshold applied), the map dimmed under it.
- **Before**: what the map has now of what the image paints: its terrains,
  its biomes, the layer, its water or its heights.
- **After**: the same, as the import leaves it.
- **Before | After**: before on the left of a line, after on its right;
  drag the line.
- **Changes**: after, the columns that change lit, the rest dimmed.

**Overlay** sets how strongly the image, or what it paints, lies over the
map, from 0 % (the map alone) to 100 %. The left button drags the image, the
wheel (or a pinch) zooms about the pointer, the right or middle button pans,
**Fit** shows the map and the image whole again; the arrow keys move the
image a block, 16 with Shift. Under the canvas, the window says about how
many columns the import changes, and what share of the map.

**What each value paints** is the legend: from what grey the target is
painted and what happens under it (rubbed out or left as it is, as **With
what is there** says), how dense a scattered layer grows, the heights black
and white stand for, or, for a colour image, each annotation colour it
holds and how much of it.

**Levels** shows the image's histogram, its greys from black to white, with
the curve its values go through: **Black point** and **White point** (also
dragged on the histogram) stretch the greys between them over the whole
range, **Midtones** lightens them (above 1) or darkens them, and **Blur**
softens the image over that many blocks before a threshold cuts it, for
smooth edges, or smooths a heightmap. **Reset the levels** puts them back.
They apply to the import itself, as the views show it.

The views are worked out from the map read once with the image (a column
every few blocks, 512 across at most) and the image's picture, whatever
their size: moving, scaling or changing a setting redraws them at once.
The import itself reads every pixel and paints every column.

### A region out and back

**File ▸ Export Region…** writes a part of the map as images of 1 pixel a
block, to work on in another tool (World Machine, Gaea, an image editor)
and bring back: the **selection** (its bounding rectangle), **the whole
map**, or **these blocks** (a top left corner and a size). Into a folder of
its own, named after the map and the region's corner, go `height.png` (16
bits: black the world's floor, white its top), one image for each terrain
and layer found there (`terrain Grass.png`, `cover Tall grass.png`, `trees
Big oaks.png`, `under the trees …`, `pockets …`, `tree size medium.png`:
white where it is, black elsewhere), and on demand one for each biome, the
water and the annotations (in their colours, when there are some). Beside
them, `region.txt` says where the region lies and which heights black and
white stand for:

```text
x = -512
z = -512
width = 1024
depth = 1024
black = -64
white = 320
```

**Edit ▸ Import mask…**, given these images (all of them at once, or
those that came back from the other tool), reads the `region.txt` beside
them and puts them back in their place (**Where region.txt says**; an image
made larger or smaller meanwhile is scaled to fit the region), each painting
what its name says, the heights between the black and white of the file.
Only a name the other tool kept is understood; any other image gets a
target to pick, as usual. Rows are written as they are read, a row of tiles
at a time: a region of 16,384 × 16,384 blocks, its heightmap and a terrain,
is written in about 2 s on an M4.

With **Review it when it comes back** on (as it is unless turned off),
Export Region also keeps a version of the map and writes `zone.json` beside
the images. Brought back, the images are then painted onto that version and
what they change is shown on the map to accept or reject, as a contribution:
see [A zone through another tool](versioning.md#a-zone-through-another-tool).

**Massif map** (rather than Images) writes the region as a `.massif` of its own instead, with
a border of whole tiles around it, for someone to work on in Massif; File ▸
Merge Massif Map… brings their work back: see
[A region of the map for someone else](versioning.md#a-region-of-the-map-for-someone-else).

### A part of the map as a schematic

**File ▸ Export Selection as Schematic…** (or **Export as Schematic…** under
the selection in a selection tool's settings) writes the selected columns as
a schematic, to paste a mountain, a village or a forest with WorldEdit or to
build it from with Litematica. It holds the blocks the export to Minecraft
writes there, built the same way: the ground and its strata, the water as
the export settles it, the caves, the covers' plants, the trees and other
objects, the structures with their chests and signs.

- **Format**: **WorldEdit .schem** (Sponge schematic version 3: WorldEdit
  7.3 and later, FAWE, Axiom), **WorldEdit 7.2 .schem** (version 2, biomes
  by the column) or **Litematica** (`.litematic`, no biomes).
- **Down to**: a number of blocks **under the lowest ground** of the
  selection (8 to begin with), or **the world's floor**. The box goes up to
  the highest block of the selection.
- **Trees and objects**, **Structures**, **Biomes**: left out when off.
- **Keep the air**: pasted, the air clears what stands there (WorldEdit's
  `//paste -a` leaves it out). Off, the empty blocks are structure voids,
  which Litematica and Massif leave as they are; WorldEdit places them as
  blocks, so keep the air for it.
- **Origin**: the middle of the box's foot, or its corner: where the box
  goes from the block a player stands on when pasting it.

The blocks are built in the background as the dialog opens, again when a
setting they depend on changes (**Stop** stops it), and the file is made
from them, so that before anything is saved the dialog shows the box's size
and heights, how many blocks it holds, the file's size, the **materials**
(each block by its name, the most first, with its icon) and the **trees and
objects** of each object layer. **Copy as text** copies the size and the
materials, a line each. A box may hold 64 million blocks; past 16 million
it is said to be large, as WorldEdit and Litematica are slow to paste it
and Massif's own import of objects refuses it. A selection of 256 × 256
columns of hills with trees is built and written in about 1 s.

In an object layer's list of objects, **Export as .schem…** writes the
objects picked as Sponge schematics, their anchor the origin: one to a
file, several to a folder, a file each. **Grow trees…** writes its trees the
same way.

## New worlds and export

A map is made on the home screen and written as a Minecraft world by the export. This page has the settings of a new map, every option of the export dialog, the versions of the game it writes, and the world rules.

### A new map

A new map (the home screen) takes a name, a size (width × depth, in
multiples of 128 blocks, up to 65,536: type any number, and once you press
Enter or leave the field it becomes the nearest multiple, a note under it
saying so — 5000 becomes 4,992), its **height** — Minecraft's own (−64 to 320) or
taller, up to 512, 1024 or 2032 (the highest Minecraft allows), and as deep
as −2032 — its land — a
topography (flat, plains, hills, mountains, islands) with its relief height
(up to fifteen times the landform's in a tall world; peaks close in on the
top of the world rather than being cut flat), feature size and ground
level, or a template of the terrain generator (alpine range, continents,
archipelago…), whose map then opens on the Terrain side with its nodes —
the water level, lava instead of water (opaque and glowing, its shores
scorched basalt and blackstone, never sand), beaches (sand along the
water, up to six blocks from it: low ground farther inland stays grass), automatic terrains
(grass, sand, rock and snow by height and slope) or a single one, and a
seed; a preview of the whole map updates as you change them. A map made
from a height map takes a height too.

### The export dialog

**File ▸ Export to Minecraft…** (⌘E) opens the export dialog. Its three tabs, down its right side, set the same export; switch between them before choosing **Export**.

#### General

- **Where to save it**: the **Saves folder** (your Minecraft saves, found by itself; **Choose the folder** for another) and the **Name** of the world in Minecraft.
- **Minecraft**: the version to write for, from 1.7.10 to 26.3 and every newer game installed (the installed ones are marked). A line says what that version's chunks hold (Y −64 to 320 for today's chunks, 0 to 255 before 1.18). With mods on the map, a note says when the version does not suit them, with a button to export for theirs.
- **Game**: **Allow cheats**, **Structures beyond the map** (Minecraft generates villages, strongholds and the like in the land it makes around the map), and **Only the selection** when there is one (only the tiles of 128 × 128 blocks it reaches are written).
- **World border**: its **Centre** and **Size**, and **Fit the map** to centre it on the map, just large enough.
- **Data packs**: zips or folders copied into the world and enabled (**Add data packs**, **Add a data pack folder**, **Remove**).

[![General tab showing the saves folder, world name, Minecraft version, world border and data packs.](assets/workspaces/export-general.webp)](assets/workspaces/export-general.webp)

#### World

- **Around the map**: what a player finds beyond the edges: **Vanilla terrain from the world's seed**, or **Nothing: the map floats in the void**.
- **Ores**: **Lay ores** in the rock as Minecraft lays them (Minecraft adds none to the map's finished chunks); where, how many and how large is in **Edit ▸ Ores…** (see [Ores](#ores)).
- **Spawn point**: where players appear the first time, for this export.

[![World tab with seed terrain selected, ore placement enabled and the player spawn coordinates.](assets/workspaces/export-world.webp)](assets/workspaces/export-world.webp)

#### Rules

**Game mode** (Survival, Creative, Adventure, Spectator), **Difficulty** (Peaceful, Easy, Normal, Hard), **Hardcore**, and the **World rules**: a preset or each rule (see [World rules](#world-rules)).

[![Rules tab showing Creative mode, Normal difficulty and the Vanilla world rules preset.](assets/workspaces/export-rules.webp)](assets/workspaces/export-rules.webp)

#### Test Export

**Test Export…** writes a part of the map to a world of its own beside the export ("*name* test"), to try it in the game quickly: **The selection**, **Around the view** (384 × 384 blocks around the middle of the map view), **These blocks** (a corner, width and depth typed in), or **Regions**: **Pick on the map…** shows the region files' grid and names over the map; a click takes a region of 512 × 512 blocks, a drag takes or leaves several, and **Done** comes back. **Snap to regions** rounds the other parts out to whole region files. The dialog lists the region files it will write (`r.0.-1.mca`…), and **Export the test** writes them.

![The export dialog's Test Export with Regions chosen and the region files listed](images/export-test-regions.png)

### Versions of the game

**File ▸ Export** writes the world to a Minecraft saves folder for the
version of the game you choose — the newest one installed on the computer
to begin with — with its cheats, structures beyond the map, world border,
spawn point, what surrounds the map (vanilla terrain or void), data packs
(copied in and enabled), and, in its **Rules** tab, its game mode,
difficulty, hardcore and world rules (see [World rules](#world-rules)).
Versions before 1.18 hold 256 blocks of height: the map is cut below 0 and
above 255, and the dialog says so. The water is written so that the game
leaves it as it is (see [Water in the game](#water-in-the-game)). **Test Export…** asks which part of the map to try: the
selection (draw a rectangle, an ellipse or a lasso with the Select tools
first), the 384 × 384 blocks around the middle of the map view, or blocks
typed in; the tiles of 128 blocks it reaches are written to a world of
their own beside the export ("… test"), to check the result in the game
quickly. **Regions** tests whole region files, as WorldPainter's test
export does: **Pick on the map…** makes way for the map, with the region
files' grid and names over it; a click takes a region of 512 × 512 blocks
(again leaves it), a drag goes on taking or leaving them, and **Done** at
the foot of the window comes back to the dialog (Esc cancels). For the
other parts, **Snap to regions** rounds them out to the whole region files
they reach, as merging maps region by region wants them. Either way the
dialog lists the region files it will write (`r.0.-1.mca`…), and the test
world has those files and no others.

The versions offered go from 1.7.10 to 26.3, and every newer game
installed on the computer. From 1.18.2 on (1.19, 1.20, 1.21 up to 1.21.11,
26.1, 26.2, 26.3), the chunks are written as that version writes them
itself, with its own data version: the game opens them as they are,
without converting them first, so the first load is quicker and nothing is
lost in a conversion. A block or a biome the version does not have yet
becomes the closest one it has — a cherry or pale oak log a birch or dark
oak log, a cherry grove a meadow, a pale garden a dark forest; a block
with nothing alike is left out of trees and plants, and is stone in the
ground — and a block renamed since takes the version's name (short grass
is grass before 1.20.3, an iron chain a chain before 1.21.9). Blocks of
mods are written as they are.

The world's settings are that version's own too: `level.dat` takes the
shape the version gives it (the spawn point and the border as 1.21.9
writes them, the border in a file of its own from 1.21.11), and for 26.1
and later the world is laid out as 26.1 lays it — the regions under
`dimensions/minecraft/overworld/`, the world's generation, weather, time
of day and border in files of their own. The game opens the world as it
is, with no "make a backup" question and nothing converted.

### World rules

The export dialog's **Rules** tab sets what goes on by itself in the world,
written into it so that it opens as set: no `/gamerule` to type after each
export.

![The export dialog's Rules, Creative / Frozen chosen, rules greyed where the version lacks them](images/export-rules.png)

- **Game**: the game mode, the difficulty, hardcore.
- **Preset**: **Creative / Frozen** (nothing spawns, spreads, grows or
  decays: no mobs, patrols, wandering trader nor phantoms, random ticks at
  0, fire that does not spread, mobs that break no blocks, no weather nor
  day going by, noon for good — to walk round a map as it was built),
  **Survival safe** (fire does not spread and mobs break no blocks, the
  rest as the game has it) and **Vanilla** (the game's own rules). Under
  **Yours**, the presets you saved: change a rule and **Save as preset…**
  keeps the rules under a name of your own. **Settings ▸ Export** lists
  them, to rename or remove.
- **Mobs**: mobs spawn, pillager patrols, the wandering trader, phantoms,
  mobs change blocks (creepers, endermen…).
- **Blocks and players**: fire spreads, **Random ticks** (the blocks ticked
  at random in each chunk section every game tick, 3 in the game: crops and
  grass grow, leaves decay, ice melts; 0 stops all of it), keep inventory.
- **Time and weather**: day and night go by, the time of day the world
  starts at (or stays at: morning, noon, sunset, night), the weather
  changes, and the weather it starts with (or keeps: clear, rain,
  thunder).

The rules are written as each version keeps them: as `GameRules` in
`level.dat` up to 1.21.10 (`doMobSpawning`, `randomTickSpeed`…), under the
new names from 1.21.11 (`minecraft:spawn_mobs`, `minecraft:random_tick_speed`,
fire spread as `minecraft:fire_spread_radius_around_player`…), and in
`data/minecraft/game_rules.dat` from 26.1. A rule a version does not have
(the patrols and the wandering trader before 1.15.2, phantoms before 1.15,
the weather cycle before 1.11) is greyed, and left out of its world; only
the rules that differ from the game's are written. The map keeps the rules,
game mode and difficulty of its last export, and the dialog offers them
again.

## Ores

The export writes the map's chunks finished, so Minecraft adds no ore to
them: the export lays its own, in the rock, as the game lays them since
1.18. **Edit ▸ Ores…** sets them — for each ore, bands of heights where its
veins start: few at the ends of a band, the most over its peak (a
trapezoid: a triangle when the peak is one height, an even spread when it is
the whole band), so many veins a chunk, so many blocks a vein.

![Edit ▸ Ores…: the cut through the map with the candles of each band, and the chart](images/ores-dialog.png)

- **Across the top**, each ore with what the map is expected to hold of it
  (a click picks it; a funnel marks one kept to some biomes), and the map
  small: click or drag on it to choose the row the map is cut through. The
  **+** at the end adds back an ore taken out.
- **The cut**: the map along that row, its rock, water and sky, with the
  ores exactly where the export will lay them, and over it a **candle** for
  each band — the wick from the lowest height to the highest, the body over
  the peak, its width the size of a vein. Drag the wick's ends or the
  body's edges to move those heights, the body to move the whole band, its
  side to make veins larger or smaller; the wheel over a body sets the
  veins a chunk. Dragging elsewhere looks along the cut.
- **The chart**: the ore expected at each height of the map, ore by ore, in
  **Blocks**, on a **Log** scale, or as a **Share** of the solid ground at
  that height (the map's rock read a column in sixteen); pointed at, every
  ore's amount at that height.
- **Below**, the details of the picked ore, its colour down their side:
  the biomes it keeps to (see below), its bands in numbers (from, most
  from … to, up to, veins a chunk, blocks a vein), **Add a band**, **Laid**
  to leave that ore out, and **Remove**. The band picked is ringed in the
  accent, as its candle is in the cut; the band whose candle the pointer is
  over is lit, and pointing at a band's row lights its candle.

**Delete** (Backspace on a Mac keyboard) takes the picked ore out, as
**Remove** does, and the one after it is picked; **⌘Z** (Ctrl+Z) puts back
what was taken out — an ore or a band — where it was, whatever was changed
since. While a number is typed, the keys edit the number. The **+** adds an
ore back as it was when taken out, or as the preset for the map's heights
has it.

**Biomes.** An ore can be laid **Only in** some biomes, or in every biome
**Except in** some, as a brush keeps to what is on the map: tick the box,
then **Add** opens the biomes (with a search) to tick on and off, each
ticked one shown as a chip that a click takes off. Only in with no biome
ticked lays the ore nowhere. The biome is the one the export writes at
the block — that of its cell of 4 × 4 × 4 (before 1.15, of its column).
The veins are drawn as before, their blocks just not laid outside those
biomes, so the other ores stay where they were (and take the rock left
over); the cut shows it, and the amounts and the chart count the rock of
those biomes only. Without a choice, an export is the same as before.

**Minecraft 1.18+** (the default for every map, new or opened) follows the
game's own veins and heights, the numbers set so that each ore's share of
the rock at each height comes out close to worlds the game makes — coal
over the whole height, most of it high up; iron at 16 and in mountains;
copper around 48; gold, redstone, lapis and diamonds deep down, diamonds
mostly in deepslate; emeralds high in mountains only. **Before 1.18** lays
them as older versions did, from 0 to 128. They are approximations of the
game, not its exact generator. **Richness** multiplies every band's veins;
**Lay ores** turns them all off. **Large veins** adds the game's large ore
veins since 1.18: ribbons of copper ore in granite between y 0 and 50, of
iron ore in tuff between y −60 and −8, with a block of raw metal now and
then; new maps have them, maps made before them keep their ores as they
were until it is turned on.

Ores only take the place of natural rock — stone, granite, diorite and
andesite; deepslate and tuff get the deepslate ores — and never of a block
in the open (one without solid ground right above it): none at the
surface, though cliffs and caves show some, as in the game. A vein starts
in one chunk and may reach into the next; the first vein to reach a block
keeps it. The veins come from the seed, the chunk and the band: the
export, the cut and the survival check agree block for block. Versions
before 1.17 get plain ores for the deepslate ones (and iron for copper).
The settings are saved with the map; the export's World tab can leave the
ores out of one export, and so can `massif-cli export --no-ores`.

## Survival check

**Edit ▸ Analyse the map…** checks the whole map for a survival game. Every
block is counted as the export writes it — the layers of each column's
terrain, the stone and deepslate under them, the bedrock floor, the water,
the plants of the covers, the blocks of the objects — and the land
measured: the height players stand at, the step from each column to its
neighbours, the water over it, its biome, the land around the spawn point.
On a WorldPainter map of 5,888 blocks a side (4.3 billion blocks) it takes
a few seconds, in the background. The ores are counted where the export
lays them, block for block.

![Edit ▸ Analyse the map…: the survival check's findings](images/survival-check.png)

What it finds comes in three tones, as a survival player would read it:

- **Red, breaks survival**: riches lying around (storage blocks of
  diamonds, emeralds or netherite, ores at the surface, or an ore far more
  common than Minecraft makes it underground), bedrock above the bottom
  five layers or at the surface, blocks no player can break (barriers,
  command blocks), holes into the void, no wood at all, no stone for tools,
  a spawn point in lava or over the void.
- **Amber, missing**: ores — the export writes the map's chunks finished,
  so Minecraft adds no ore, tree, village or stronghold to them; the ores
  are the ones the export lays (see [Ores](#ores)), and the card leads
  there —, lava for a Nether portal, water, grass (animals only spawn on
  it), trees near the spawn, leaves to grow wood back, a spawn point in
  water or off the map, peaks cut flat by the top of the world, many drops
  of 23 blocks or more (a fall that kills at full health).
- **What holds up**: the trees, the ores to mine, the water, the grass, the
  floor, the spawn point and its surroundings.

Above them, the way through a game: wood, stone, iron, diamonds, the Nether
(ten obsidian, cast from lava with a bucket of water or mined with a diamond
pickaxe). Below, **How to fix it** gathers what to do about each.
**Relief** draws the heights players stand at, the sea marked, and the
steps between columns: none, one block (walked), two or three (climbed),
4 to 22 (a fall that hurts), 23 and more (a fall that kills); the water and
the shape of the land (overhangs, air under the surface, columns at the top
of the world or open to the void). **Blocks** lists the ores (blocks,
storage blocks, in the open, a chunk), the blocks at the surface and in the
ground, the plants and the objects; **Biomes**, the biomes as the export
gives them. The card beside sums it up: ores, trees, water, lava, grass,
spawn; the map's chunks, heights and walkable share; its surface.

**Show on map** glides the map to the spot of a finding (the nearest to the
spawn) and points it out. **Copy report** copies the whole check as
Markdown, to paste where people talk about the map; the **…** menu saves it
to a file. Closed and opened again on the same map, the check comes back at
once. From the command line, `massif-cli analyse <map.massif>` prints the
same report.

## Render

[![Annotated village render with camera controls, sun and sky settings, and an animation timeline.](assets/workspaces/render.webp)](assets/workspaces/render.webp)

Frame the village with the camera, adjust the light under Sun and Sky, then use the timeline to set up a camera move. [View full size](assets/workspaces/render.webp).

The **Render** side (Beta, at the top left after Terrain and World) draws
the open map path-traced on the graphics card: the light of the sun and
the sky bounces off the land, shadows are soft, water mirrors the sky and
lets the sea floor show in the shallows, and glowing blocks (glowstone,
lanterns, torches, lava…) light what is around them. The map is built as
the game would get it from an export (its levels, covers, trees and
objects, ores and water), with the game's textures and models, nearest
the view first: the image fills in from the middle out. While the view
stays still it adds a sample a pixel a frame and grows clean; moving, it
draws a quarter of the pixels and starts again.

Its panels are editors as any other, in the **Render** section of an
area's chooser: **Render Viewport**, **Sun and Sky**, **Camera**, **Render
Output** and **Timeline**. **Still** shows the viewport with the sky, the
camera and the output beside it; **Animation** the viewport over the
timeline. Areas split, swap, float and change as on the other sides. One
area draws the viewport at a time; a second one says where it is.

- The view moves as Blender's: **drag** (or the middle button) turns about
  the point under the pointer where the drag began, **Shift-drag** slides,
  the point held following the pointer, the **wheel** (or a pinch) comes
  closer to the point under the pointer, a share of the way each step, so
  that two blocks over the grass and two kilometres over the map are both
  in hand. **Double-click** (or **.**) comes in close on the point there;
  on the sky, the whole map. **Numpad 1, 3, 7** look from the front, the
  right, above (with **Ctrl**, the other side); **Shift-click** focuses the
  lens there.
- Walk as with Blender's walk navigation: hold the **right button** to look
  around from where the camera stands, **W A S D** (by their places: Z Q S
  D on an AZERTY keyboard) to move where you look, **Q** and **E** down and
  up, at a speed by the distance to what is ahead, **Shift** four times
  faster, **Alt** four times slower, the **wheel** while walking to set it.
  The view stays denoised at half its resolution while it moves.
- **Camera** ▸ **Position**: the eye's **Location** to the tenth of a block,
  its **Turn** and **Tilt**. **Lock Camera to View** (on at first): off,
  the view moves alone and stills, keyframes and the video keep to the
  camera; **Align Camera to View** (Ctrl+Alt+Numpad 0) puts it where the
  view is, **View through Camera** (Numpad 0) the view where it is.
- **Sun and sky**: a **Look** to start from (Clear noon, Fair weather,
  Golden hour, Blue hour, Foggy morning, Night, Valley mist: a thin bright
  mist in the low ground at sunrise, the air clear above; the fog and the
  mist lie on the floor of the map's valleys, measured from the scene,
  whatever the map's height), then the **Time**, the
  **Day** of the year and the **Latitude** put the sun; **North** turns the
  map under it. **Sun** and **Sky** set their light. At night, a moon.
- A **Look** sets everything of the sky it owns: the hour, the sun's and
  the sky's strength, the air, the haze, the fog, the clouds and the
  exposure (the Blue hour and the Night exposed for their dim light, as a
  camera would be); the lens, the bounces and the grass stay as they were.
  Touching one of its settings afterwards makes the look **Custom**. When
  the camera path has keyframes, a look or the **Time** moved by hand moves
  their hours as much, so that the path does not put the old hour back.
- While the view moves it is drawn at half the resolution and denoised
  every frame; on a Retina screen the view draws a pixel a point, as
  Blender's automatic pixel size, a quarter of the rays; the graphics card
  is sent one sample at a time, and none while Massif is in the
  background, so that the rest of the app stays smooth.
- **Atmosphere**: the **Air**'s blue, **Haze**, a **Fog** (set by its
  **Visibility**, how far one sees through it, 40 to 4000 blocks) that fills the
  low ground first (its **height** and how fast it **fades** above),
  **Shafts of light** through it, and a **Sea around the map** out to the
  horizon.
- **Clouds**: their **Cover** (0: none), **Height**, **Thickness**,
  **Density**, **Size**, and how far the **Wind** has moved them; they
  shade the land.
- **Camera** ▸ **Lens**: the **Angle** of view, the **Aperture** (0: all
  sharp) and the **Focus** distance, for a depth of field. **Position**:
  the point it **looks at**, its **Turn**, **Tilt** and **Distance**, and
  **Frame the whole map**.
- **Image**: how much the glowing blocks **Glow**, the light's
  **Bounces**, **Clamp indirect** (the most light a bounce brings, 10 at
  first: rare bright paths cut down, so that the image settles sooner with
  fewer bright specks, for a little less light; 0 for no limit),
  and **Denoise**.
- **Color Management**, as Blender's: the **Display Device** (sRGB), the
  **View Transform** (Standard, Khronos PBR Neutral, **AgX** at first,
  Filmic, Filmic Log, False Color, Raw), its **Look** (AgX: None, Punchy,
  Greyscale and seven contrasts from Very High to Very Low; Filmic: its
  contrasts), the **Exposure** in stops and the display's **Gamma**. AgX
  and Filmic take each colour along an S curve over 16.5 stops of light,
  so that shadows stay soft and bright colours run to white instead of
  burning to a hue; Khronos PBR Neutral keeps colours as they are and
  compresses only the brightest; False Color shows the exposure in bands
  (grey is mid grey, green a stop under, yellow two over, red the
  brightest). Written from the transforms' public descriptions.
- **Denoise** (on at first) smooths the grain of the first samples away
  while they add up, as Cycles' viewport denoising does: an edge-avoiding
  filter guided by what each pixel's rays first meet (its colour, which
  way it faces, how far it is), which keeps the textures and the edges of
  the blocks sharp. The samples themselves are kept as they are: the
  filter only draws the image shown from them, and the more samples, the
  less it smooths, until past some 128 samples a pixel it leaves them be.
  Intel's **Open Image Denoise** (the denoiser Blender's Cycles uses,
  version 2.5.1, Apache-2.0, its licence and third-party notices in the
  app's folder) comes with Massif's builds; it
  takes over as the samples settle: after 1, 4, 16, 64, 256 and 1024
  samples and on the last, without holding up the view, and it denoises
  the image saved. Measured on a hilly map late in the day, against 1024
  samples: 1 sample through it is nearer the end than 4 raw ones (an
  error of 0.026 against 0.043); it takes some 0.2 s at 1920 × 1080.
- The image is anti-aliased as Cycles filters a pixel, the textures are
  averaged where they are smaller than a pixel (the game's crisp pixels
  kept up close), and leaves and glass far off add up to how much of them
  is solid, so that woods seen from afar settle smooth instead of
  flickering in a pattern of pixels.
- **Grass** (first in Render Output): as the game's **Blocks**, or as
  **Blades** of grass: over every grass block (not snowy), in the place of
  the short grass and the ferns, a meadow of thin blades made up as the
  image is drawn. The field is combed, as by the wind, the way a broad
  noise of the place says, and grows higher or lower in patches; the tall
  grass and large ferns become taller blades. Each blade bends along a
  curve to a fine tip, rounded across, darker and cooler at its foot and
  lighter and yellower to the tip, each a little its own shade, a few dry
  ones among them; the sun shines through them from behind, and the ground
  between them takes the grass's shade, not its pixels. The flowers stay
  as they are. **Density**: blades a block (45 at first); **Height** (0.8
  of a block at first). Far off, fewer and wider blades cover as much.
  Blades cost time: a sample at the grass's height took some four times as
  long as with the game's grass on this Mac. Changing between Blocks and
  Blades builds the scene again.
- **Image to save**: a **Size** (the view's, HD, 4K or 8K), the
  **Samples a pixel** (more: cleaner and slower) and **Denoise** (for an
  image of few samples: past some 128 it changes nothing), then **Save image…**
  asks where and draws the image a band of rows at a time, the view showing
  it as it comes, with its progress and **Cancel**; it is written as a PNG.
  A size larger than the graphics card can draw is refused.
- **Scene**: tiles built, chunks, the time they took, the graphics
  memory, and the samples a second.

- **Timeline**: keyframes of the camera (where it stands, what it looks
  at, its angle) and of the hour, over time. Put the playhead somewhere
  (click or drag along the track), place the camera, then **Add keyframe**
  (on a keyframe: **Update keyframe**); **Remove** takes the picked one
  away. Drag a keyframe to move it in time. Between keyframes the camera
  follows a smooth path through them, the angle and the hour ease from one
  to the next (midnight crossed the short way). Moving the playhead puts
  the view there; **Play** runs it (not with less motion: scrub instead);
  the buttons beside go to the start, the previous or next keyframe, the
  end. **Length** and **Frames a second** say how the animation will be
  drawn. The keyframes are saved with the map.
- Settings are keyed as in Blender: a small diamond beside the sun, the
  sky, the air, the haze, the fog, the clouds (cover, height, density,
  size, wind), the hour, the exposure, the lens's aperture and focus; a
  click on it, or **I** with the pointer over the setting, keeps its value
  at the playhead (the diamond fills), a click on a filled one or
  **Alt+I** takes the key away; an outlined diamond says the setting is
  animated. Between its keys a setting eases from one value to the next,
  before the first and after the last it holds; moving the playhead puts
  the values there, and the video draws them. Their keys show on the
  Timeline as small diamonds under the camera's.
- The Render panels are in Blender's sections, each folded away by a
  click on its title: **Sampling** (the **Viewport samples** the view
  stops at, **Denoise**, **Clamp indirect**), **Light Paths** (bounces,
  glow), **Color Management**, **Grass**, **Output: image** and **Output:
  animation**.
- The Timeline answers Blender's keys (not while a field is being typed
  in): **Space** plays and pauses, **I** (or **K**) keeps a keyframe at
  the playhead, **Alt+I** removes the one there, **←** and **→** go a
  frame back or on, **↓** and **↑** to the previous or next keyframe,
  **Shift+←** and **Shift+→** to the start or the end, **Home** frames the
  whole map.
- **Animation to save** (Render Output): **Render animation…** draws the
  Timeline's camera path frame by frame, each frame a still of the **Size**
  set for images and of so many **Samples a frame**, denoised as the stills
  are, and saves it as an **MP4 video** (H.264, through macOS's own
  encoder; on Windows and Linux, PNG images only for now) or as **PNG
  images** in a folder you pick (`name_00001.png` on). The progress shows
  the frames done and the time left; **Cancel** stops, the frames drawn so
  far kept: an MP4 closed on them, and PNG images asked again into the same
  folder go on from the first one missing.

A map larger than the graphics card's buffers is drawn in part, and says
so. With less motion (Settings ▸ Accessibility), the view stops at 64
samples a pixel.

![The Render side's Still workspace: the Render Viewport with Sun and Sky, Camera and Render Output](images/render-still.png)

## Save points

A save point keeps the map as it is now under a name — "before the rivers",
"version 2" — to come back to it, open it or compare the map with it later.
**File ▸ Save Point…**, or **Save point…** at the top of the **Versions**
panel (under World in an area's list of editors), asks for its name.

![The Versions panel with three save points](images/versions-panel.png)

The Versions panel lists the map's points, the newest first, each with a
small picture of it from above, its name and when it was made:

- **Restore** puts the map back as it was then: its land, levels, sculpted
  chunks, selection, blocks and settings (it keeps its name and its points).
  It is one step to undo: ⌘Z brings back the map as it was before.
- **Compare** draws over the 2D map where the map and the point differ: in
  the accent colour where the land is higher or lower, the stronger the more
  blocks; in grey where only its paint differs (terrain, biome, ground
  cover, objects or water). Under the point, the panel counts the columns
  higher, lower and painted differently, and the most the land rose and
  sank. The comparison follows the map as you change it. **Stop comparing**
  takes it off.
- In a point's **…** menu, **Open as a copy** opens the point as a map of its own,
  never saved, to change and save under another name; **Open read-only**
  opens it to look at: any change is taken back at once, and **Save As…**
  makes it a map of its own. Either closes the map open now, asking first
  if it has changes to save. **Rename…** and **Delete…** do what they say.

Save points are kept in the map's `.massif` file, so they go with it when
the file is copied, sent or backed up, and autosave keeps them too. They
take little room: a point shares every tile it has in common with the map
and with the other points, so it costs only the tiles changed since it was
made (128 × 128 blocks each, compressed): on a map of 8,192 blocks a side
(a 50 MB file), thirty points with a change of a few hundred blocks between
each added 4 MB, and saving took as long as without them. Making a point, renaming or
deleting one is a change to save; deleting one takes its tiles out of the
file at the next save. Versions of Massif before 0.6 open a file with save
points as the map alone, and saving it from them drops the points. Save
points are not exported.

Save points are also the map's **versions**: each knows the one it was made
from and who made it, so that copies of the map sent to other people can be
joined again. To work on one map with others, or to bring back a zone
edited in another tool, see [Working on one map together](versioning.md)
(beta).

## Autosave

While the map has changes not saved, Massif keeps a copy of it: once it has
had changes for the time set in **Settings ▸ Autosave** (5 minutes by
default), then again after each as long. The copy goes to a folder of
Massif's own, never over your file: `Autosave`, next to `startup.log`
(`~/Library/Logs/Massif/Autosave` on a Mac, `%LOCALAPPDATA%\Massif\Autosave`
on Windows, `~/.local/state/massif/Autosave` on Linux). It is taken between
two strokes and written in the background, on threads that give way to the
rest of Massif: on a map of 44,544 × 33,536 blocks (91,176 tiles, a 1 GB
file), the editor stops for 3 to 8 milliseconds to take it, and the frames
keep their pace while it is written (5 to 12 seconds).

![The Restore unsaved changes? window after a forced quit](images/restore-unsaved.png)

Should Massif close without saving — a crash, a forced quit, a power cut —
it shows, the next time it starts, **Restore unsaved changes?**: each map
left so, by name, when its copy was taken, its file and its size.
**Restore** opens the copy as that map, with changes to save (⌘S saves it to
its own file; a map never saved asks where); **Discard** deletes the copy;
**Later** keeps it to be offered at the next start.

Should Massif run into an error while you work, it keeps the map there
and then: it writes a copy at once, whatever the time set, and says
**Massif ran into an error**, with the error. **Save As…** saves the map
under a name of your own; **Carry On** goes back to work; **Quit** closes
Massif and keeps the copy, offered back at the next start as after a crash.
An error in a task in the background (opening, saving, exporting, carving
caves…) only stops that task, the map as it was.

Saving never harms the file already there: Massif writes to a new file
beside it and puts it in place only once it is whole and on the disk. A
full disk, a folder that cannot be written, or a file held by OneDrive, an
antivirus or a backup tool leaves the old file as it was, and the message
says which it was and what to do. Exports, packs, height maps, images and
Massif's own settings are written the same way.

Saving the map removes its copy, and so does closing it: quitting, or
opening or making another map, whether its changes were saved or let go.
At most 5 copies are kept, of at most 4 GB together, the oldest removed
first; a copy that Massif running in another window is keeping is never
removed.

## Working together

Several people can work on one map, and a part of a map can go through another tool and come back:

- **Save points are versions.** Each save point knows the one it was made from and who made it (see [Save points](#save-points)); the **Versions** panel draws them as a timeline, with the map's line and a lane for each branch.
- **A region for someone else**: **File ▸ Export Region…** writes a part of the map as a heightmap and masks (or as a map of its own), to work on in another tool or by someone else; **File ▸ Merge Massif Map…** brings the work done on a copy or a region of this map back, reviewed on the map before anything changes.
- **Contributions**: a contributor works on a copy and sends their changes back as a small `.massifpatch` file; the owner sees on the map what each contribution changes, accepts or rejects it area by area, settles the places where both changed the same land, and applies it as one step to undo and a new version.
- **A zone through another tool**: Export Region with **Review it when it comes back** writes a `zone.json` beside the images; editing them in World Machine or Gaea and importing them back (**Import a contribution…** in the Versions panel) reviews them as a contribution.
- **A shared folder** (Dropbox, a network drive) can hold the versions for everyone.

All of it is in [Working on one map together](versioning.md) (beta).

![The review of a contribution: its changed area and the conflict on the map, the Versions panel beside it](images/review.png)

## Scripting

**File ▸ Run Script…** runs a script on the open map, and `massif-cli script` runs one from the command line. Massif runs three kinds of text:

- **WorldPainter's JavaScript scripts** (`.js`), as they are: the `wp` API, its arguments and parameters, the script's descriptor. Scripts written for WorldPainter run unchanged, in a modern JavaScript engine; anything Massif cannot do yet stops the script with its line, never silently.
- **Massif's own API, in Lua** (`.lua`): reads and writes rectangles of columns at once (heights, water, terrains, biomes, layers, annotations, tree sizes, the selection), paints masks from images, runs the Whole map passes, opens, makes, saves and exports maps.
- **MassifScript** (`.mscript`), a different thing: the Terrain Generator's node graph written as text, in the graph's **Script** pane. With **Copy for AI** it gives any AI chat what it needs to write a graph for you.

The dialog of Run Script… shows the script's parameters (from its descriptor), runs it with a progress bar and **Stop**, and shows what it prints. A script reads and writes only files in its own folder and those you give it, and has no network. Every run is one step to undo.

- [Scripting](scripting.md): the dialog, WorldPainter's scripts, and every function of the Lua API, with examples.
- [MassifScript](massifscript.md): the language, the editor, Copy for AI, and [the reference of every node](massifscript-reference.md).
- An **MCP server** for MassifScript, so that an AI assistant could build and change graphs in Massif directly, is planned but not shipped: today, Copy for AI and pasting the answer back is the way.

![File ▸ Run Script…: a Lua script's parameter and its output after a run](images/run-script.png)

## Command line

`massif-cli`, which comes in the release archives beside the app, does from a terminal what the app does, for batches, servers and scripts. Run it without arguments for the full list.

| Command | What it does |
|---|---|
| `massif-cli new <out.massif> [--size WxD] [--landform NAME] [--seed N] [--name TEXT] [--limits MIN..MAX] [--relief BLOCKS]` | A new map (size in tiles of 128 blocks, 8x8 by default; landform flat, plains, hills, mountains or islands). |
| `massif-cli info <world.massif>` | What the map holds. |
| `massif-cli verify <world.massif>` | Checks the file. |
| `massif-cli export <world.massif> <folder> [--version V] [--void] [--survival] [--no-ores] [--overwrite] [--rules frozen\|survival-safe\|vanilla\|map] [--regions x,z:x,z]` | Exports to Minecraft (1.18.2 by default). |
| `massif-cli rules <preset> [--version V]` | Prints the game rules a preset writes. |
| `massif-cli import <old.world> <out.massif>` | Imports a WorldPainter world. |
| `massif-cli import-minecraft <world folder> <out.massif> [--above keep\|trees\|drop] [--area X0,Z0,X1,Z1] [--nether\|--end] [--mixes] [--caves] [--no-covers]` | Imports a Minecraft world. |
| `massif-cli probe <world.massif> [<x> <z>…] [--random N]` | What the export writes at columns, as JSON. |
| `massif-cli analyse <world.massif>` | The survival check, as a report. |
| `massif-cli water`, `rivers`, `lakes`, `spring` | Water: a report, rivers and lakes over the whole map, a river from a source. |
| `massif-cli crop <world.massif> <out.massif> <x0> <z0> <x1> <z1>` | A part of the map as a map of its own. |
| `massif-cli caves <world.massif> <out.massif> [--biomes] [--seed N] [--keep]` | Caves over the whole map. |
| `massif-cli cut <world.massif> <y> <out.png> [--rect …]` | A picture of the map cut at a height. |
| `massif-cli structure …`, `village …` | Places a structure or a village. |
| `massif-cli village-check <pack>`, `village-pack-from-world <world> <dir>` | Checks a village pack; cuts one from a world. |
| `massif-cli script [--world <map>] [--out <map.massif>] [--allow <folder>] [--param name=value] <script> [arguments…]` | Runs a WorldPainter `.js` or a Massif `.lua` script. |
| `massif-cli terrain schema\|catalog\|validate\|patch\|render\|stream\|migrate\|build` | Works on Terrain Generator graphs (JSON): their schema, every node, checks, edits, pictures, building them into a map. |
| `massif-cli mscript guide\|build\|check\|decompile` | MassifScript: the guide for an AI, a script to a graph, checks, a graph to a script. |

## Settings

**Settings** (⌘, — in the **Massif** menu, the first, on every system — or
the home screen's sidebar) has ten sections, in a window of
their own:

- **Appearance**: the theme (**Follow the system**, the default, **Light**
  or **Dark**; also in the **View** menu), the accent colour (the colour of what is
  chosen, of the main buttons and of the home screen's art: eight colours, or
  any other), the **Loader** shown while Massif works (eight grids of dots
  lighting in patterns — Orbit, Ripple, Scan, Rain, Bars, Spiral, Sweep,
  Twinkle — each alive in its tile, still with Reduce motion), the **Tool
  icons** (**Line**, the default, in the style of the interface's other
  icons, or **Classic**, Massif's first drawings), the **Crosshair** of the player's view: a **Dot**, a
  **Cross** (the default: four short strokes around an open middle), a
  **Circle**, a plain **Minecraft** plus, or **None**, with its **Size** (6
  to 32 points) and **Opacity**, beside a preview on light ground and dark.
  It is drawn white in a soft dark outline, so that it shows on snow as on
  deep water, more strongly in high contrast. The 2D map draws it at the
  middle of its view too, as at the middle of the player's view, to show
  where the view is centred; the pointer keeps the system's crosshair, with
  a white dot. Last, the **Home background** (**Ridges**, **Waves**, **Pixel**
  or **Prism**, alive in a picture above its few settings, and **Animate the
  background**; see [the home screen](#the-home-screen)).
- **Accessibility**: **High contrast** — text black or white, muted text
  past 7 to 1 on the cards, lines and the edges of controls past 3 to 1, the
  accent darkened or lightened until it reads, links underlined, and the
  home screen's ridges reduced to their crests, none behind its words; **Reduce motion** — the map
  and the 3D views stop where they are let go and go straight where they are
  sent (no gliding on, no coasting, no easing), the home screen's ridges and
  the cards of tools and brushes stay still, switches flip at once; and
  **Interface size** — 100, 110, 125 or 150 %, everything larger at once (⌘+
  and ⌘− change it too, ⌘0 puts it back, and the change is kept). On macOS
  the first two follow the system by default (System Settings ▸
  Accessibility ▸ Display: Increase contrast, Reduce motion), and can be set
  off or on whatever it says.
- **Typeface**: what the interface is set in — the system's own (SF Pro on
  macOS, Segoe UI on Windows), or one of four built into Massif: **Inter**,
  drawn for screens, **Geist**, a crisp geometric grotesque, **IBM Plex
  Sans**, engineered, and **Manrope**, open and rounded. All are variable
  fonts under the SIL Open Font License, so the medium and semi-bold of
  titles and values are real weights. Each is shown in itself, and a click
  sets the whole interface in it, at once.
- **Sounds**: little sounds, synthesised as they play — soft mallets,
  clicks and air, as [Cuelume](https://cuelume.dev/) makes them, so there
  is no sound file in the app: three notes rising when a long task is done,
  two when the map is saved, a low pair when something fails, one wavering
  when something is refused or stopped; with **All**, soft ticks for
  choices, switches, windows opening and closing, workspaces. **Off**,
  **Essential** (what ends and what fails) or **All**, a volume, and each
  sound to hear. The sound output is open only while a sound plays (and two
  seconds after): silent, Massif holds neither the speakers nor wireless
  headphones, which can go to another device.
- **Notifications**: the system's own (the Notification Centre, a toast on
  Windows, the desktop's on Linux) when a task of more than two seconds ends
  — an export, an import, a conversion, a pass over the whole map —:
  never, while Massif is not in front, or always; and a button to try it.
- **Memory**: a limit (automatic, 1, 2, 4 or 8 GB) for what is kept decoded
  — three fifths for the map's tiles, measured at the size they take, the
  rest for the column data and images of the map view —, beyond which what
  was used least is put away, compressed; and what is in use now.
- **Autosave**: on or off, and how often: every 1, 2, 5 (the default), 10
  or 15 minutes of changes not saved (see [Autosave](#autosave)).
- **Updates**: **Check for new versions**, on by default (see
  [New versions](#new-versions)), and the version skipped, to be told of
  again.
- **Graphics**: how much the graphics card is asked to do. **Low**,
  **Medium** and **High** set everything at once; a change to any setting
  makes it **Custom**, and every change applies at once, without a restart.
  The 3D views: their **Render scale** (drawn at 50, 67, 85 or 100 % of
  their pixels and stretched to fit: what saves the most, close to half the
  time at 67 %), **Smooth edges** (four samples a pixel; once the view has
  been still for a moment, the traced ground is drawn again with a ray
  through each sample, its steps' edges smoothed too, for about three times
  the time of that one frame), **Fog**, **Water
  reflections**, **Game textures in the 3D views** (each block drawn with
  its texture from the game or its mod, in the 3D View and 3D Sculpt as in
  the player's view; off, a flat colour a block, as Low draws them; when no
  Minecraft is found, or the graphics card cannot draw them, the views draw
  the flat colours and a line under the switch says why, also written in
  `startup.log`), and
  how much of the map the 3D view keeps on the graphics card (about 320 MB, 800 MB or
  2 GB): a larger map is drawn with a column every 2, 4 or 8 blocks, all of
  it, coarser — on a huge map with much water, far faster than every block.
  Come low over the land (within 1,500 blocks of it), and the 3D views draw
  every block, textured, over about 3 km around the eyes, made away from
  the frames in a second or so and made again as the eyes move on, the
  land farther off as before; the status line says so (every block near
  the eyes). Water there is drawn as on a smaller map, clearer over the
  shallows, where farther off it is a plain surface.
  **Load**: **Around the view** (the default) or **The whole map**. Around
  the view, a 3D view builds the land within the **View distance** of its
  camera (1,024, 2,048, 4,096 or 8,192 blocks), the nearest first, and
  shows the rest at most as the first look the 2D map's summaries give,
  nothing decoded; the Diorama builds the tiles in view and a quarter of the
  view around them, within the view distance of its middle; the Render
  holds and builds only the part of the map within the view distance of the
  point its camera looks at, and makes it again around the new place once
  the camera has gone half the distance away (for a camera path that flies
  far, choose the whole map). Once what they show is built they rest, and
  moving builds the new place. On a huge map
  they are ready far sooner, and the processor rests. The whole map builds
  every tile, as before 1.2, for flying over all of it at length.
  The Terrain viewport: its **Light and shadows** (the steps of its
  shadows, clouds and sky light; saved images always take the most) and its
  **Clouds**, what costs the most there. Then the map view's images kept on
  the graphics card (128 MB, 512 MB or 2 GB) and how many frames a second
  the window draws while nothing is touched but something moves (20, 30, 60
  or no limit). On its very first start, Massif picks the preset for the
  graphics card it finds (Low on OpenGL, older integrated graphics or a
  software renderer; Medium on recent integrated graphics; High on a
  graphics card of its own) after drawing a few frames of the 3D view as a
  trial, and writes the choice in `startup.log`; a preset you choose is
  never changed. When the 3D views stay slow for a while as you move them
  (not while their tiles are still being built), a quiet note suggests the
  preset one step lower, with a button that applies it; its × stops the
  suggestion for good.
- **Developer**: developer mode adds a **Developer** panel to every area's
  menu, as the brushes or the layers: frames a second and the time each
  takes (and how much of it is Massif's own work), the memory the system
  counts for Massif and the processor's time, in charts that run live; then
  what is kept decoded, the map, the tasks running, the graphics card, and
  why Massif draws again.
  **Frames a second** sets how the window draws: **Vsync** (the default)
  keeps in step with the screen; 60, 120, 144 or 240 frames a second at
  most, or **Unlimited**, show each frame as soon as it is drawn (the
  graphics card's Immediate mode, else Mailbox; where it has neither, the
  screen still sets the pace). **Draw every frame** keeps the window
  drawing when nothing changes, so the charts show how many frames the
  computer makes. Both apply at once, and only in developer mode.

![Settings ▸ Graphics: the Low · Medium · High presets above the 3D views' settings](images/settings-graphics.png)

## Controls

Massif follows WorldPainter's defaults. These are the keys out of the box;
**Settings ▸ Keyboard** changes any of them.

![Settings ▸ Keyboard: the shortcuts in columns, and the search](images/settings-keyboard.png)

### Your own shortcuts

[![Keyboard settings with the Massif preset selected. The Save row in General shows ⌘S and a plus button.](assets/workspaces/keyboard-massif.webp)](assets/workspaces/keyboard-massif.webp)

To change **Save**, click its **⌘S** shortcut (**Ctrl+S** on Windows and Linux), then press the keys you want to use. Click **+** on that row to keep the original shortcut and add a second one.

[![Keyboard settings with WorldPainter selected under Start from. Redo shows ⌘Y first.](assets/workspaces/keyboard-worldpainter.webp)](assets/workspaces/keyboard-worldpainter.webp)

Select **WorldPainter** under **Start from** to use its bindings: on macOS, the Massif preset lists **⇧⌘Z** first for **General ▸ Redo**, the WorldPainter preset **⌘Y**.

Settings ▸ Keyboard lists every shortcut in columns — General, Tools,
Brushes, Map, 3D view, Player — with a search over names and keys.

- Click a shortcut, then press the new keys. An action can have two; the
  `+` beside its keys adds one, a right-click removes one. For what is held
  (Move the map, flying, walking, adding to a selection), modifiers alone
  count: press Shift and let it go.
- Keys another action already has, where both are listened to, are asked
  about first: **Replace** takes them from it, **Cancel** leaves both as
  they were. W is Forward both in the 3D view and in Player, which never
  listen at once.
- **Start from** Massif's keys or WorldPainter's: there ⌘Y redoes first,
  and ⌘ with +, − and 0 zoom the map rather than the interface. Your changes
  stay over the preset chosen; ↺ beside an action, or **Reset all**, takes
  them back.
- **Export…** writes every shortcut to a keymap file (JSON, the same on
  macOS, Windows and Linux: `Cmd` is ⌘ on a Mac and Ctrl elsewhere);
  **Import…** reads one. An action the file does not name keeps the
  preset's keys; a name Massif does not know is left aside.
- The menus, macOS's menu bar too, show the keys as they are now.
- While a text field is typed in, shortcuts wait, but those with ⌘ (Ctrl)
  that typing has no use for: New, Open, Save, Save As, Export, Settings,
  Help and the interface's size.
- The mouse keeps its own: the wheel and its modifiers, the buttons.

### Out of the box

| Input | Action |
|---|---|
| Left / right button | Apply the tool / its inverse (lower, erase, drain…) |
| Alt + click | Pick what is under the cursor: the terrain, or the biome, ground cover or object layer of the tool in hand (in 3D too) |
| Eyedropper tool (⌥Y), click | What is under the cursor to paint with, as WorldPainter's: its terrain, its biome or a layer, in a list by the pointer; the one picked is painted with, its tool in hand (only a terrain there: picked at once). Shift + click: the terrain, height and water at once, back to the tool before (in 3D too) |
| Scroll | Brush size (Shift: one block at a time) |
| Alt + scroll | Turn the brush, as in WorldPainter, 5° a notch (Shift: 1°); ⌘ + scroll too on a Mac, Ctrl + Shift + scroll on Windows and Linux |
| Ctrl + scroll, pinch | Zoom the map |
| Middle button, Space + drag | Move the map |
| Home, View ▸ Fit the whole map, the button by the zoom | The whole map in view (a map opened is framed so by itself) |
| `−` `+` | Brush size |
| `1` … `9`, `0` | Intensity 10 % … 100 % |
| `[` `]` | Turn the brush |
| `R` `L` | Turn the light of the map |
| ⌥H ⌥A ⌥S ⌥P ⌥R ⌥F ⌥Y | Height, Flatten, Smooth, Paint, Spray, Flood, Eyedropper |
| ⌘Z / ⇧⌘Z, ⌘S, ⌘O, ⌘N, ⌘E | Undo / redo, save, open, new, export |
| ⌘A, ⌘D, ⇧⌘I | Select the whole map, deselect, invert the selection |
| ⌘+, ⌘−, ⌘0 | A larger, smaller interface; back to its size |
| Ctrl + Space | Fill the window with the area under the pointer |
| Shift or ⌘ (Ctrl) / Alt + drag (Select tools) | Add to the selection / take out of it (both: intersect) |
| Right-click, Esc (Select tools) | Clear the selection |
| Esc | Cancel the stroke or the selection being drawn |

In the 3D view:

| Input | Action |
|---|---|
| Left drag / right drag | Turn / move the view |
| Scroll, pinch | Come closer / go back |
| W A S D or arrows, Q / E | Fly over the map, down / up (Shift: faster) |
| Double-click | Look at the point under the cursor |
| Header buttons | Look at the middle of the map view; show the whole map |
| The dot on the 2D map | Where the 3D view is (the point it turns about, or the player, ringed), with a wedge the way it looks; it follows the view live, and dragging it moves the 3D view there. With the grid shown, a small faint plus marks the world's origin, x 0, z 0 |
| In 3D Sculpt, with the tool of the Tools panel | Left uses it where the cursor points, right inverts; middle drag or Alt + drag turns, Shift + middle drag moves |
| 3D Sculpt's **Player**, cursor free (Axiom's editor) | Left drag looks around; right uses the tool under the cursor (Alt: inverts); middle picks the block; Ctrl (⌘) + left turns about the point, Ctrl + right moves, Ctrl + middle sizes the brush; the wheel sets the flight speed |
| 3D Sculpt's **Player**, mouse taken (a click) | The mouse looks around; right uses the tool at the crosshair, left inverts; Esc gives the mouse back |
| 3D Sculpt's **Player**, keys | W A S D walk, Ctrl or W twice sprints, Space jumps (twice: flies, up), Shift or Q sneaks (flying: down), G turns Ghost on and off (flies through the ground) |

### Moving smoothly

The map and the 3D view are made to follow the hand:

- A 3D view the workspace no longer shows (another workspace, or its area
  given to another editor) stops building its tiles at once, and after 20
  seconds out of sight gives back its tiles and the memory it holds on the
  graphics card; shown again, it builds them again. Opened on a large map,
  it first shows all of it at once from what the 2D map has read of every
  tile (a few columns a tile), then reads every column, the nearest first;
  a part not built yet is never drawn black. Its threads for the
  tiles are at most 16, below the interface's priority on Windows.
- A drag of the map let go while moving glides on, slowing down within a
  quarter of a second (any click stops it); the zoom buttons and keys and
  **Fit the whole map** glide to where they go in about 150 ms, around the
  middle of the view, and land exactly there.
- In 3D, a turn or a move let go while moving goes on for a moment, flying
  with W A S D starts and stops over a tenth of a second, a double-click and
  the header's *look at the map view* glide there.
- While the 3D view moves it is drawn at 60 % of its size on a Retina
  screen (80 % elsewhere) and stretched, then sharp again 150 ms after it
  stops: tracing millions of pixels a frame was what made turning heavy.
- The map's images are made a few tiles at a time, within a budget of each
  frame — 5 ms while a view moves, 12 ms otherwise, the coarser images
  standing in meanwhile — and while one view moves, the 3D view puts off
  building its tiles (a tile can take longer than a frame) until it stops.
  On a WorldPainter map of 5,888 blocks a side, at 60 frames a second on a
  Retina screen, zooming the map went from 32 ms of work a frame to 6, and
  moving it from 12 to 2.

**Settings ▸ Developer** shows frames a second, the time of each frame and
of its work, live.

## New versions

Once a day at most, a moment after it starts and without holding it up,
Massif asks GitHub which release of Massif is the latest. The request
carries Massif's version and nothing else: nothing about you, your computer
or your maps. The answer is kept a day in `update.json`, next to
`startup.log`; offline, or if GitHub does not answer, Massif says nothing.

When the latest release is newer than the version running, a small pill,
**0.6.1 is out**, shows at the right of the top bar, and a line, **Massif
0.6.1 is out**, above **Settings** on the home screen. Either opens the
release's notes: what is new in every version since yours. **Open the
release page** opens it in your browser, to download the new version and
install it as you did this one; Massif never downloads or installs anything
itself. **Skip this version** stops telling of that one (a newer one is
told of again), and **Later** keeps the pill. **Settings ▸ Updates** turns
the check off, or tells of a skipped version again.

## In the Finder and Explorer

A `.massif` file shows its map as its icon: Massif keeps a small picture of
the map in the file, which the system reads without opening the map — as
WorldPainter's fork does for `.world` files.

- **macOS**: the Quick Look extension that comes with the app
  (`tools/macos`) draws it, in the Finder and in Quick Look. A double-click
  on a `.massif` opens it in Massif; Open With ▸ Massif imports a
  WorldPainter `.world`.
- **Windows**: `MassifThumbnails.dll`, next to `Massif.exe`
  (`crates/massif-thumbnails`), draws it in Explorer. Massif registers it
  for the current user each time it starts (no installer, no administrator
  rights), with `.massif` files opening in Massif on a double-click; moving
  its folder and starting it again moves the registration.

Maps saved before this have no picture until they are saved again.

## Reporting a problem

**Help ▸ Report a Problem…** makes one `.zip` file to send us when Massif
crashes or is slow: in `#bug-reports` on [Massif's Discord](https://discord.gg/2dP7xJb655), or with
an [issue on GitHub](https://github.com/M-U-C-K-A/Massif/issues). Nothing is
sent from Massif: you save the file where you like and send it yourself. The
dialog links to both. It holds:

![Help ▸ Report a Problem…: what it attaches](images/report-problem.png)

- `report.txt`: what happened, in your words if you write them; Massif's
  version and the commit it was built from; the system, the processor and
  its threads, the memory, the graphics card, its graphics API and driver,
  the window's size and scale; the settings that change how Massif draws
  (theme, typeface, interface scale, accessibility, memory, a
  `WGPU_BACKEND` if set); the map's size, heights and levels; and the
  Developer panel's figures over the last minute (frames a second, frame
  times, memory, processor, what Massif keeps);
- `startup.log`, `startup.previous.log` and `graphics.txt` (see below);
- `last-minute.csv`: every frame of the last minute, with the memory and
  the processor;
- `map.massif`, the map as it is now, only if **Include the map** is on.

The dialog lists every file with its size, and **Show** opens each one as it
will be written. Your home folder is written `~` in all of them, and the
names in any other account's folder are left out. **Copy Summary** puts a
few lines on the clipboard for a message: the version, the computer, the
graphics card, how the last minute ran and the last error of the log.

**Performance recording** is for what is slow: **Start Recording**, do the
slow thing, then **Stop** in the pill at the top of the window (it stops by
itself after ten minutes). The report then also holds `recording/`, the
files of a performance recording (see below).

## Recording performance

When Massif stutters, **Help ▸ Record Performance…** (also **Record…** in
**Settings ▸ Developer**) records it for us. **Start Recording**, then use
Massif as when it slows down: move the map, sculpt, paint, open the views
that stutter. A red dot and the time recorded show at the right of the top
bar; click it, or **Help ▸ Stop Recording Performance**, to stop. The
recording is saved on your desktop as `Massif performance <date>.zip`, shown
in the Finder (the Explorer on Windows), to send in `#bug-reports`. It stops
by itself after ten minutes, and costs too little to be felt while it runs.

It holds:

- `summary.txt`: the frames, the slowest of them and what took their time,
  where the interface's time went over the whole recording, the other
  threads' work, and why frames were drawn when nothing happened;
- `frames.csv`: every frame, its time, how long it kept the interface busy
  and the app's own work in it, the time of its main parts (the 2D map and
  its tiles, the 3D views, the Diorama, the Render side, the panels,
  laying out, tessellating, drawing, waiting for the screen), the memory,
  the processor, the tiles decoded, the graphics card's memory, the tool in
  hand, the input, why it was drawn and the tasks running;
- `trace.json`: a timeline of every part of every frame on every thread,
  Massif's and its libraries' (egui, wgpu), with the input, the memory and
  the processor, to open in `chrome://tracing` or
  [ui.perfetto.dev](https://ui.perfetto.dev);
- `inputs.jsonl`: the pointer, the keys and the tool in hand, frame by
  frame: what you did, to play it back on the same map;
- `system.txt`: Massif's version, the computer, its graphics card and
  driver, the settings that change how Massif draws (graphics preset,
  frames a second), the map's size and levels, what Massif keeps, the
  workspaces and their areas, floating panels included;
- `layout.json`: the workspaces, for the replay.

Never the map itself, nor what you type (only how many characters), nor
where your files are: the home folder is written `~`.

The replay plays a recording back on a map, offscreen, and times its frames
as the recorder does (see [development](development.md#performance)).

After Massif closed without saving, the dialog that offers to restore your
map has **Report the Problem…** too.

## If Massif does not start

Massif writes down every step of its start in a log of its own, with the
time each took, the graphics card it found, and any error or crash, without
sending anything anywhere:

| System | Log |
|---|---|
| Windows | `%LOCALAPPDATA%\Massif\startup.log` |
| macOS | `~/Library/Logs/Massif/startup.log` |
| Linux | `~/.local/state/massif/startup.log` |

The log of the start before is kept beside it as `startup.previous.log`, so
starting again after a start that hung does not lose it. If Massif starts
again, **Help ▸ Report a Problem…** puts both in one file to send, with the
computer and the graphics card (see above); otherwise send the two logs:
the last line says where Massif stopped.

- The first frames are followed step by step until one is on the screen:
  the interface laid out, part by part (the home screen's ridges, its side
  bar, the new map's settings), its shapes and textures sent to the graphics card,
  the image the window gives to draw on (`get_current_texture`), the frame
  drawn and presented. A frame the window gives no image for is written
  down with the reason.
- If Massif cannot start (no graphics card it can use, a driver that
  refuses), or stops on an error, a message says so and where the log is. On
  Windows a crash, in a graphics driver say, is written down with its
  exception and the file it came from.
- The window stays hidden until its first picture is drawn, so that it does
  not flash white; on Windows it is shown before, as drawing in a window
  never shown is what some drivers handle worst.
- When the start stays 12 seconds in a step of the graphics card (opening
  it, taking the frame's shapes and textures, giving the window's image,
  presenting it), the graphics driver is stuck: the graphics API Massif was
  drawing with is set aside on this computer, and the next starts draw with
  another one — on Windows DirectX 12 after Vulkan, then OpenGL. On Windows a
  message offers to start again with it right away. A start that died
  without a word in such a step is found by the next one, which sets its API
  aside in the same way. The APIs set aside are listed in `graphics.txt`
  beside the log: delete the file to let Massif try them again (after a
  driver update, say).
- A start that is slow in its own work (laying out the interface, say) is
  only written down in the log, with the part it is in: another graphics API
  would not help it. The pictures of the home screen, the new map's preview
  and the dates of the recent maps are made on threads of their own, and the
  brushes and the game's textures load on threads kept for them, so that the
  first frame waits on none of it. Massif 0.5.1 set every API aside when its
  first frame was held up by them; those lines of `graphics.txt` (they end
  with "the first frame did not come") are taken out, and the APIs tried again.
- Massif's window keeps answering while it starts: nothing it does on its
  way waits on the disk, the network or Explorer. The log is written by a
  thread of its own, the folder of the copies kept while you work is read in
  the background (the offer to restore one comes as soon as it is read), the
  `.massif` file types are registered beside the start, and the work of the
  home screen and of the loads runs below the window's priority, so that a
  computer with few cores still draws it. The making of the app is written
  down part by part, with the time each took.
- For as long as Massif runs, a frame that keeps it busy more than five
  seconds (when Windows would call the window "not responding") is written
  in the log, with where Massif is; on Windows, the code it is in, by file
  and place (`massif.exe+0x…`, or a driver's file). It is written again
  after 15 seconds, a minute and three, and once the window answers again,
  how long it took.
- Massif draws with Vulkan, DirectX 12 or OpenGL on Windows, Metal on macOS,
  and Vulkan or OpenGL on Linux. Updating the graphics driver often helps.
  To choose one, set `WGPU_BACKEND` to `dx12`, `vulkan` or `gl` where
  Massif is started; `WGPU_POWER_PREF=low` picks the integrated graphics
  card of a laptop rather than the dedicated one. On Windows, type `cmd` in
  the address bar of Massif's folder, then in the window that opens:

  ```
  set WGPU_BACKEND=dx12
  Massif.exe
  ```

  Massif started from Explorer afterwards does not see it. The log says
  which API it was asked for.
