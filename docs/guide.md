# Using Massif

[← Back to the README](../README.md)

The editor, part by part: the home screen, the window, brushes and their options, painting, decorating, water, height maps, 3D sculpting and the controls. The terrain generator has [a page of its own](terrain-graphs.md); how Massif works inside is in [internals](internals.md).

- [The home screen](#the-home-screen)
- [The window](#the-window)
- [Settings](#settings)
- [New worlds and export](#new-worlds-and-export)
- [Brushes](#brushes)
- [Automatic biomes and paint](#automatic-biomes-and-paint)
- [Noise painting](#noise-painting)
- [Ground covers](#ground-covers)
- [Custom objects](#custom-objects)
- [Rivers](#rivers)
- [Height maps](#height-maps)
- [Real blocks](#real-blocks)
- [3D Sculpt](#3d-sculpt)
- [Cards](#cards)
- [In the Finder and Explorer](#in-the-finder-and-explorer)
- [Controls](#controls)

## The home screen

Massif opens on its home screen. On the left are the ways in — **New map**,
**From a height map**, **Open a map…** (a `.massif`), **Import from
WorldPainter…** (a `.world`) — and the maps opened lately. In the middle are
the settings of a new map beside a preview of the whole map, as tall as they
are (the cross marks its centre), and **Create map**. What is written over the
picture has a halo of the page's colour around its letters, so that it reads
over any part of it. A `.massif`, a `.world` or a height map dropped on the
window opens too.
**File ▸ New** (⌘N) comes back to the home screen, with a way back to the open
map (Esc).

Behind it is one of the pictures of
[`crates/massif-app/assets/backdrops`](../crates/massif-app/assets/backdrops) —
ten scenes at night: a canyon, the Alps, volcanic islands, stone pillars in
the mist, dunes, a fjord under the northern lights, terraced fields, chalk
cliffs, a lake in the pines, a desert under a ringed planet —, a new one each
time Massif opens (those not shown yet first; the button at the bottom right
shows another), reduced to sixteen colours of its own with an ordered dither
in square cells that moves on about once a second. Every `.jpg` or `.png`
dropped in that folder is built in, but the layers of a scene (`_ciel`,
`_relief`).

## The window

The window is made of areas, as in Blender. Each area shows any editor: the
map, the 3D view, tools, tool settings, brush settings, brushes, custom
brushes, terrain and biome palettes, levels or world info.

- The icon before an area's title opens every editor on one page, in
  columns: the World side's views, tools and paint, then the Terrain
  side's generator. The line under them says what the one under the
  pointer is for, and whether another area shows it already. An empty
  area holds the same page.
- The ⋮ menu of an area splits it, joins it with its neighbour, maximises it
  (Ctrl+Space) or closes it. Hold Shift for a one-click close button.
- Drag any corner of an area into it to split it where you let go: the new area opens on that corner's side (across: left or right; up or down: above or below).
- Drag an area by its title onto another to move it, or onto its centre to
  swap the two. Drag the gaps between areas to resize them.
- The app has two sides, switched at the top left: **World**, the map
  editor (workspaces Layout, Sculpting, Painting, 3D, 3D Sculpt, Maps,
  Minimal), and
  **Terrain**, the terrain generator (Graph and Render). Each side
  remembers the workspace it was left on.
- Workspaces are tabs of arrangements; duplicate one with `+`, right-click
  it to reset it (back to how it was at first; a copy goes back to what it
  was copied from), duplicate it or delete it. The layout is remembered.

A slider is a slim track with its value after it: click the track to jump
there, drag the thumb (Shift: finer), drag the value sideways, or click it to
type one. A number clicked to be typed in gets the caret where it was clicked
(in any number field), so that a digit adds to it rather than replacing it. A
narrow Tools area keeps each row of tools on one line, its name under the
pointer; the tools have icons drawn for what they do to the land.

What Massif did and what runs in the background (saving, exporting, the
automatic tools over a whole map) shows at the bottom right of the window,
over the areas, with a **Cancel** for long tasks.

**Help ▸ Massif Help** (F1) opens the help: every part of Massif, topic by
topic, with a search, and the keyboard shortcuts.

### Menus

**File**, **Edit**, **View** and **Help** hold every command, each item with
its shortcut beside it. An item is greyed out when it cannot be used: what
acts on the map waits while the home screen is shown, and everything but
Quit waits while a dialog is open or a map is being opened.

On a Mac they are in the menu bar at the top of the screen, as in every Mac
app, and the window's top bar keeps only its buttons. **About Massif**,
**Settings…** (⌘,) and **Quit Massif** (⌘Q) are in the **Massif** menu,
where Mac apps keep them, and **Help** has the system's search of the menus.
⌘Q asks first when the map has changes that are not saved, as **File ▸
Quit** does. On Windows and Linux the menus sit at the top left of the
window, with Ctrl for ⌘.

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

## Settings

**Settings** (⌘, — in the **Massif** menu on a Mac, in **View** on Windows
and Linux — or the home screen's sidebar) has seven sections, in a window of
their own:

- **Appearance**: the theme, the accent colour (the colour of what is
  chosen, of the main buttons and of the home screen's art: eight colours, or
  any other), the home screen's picture moving or still.
- **Accessibility**: **High contrast** — text black or white, muted text
  past 7 to 1 on the cards, lines and the edges of controls past 3 to 1, the
  accent darkened or lightened until it reads, links underlined, and the
  home screen's picture calmer behind its words; **Reduce motion** — the map
  and the 3D views stop where they are let go and go straight where they are
  sent (no gliding on, no coasting, no easing), the home screen's picture and
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
- **Developer**: developer mode adds a **Developer** panel to every area's
  menu, as the brushes or the layers: frames a second and the time each
  takes (and how much of it is Massif's own work), the memory the system
  counts for Massif and the processor's time, in charts that run live; then
  what is kept decoded, the map, the tasks running, the graphics card, and
  why Massif draws again.

## New worlds and export

A new map (the home screen) takes a name, a size (width × depth, in
multiples of 128 blocks, up to 65,536), its **height** — Minecraft's own (−64 to 320) or
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

**File ▸ Export** writes the world to a Minecraft saves folder for the
version of the game you choose — the newest one installed on the computer
to begin with — with its game mode, difficulty, cheats, hardcore,
structures beyond the map, world border, spawn point, what surrounds the
map (vanilla terrain or void) and data packs (copied in and enabled).
Versions before 1.18 hold 256 blocks of height: the map is cut below 0 and
above 255, and the dialog says so. **Test Export** writes only the 3 × 3
tiles around the centre of the map view to a separate folder, to check the
result in the game quickly.

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

## Ores

The export writes the map's chunks finished, so Minecraft adds no ore to
them: the export lays its own, in the rock, as the game lays them since
1.18. **Edit ▸ Ores…** sets them — for each ore, bands of heights where its
veins start: few at the ends of a band, the most over its peak (a
trapezoid: a triangle when the peak is one height, an even spread when it is
the whole band), so many veins a chunk, so many blocks a vein.

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
**Lay ores** turns them all off.

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

## Brushes

Brushes work as in WorldPainter. A brush has a shape (one of the twelve
built-in brushes, a terrain stamp, or an image), a radius, an intensity and
a rotation:

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
- **Only on / except on**: terrains, biomes, water or dry land.
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

## Automatic biomes and paint

Two tools of the Paint group work from what the map already is, with a
brush (and its options) or over the whole map at once:

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
brush, the columns that would change, in their new colour. A stroke changes every column in the core of the brush
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

**Rules…** (in Tool Settings, or Edit ▸ Automatic rules) opens both tables:
the depths, heights and slopes the land is read with, the biome of each
kind of land, the three terrains of each biome. They are kept with the
settings. Forests are told from the ground for now (podzol, coarse dirt);
trees will count when layers exist.

## The whole map at once

**Edit ▸ Whole map** runs one of the terrain graph's filters over the whole
map, without opening the graph:

- **Erosion**: rain runs down the slopes and carries soil from the heights
  to the valleys — gullies, worn flanks, fans of silt (the Erosion node:
  rain, erosion, deposition, capacity, evaporation, brush, seed).
- **Thermal erosion**: loose rock slides wherever a slope is steeper than
  it can hold — scree at the feet of cliffs, softer ridges.
- **Rivers**: traced where the water of the whole map runs, dug into the
  bottoms of the valleys, wider downstream, with their lakes, keeping to
  the water already on the map; their beds in gravel.
- **Lakes**: water fills the hollows up to where they spill (with **Fill**,
  their ground rises instead).
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

## Ground covers

The **Ground cover** tool (Paint group) paints what grows on the ground:
grass, tall grass, flowers, ferns, mushrooms, dead bushes, snow and seagrass
come with every world, and **New cover** makes one of your own. A cover is
a mix of blocks with their weights — the flowers of a meadow, or any block
at all, typed or picked from the game's blocks — with a **density** (the
share of the painted ground where something grows), a **height** (cane and
cactus 1 to 3 blocks), **only on soil** (grass, dirt, podzol, moss, mud:
plants pop off sand and stone in the game) and **under water** (seagrass,
coral). The right button takes a cover off. The brush, its intensity and its
options (height band, slope, only on a terrain…) apply as with any paint;
the eyedropper picks the cover under the cursor.

Covers grow on the topmost surface of a column — the ground, a floating
island's top, the floor a carve opens — never under an overhang. Where each
plant grows is decided column by column from the world's seed, so the map,
the 3D view and the export agree: zoomed in, the map shows every plant in
its own colour (dandelions yellow, poppies red, cornflowers blue…), and the
3D view the same. Tall plants are written in their two halves; versions of
the game without a plant get the closest they have (short grass is grass
before 1.20.3, cornflowers are blue orchids before 1.14, and before 1.13
plants are numbered, tall ones as double plants).

## Custom objects

The **Objects** tool (in Decorate, beside Ground cover) paints where objects
stand. **Import objects…** in its settings — or files dropped on the window
— reads them from:

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

Where each object stands is decided from the world's seed, square by
square, where the layer is painted on the top of the land: the map draws the
objects from above with their shadows when zoomed in, the 3D view block by
block in their colours (the game's own when its blocks are loaded), and the
export writes them whole, across chunks and tiles, for every version the
blocks exist in. Block entities (chests' contents, signs' text) and entities
are not kept.

## Rivers

The **River** tool (Water group) lays a river out from its source:

- **Click** on land: the river springs there, and its water finds its own
  way down — down the slope, a block at a time; a hollow it meets fills up
  to where it overflows, and it goes on from there — to the sea, into water
  already there, or off the map. Before the click, hovering shows where the
  water would go.
- **Click elsewhere**: that is where it ends. The way there is worked out
  over the land, as water would take it: along the valleys, climbing as
  little as it can, into the water it meets.
- **Drag the river** anywhere along its course to bend it: the point held
  becomes a point it must pass, and the way on either side follows the land
  again. Drag the source, those points or the end to move them; right-click
  one to take it away (the end: back to where the water goes by itself).
- **Dig river** (or Enter) digs it, one undoable step; **Cancel** (or Esc)
  drops it. The bar over the map says how long it is and where it goes.

Its settings: **Width** at the source, **Flow** (how much it grows
downstream: up to five times wider and two and a half times deeper),
**Depth**, **Erosion** (how much it wears the land beside its bed: a wider
valley), **Banks**, **Lakes** (which hollows fill: deep ones only, or every
one, or none — through the others it cuts a gorge), the terrain of its bed
and a river biome on its water.

- The course is smoothed into a curve through its points.
- Water runs from the source down. Its level along the river is the lowest
  the land allows so far, a
  little under the banks: it never rises downstream, whatever the bumps.
- The bed is `depth` deep in the middle and rises to the water's edge;
  banks of the chosen width climb back to the ground, smoothly. Only the
  ground is lowered: nothing is raised.
- A course that starts on a river is a branch, leaving it at its level; one
  that ends on a river or a lake is a tributary, meeting it at its level.
  Two branches that meet again make a split stream.
- The bed gets a terrain (gravel, sand…), the water a river biome.

The water's level is a whole block per column, so it stands in steps
along the river: still water, as Minecraft keeps it; mods that let water
flow over water (Streams Reflowing) make a current of it.

## Height maps

- **File ▸ New World from Height Map…** makes a world of a greyscale image
  — a 16-bit PNG from Gaea, World Machine or an image editor (JPEG, and raw
  square `.r16` / `.r32` are read too): what height black and white stand
  for, how many blocks a pixel covers, the sea's level; terrains are painted
  from height and slope. The home screen offers it too.
- **File ▸ Export Height Map…** saves the map's surface (with its levels and
  what was sculpted) as a 16-bit PNG, black the bottom of the world, white
  its top.
- In a graph, the **File input** node brings in a file as relief: a height
  map (stretched over the terrain, black and white set to any height), a
  WorldPainter `.world` or a `.massif` (fitted to the terrain, or read at
  its own place). Its Height, Water and Colour outputs work like the Map
  node's, and the file is read again when it changes. **Height map…**
  beside Convert to voxels builds the terrain at full size and saves its
  relief as a 16-bit PNG instead of writing it to the map.

## Real blocks

Massif reads the blocks of the game from your own installation: the client
jar of a released version, found where the official launcher, CurseForge or
Prism keep them (on any drive), or chosen by hand. Nothing of the game is
copied into Massif or the repository; the jar is read in place, in a
fraction of a second, every time Massif starts.

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

## 3D Sculpt

Sculpting in 3D has its own editor, **3D Sculpt** (its workspace sits next
to 3D; the 3D View's **Sculpt** button opens it): a 3D view with its tools
along the top — **Sculpt** (the left button adds matter, the right one
digs), **Smooth** and **Paint** (the terrain chosen in Terrain or Blocks, or
a noise of several; the right button puts back what the height calls for) — with the
brush of every tool: its shape (custom images too, turned as they are),
size, intensity and rotation are those of **Brush Settings**, which the
3D Sculpt workspace shows beside its view.

- Sculpt pushes the surface out or in under the brush, along its slope, a
  layer at a time — 16 blocks a second at full strength —, the brush's
  shape setting how far each point goes: a click adds a bump, holding grows
  it.
- Smooth melts bumps and steps away without wearing the land down: it blurs
  the matter, then pushes it back out a little (Taubin's smoothing), so a
  hill keeps its height and a hollow its depth.
- Shift + drag moves the view, Alt + drag (or the middle button) turns it.
  The wheel brings the view closer around what it turns about; Shift +
  wheel moves forward over the land; Alt + wheel flies toward the point
  under the cursor; ⌘ (Ctrl) + wheel sizes the brush. In 3D a brush
  sculpts 64 blocks around at most: the work of a dab grows with the cube
  of its size (a larger brush works as 64, and the toolbar says so).
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
  - Water slows and lets swim, and under it the world turns blue and dim,
    as in the game; matter sculpted over the player lifts them, and a fall
    off the map puts them back where they last stood. Player off, the
    orbit camera comes back from the player's eyes.

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

## Cards

Hover a tool and its card shows: a little looping animation of the tool at
work on a patch of land drawn like the map — a hill rising under the brush,
flowers spreading, trees growing where they are painted, a river laid out
point by point and dug, water rising in a basin — then what it does, what
each gesture and key does with it, and a remark. A brush's card shows the
land it raises, shaded and contoured on a block of ground, turning slowly
from side to side, so that its shape is easy to make out. (As in the user's
WorldPainter fork, whose cards these are.)

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

## Controls

Massif follows WorldPainter's defaults.

| Input | Action |
|---|---|
| Left / right button (or Alt + left) | Apply the tool / its inverse (lower, erase, drain…) |
| Scroll | Brush size (Shift: one block at a time) |
| ⌘ + scroll (the Windows key on a PC keyboard) | Turn the brush, 5° a notch (Shift: 1°); Ctrl + Shift + scroll on Windows and Linux |
| Ctrl or Alt + scroll, pinch | Zoom the map |
| Middle button, Space + drag | Move the map |
| `−` `+` | Brush size |
| `1` … `9`, `0` | Intensity 10 % … 100 % |
| `[` `]` | Turn the brush |
| `R` `L` | Turn the light of the map |
| ⌥H ⌥A ⌥S ⌥P ⌥R ⌥F ⌥Y | Height, Flatten, Smooth, Paint, Spray, Flood, Eyedropper |
| ⌘Z / ⇧⌘Z, ⌘S, ⌘O, ⌘N, ⌘E | Undo / redo, save, open, new, export |
| Esc | Cancel the stroke being drawn |

In the 3D view:

| Input | Action |
|---|---|
| Left drag / right drag | Turn / move the view |
| Scroll, pinch | Come closer / go back |
| W A S D or arrows, Q / E | Fly over the map, down / up (Shift: faster) |
| Double-click | Look at the point under the cursor |
| Header buttons | Look at the middle of the map view; show the whole map |
| With Build, Dig, Smooth (3D) | Left sculpts where the cursor points, right inverts; middle drag or Alt + drag turns, Shift + middle drag moves |
| 3D Sculpt's **Player**, cursor free (Axiom's editor) | Left drag looks around; right uses the tool under the cursor (Alt: inverts); middle picks the block; Ctrl (⌘) + left turns about the point, Ctrl + right moves, Ctrl + middle sizes the brush; the wheel sets the flight speed |
| 3D Sculpt's **Player**, mouse taken (a click) | The mouse looks around; right uses the tool at the crosshair, left inverts; Esc gives the mouse back |
| 3D Sculpt's **Player**, keys | W A S D walk, Ctrl or W twice sprints, Space jumps (twice: flies, up), Shift or Q sneaks (flying: down) |

### Moving smoothly

The map and the 3D view are made to follow the hand:

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
starting again after a start that hung does not lose it. Send both when
reporting a problem: the last line says where Massif stopped.

- The first frames are followed step by step until one is on the screen:
  the interface laid out, its shapes and textures sent to the graphics card,
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
- When the start stays 12 seconds in one step once its graphics card is
  chosen, the graphics driver is most likely stuck: the graphics API Massif
  was drawing with is set aside on this computer, and the next starts draw with another one — on Windows
  DirectX 12 after Vulkan, then OpenGL. On Windows a message offers to start
  again with it right away. A start that died without a word is found by
  the next one, which sets its API aside in the same way. The APIs set aside
  are listed in `graphics.txt` beside the log: delete the file to let Massif
  try them again (after a driver update, say).
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
