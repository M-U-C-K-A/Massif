# Coming from WorldPainter

[← Guide](guide.md)

Massif is made to take over from WorldPainter: it opens your `.world`
files and your `.layer` files, reads your custom brushes in place and runs
your scripts. Most of WorldPainter's tools are here under the same name or
a close one. This page says where each one is.

## Start as in WorldPainter

The first time Massif starts, it asks **Coming from WorldPainter?**

- **Yes, set it up like WorldPainter** opens the **WorldPainter**
  workspace. The 2D map is big in the middle, the tools are on the left
  (with Tool Settings and Brush Settings under them), and Terrain, Layers
  and Brushes are on the right. It also switches the keyboard shortcuts to
  WorldPainter's. Once a map is open, a short tour of eight steps shows
  where your tools are.
- **No, I'm new to both** keeps Massif's own layout and its tour.

You can change either choice at any time:

- the **WorldPainter** tab, at the end of the World side's workspaces,
  shows the workspace;
- **Settings ▸ Keyboard ▸ WorldPainter** sets the shortcuts;
- **Help ▸ Where are my WorldPainter tools?** shows the tour again.

The tool cards also say what WorldPainter calls each tool ("In
WorldPainter: Height, Raise Mountain, Raise Pyramid").

Nothing is taken away. The 3D view and the node graph stay one click away:
pick them as an area's editor, or switch to the **Terrain** side at the
top left.

## Where each thing is

| WorldPainter | Massif | Where |
|---|---|---|
| Open a `.world` | **Open…** (`⌘O`) or **Import WorldPainter World…** | File menu, top bar, home screen. The `.world` is only read: save your work as a `.massif` |
| New World | **New World…** (`⌘N`) | File menu, home screen |
| Import ▸ Height map | **New World from Height Map…** | File menu, home screen |
| Height (`⌥H`), Raise Mountain (`⌥M`), Raise Pyramid | **Raise / Lower**; the brush gives the shape | Tools ▸ Sculpt terrain; shapes in Brushes |
| Smooth (`⌥S`), Flatten (`⌥A`) | **Smooth**, **Flatten** | Tools ▸ Sculpt terrain |
| Path | **Road** | Tools ▸ Sculpt terrain |
| Pencil (`⌥P`), Spray Paint (`⌥R`) with a terrain | **Paint terrain** | Tools ▸ Paint; terrain in the Terrain panel, every block in Blocks |
| Fill | Select the area, then paint it; or **Auto paint** for the whole map by rules | Tools ▸ Select, Edit menu |
| Paint a biome; Automatic Biomes | **Paint biome**; **Auto biome** | Tools ▸ Paint; Biomes panel |
| The Layers palette (Frost, trees, plants, Resources…) | The **Layers** panel and the Decor tools: Ground cover, Objects, Tree sizes, Underground pockets, Combined layer | Right of the WorldPainter workspace |
| Custom layers, Import layer (`.layer`) | **Import WorldPainter layer…** | Top of the Layers panel |
| Custom terrains and materials | Every block of the game and of your mods; block palettes | Blocks panel; Settings for mods |
| Brushes, custom brushes folder | **Brushes**, **Custom Brushes** (WorldPainter's brushes folders are read in place) | Brushes panel |
| Brush intensity `1`–`0`, rotation `⌥3` `⌥4` `⌥9` | The same keys | Brush Settings |
| Flood (`⌥F`) | **Flood** | Tools ▸ Water |
| Sponge | **Flood** with `Shift` and the right button, or **Water brush**'s right button | Tools ▸ Water |
| River layer | **River**, and **Edit ▸ Whole map ▸ Rivers** | Tools ▸ Water, Edit menu |
| Select, Select with Brush | **Rectangle**, **Ellipse**, **Lasso** and **Selection brush**; `⌘A`, `⌘D` | Tools ▸ Select, Edit menu |
| Text, Annotations | **Annotations** | Tools ▸ Paint |
| Caverns, Chasms, Tunnel layers | **Caves…**, and levels | Edit menu, Levels panel |
| Global Operations | **Whole map** | Edit menu |
| World and dimension properties | Name, seed, sea level and size | Info panel |
| 3D View | The **3D View** and **3D Sculpt** editors | 3D and 3D Sculpt workspaces |
| Export (`⌘E`) | **Export to Minecraft…** | File menu, top bar |
| Export ▸ Height map | **Export Height Map…** | File menu |
| Scripts | **Run Script…**: WorldPainter's `.js` as they are, or Massif's Lua | File menu ([Coming from WorldPainter scripts](coming-from-worldpainter-scripts.md)) |
| Undo `⌘Z`, Redo `⌘Y`, zoom `⌘+` `⌘−` `⌘0`, light `R` `L` | The same keys with the WorldPainter shortcuts | Settings ▸ Keyboard |

On Windows and Linux, `⌘` is `Ctrl` and `⌥` is `Alt`.

## What your layers become

A `.world` keeps its terrains and its layers apart, as WorldPainter does.
Your terrains, custom ones included, stay the map's terrains, and each layer
becomes a layer of the Layers panel, painted where it was:

- **Custom ground covers** (a block of water, snow layers, Conquest
  Reforged's grass layers…) become ground covers of their blocks, as high as
  they are thick. One that takes the place of the top of the land lies on
  the ground lowered under it.
- **Every layer of a block stays on it**, with its own intensity, as
  WorldPainter keeps them: a block with a forest at 50 %, bushes at 15 %
  and three ground covers keeps all of them. Covers lie over each other as
  WorldPainter lays them (custom ground covers on the ground, plants on
  them, frost over everything), and the export grows each in turn; each
  tree layer stands its trees thinned to its intensity there. The Info
  panel lists them all; rubbing one out leaves the others, and painting a
  layer over the block takes the place of all of that slot's.
- **Covers of layers** — ground covers whose blocks are the layers of a
  block (`conquest:grass_block_layer[layers=3]`, snow layers, covers mixing
  several of them) — are listed first in the import report, ticked,
  whether or not Conquest Reforged is read in the Blocks panel (the map's
  own blocks are known). **Make them the ground's own** turns those
  you keep ticked into the land itself: each column rises by its layers'
  eighths, its top becomes the full block they are layers of (its terrain
  stays when it already is), and Smooth terrain lays the same layers on
  export, now from the land's true height, which the map, the 3D view, the
  brushes and the smoothing all follow. It is one step to undo. Layers of
  a block that has no full block of its own (`grass_and_sand_layer`) use
  eight of them as their full block; a mix's other blocks (small stones)
  stay a cover, and plants or frost painted over the layers stay. **Edit ▸
  Covers of Layers to Ground…** does the same at any time.
- **Forests, tree pack layers and custom object layers** become tree and
  object layers with about as many trees as WorldPainter's export stands:
  the import plays WorldPainter's placement with your objects' own shapes
  and sets the spacing and density that match, at every strength you
  painted.
- **Plant layers, Frost, Lava, Void, Annotations, pockets, combined
  layers, caves, tunnels and River** come as described in
  [Layers](layers.md).

Your **overlay images** (View ▸ Overlays) become reference images, at the
same place, scale and opacity. Their files are looked for where WorldPainter
had them, then by name beside the `.world`; one not found keeps its place,
and **Find its file…** in its settings links it again.

**Custom biomes** come in as biomes of the map, painted where they were
and exported by their names, which are their ids
(`terralith:alpine_highlands`): the game shows them with the data pack or
mod that defines them. One named otherwise keeps its name, its id made of
it. Right-click a biome of the map in the Biomes panel to rename it or
set its colours (on the map, and of grass and leaves in the 3D view and
True Colour). **Custom annotation layers** become annotations in a colour of
the map's own. Every custom layer keeps the colour WorldPainter shows it
in (on the map and in the Layers panel), and a ground cover its opacity.

The import report lists any layer that was not carried over, and any
overlay image not found. It is kept with the map: **File ▸ Import
Report…** shows it again, its covers of layers included.

## What Massif adds

- **A 3D view** of the world as it will stand in the game, redrawn as you
  paint, and **3D Sculpt** to build, dig and walk in it.
- **The Terrain side**: a graph of nodes that grows relief and erodes it,
  then sends it to the map ([Terrain graphs](terrain-graphs.md)).
- **Smooth terrain**: slabs, stairs and layers on top of the land
  (**Edit ▸ Smooth terrain…**).
- **Scripts in Lua** with Massif's own API, beside WorldPainter's
  ([Scripts](scripting.md)).
- Workspaces you arrange yourself, save points, a survival check of the
  map, the relief of real places, and more: see the [Guide](guide.md).
