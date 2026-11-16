// How the guide is laid out on the site: its groups, in reading order, and a
// two-line summary at the top of each page. Pages are named by their
// document ("g" docs/guide.md, "t" docs/terrain-graphs.md) and the slug of
// their `##` heading ('' for the document's opening). A section added to the
// docs and not listed here still shows, under "More".

export const groups = [
  ['Getting started', [
    ['g', '', 'Welcome', 'What Massif is made of, and where to start: the home screen, the window, then the brushes.'],
    ['g', 'the-home-screen', '', 'New map, open, import from WorldPainter, recent maps: everything starts from here.'],
    ['g', 'the-window', '', 'Areas you split and fill with any editor, as in Blender, and the menus over them.'],
    ['g', 'controls', '', "WorldPainter's mouse and keys, kept: how to paint, move around and zoom."],
    ['g', 'cards', '', "Hover a tool to see a little animation of what it does to the land."],
  ]],
  ['Painting', [
    ['g', 'brushes', '', "WorldPainter's brushes, stamps and your own images, and the options that limit where they act."],
    ['g', 'selections', '', 'Mark the part of the map the tools change: rectangle, ellipse, lasso, brush, or by terrain, height or slope.'],
    ['g', 'noise-painting', '', 'Lay several terrains or blocks at once, mixed by a noise: patches, veins, blobs, cells or stripes.'],
    ['g', 'automatic-biomes-and-paint', '', 'Biomes read from the shape of the land, and the ground painted from its biome.'],
    ['g', 'layers', '', 'Ground covers, object layers and terrain mixes, all listed in one panel.'],
    ['g', 'packs', '', 'Trees, plants and block mixes ready to paint, and packs to share with others.'],
    ['g', 'ground-covers', '', 'Grass, flowers, snow or blocks of your own, grown over the ground.'],
    ['g', 'custom-objects', '', 'Trees, rocks and ruins from WorldEdit, Axiom, Litematica and structure files, painted as layers.'],
    ['g', 'real-blocks', '', "Every block of the game, read from your own Minecraft with its textures."],
  ]],
  ['Shaping the land', [
    ['g', 'rivers', '', 'Click a source and the water finds its own way down to the sea, then is dug into the land.'],
    ['g', 'roads', '', 'Click points for a smooth road, graded into the land and carried over water on bridges.'],
    ['g', 'caves', '', 'Tunnels, ravines and caverns carved into the rock, with lakes of water or lava.'],
    ['g', 'real-places', '', 'A world, or a part of one, from the relief of a real place on the Earth.'],
    ['g', 'height-maps', '', 'Worlds from greyscale images, and the map saved as one.'],
    ['g', 'the-whole-map-at-once', '', "The terrain graph's filters run over the whole map: erosion, rivers, lakes, terraces."],
    ['g', '3d-sculpt', '', 'Push, smooth and paint the rock itself in 3D, and walk the land as a player.'],
  ]],
  ['Terrain graph', [
    ['t', '', 'Terrain graphs', 'A node-based terrain generator in the manner of Gaea, and how a graph becomes blocks.'],
    ['t', 'the-graph-workspace', '', 'The graph below, its viewport above, the selected node’s settings beside: as in Gaea.'],
    ['t', 'the-render-workspace', '', 'The viewport on stage with a sun, clouds, haze and a sea, and the image saved.'],
    ['t', 'templates', '', 'Ready-made landscapes to start from, each an ordinary graph to change.'],
    ['t', 'editing-a-graph', '', 'Add, wire, group and preview nodes, with the keys and the mouse.'],
    ['t', 'nodes', '', 'About sixty nodes: landforms, primitives, filters, simulations, masks, climate, colour.'],
    ['t', 'rock-over-the-void', '', 'Overhangs and floating islands from a graph, written as levels of the map.'],
    ['t', 'working-on-a-map-that-exists', '', 'Start a graph from the map you painted or imported, and keep what is there.'],
    ['t', 'rivers', '', 'Whole river systems traced in the valleys, with lakes in the hollows.'],
    ['t', 'climate', '', 'Temperature and humidity from the sea, the wind and the mountains, as masks.'],
    ['t', 'tectonic-plates', '', 'Ranges, trenches and island arcs raised by plates running into each other.'],
    ['t', 'over-the-map-not-into-it', '', 'Keep the ground you painted: what a graph adds or digs goes to levels of its own.'],
    ['t', 'size', '', 'How much of the world a graph covers, from the map itself up to 32k blocks.'],
    ['t', 'convert-to-voxels', '', 'The graph written into the map as one step to undo: heights, blocks, water, biomes.'],
  ]],
  ['Saving and export', [
    ['g', 'new-worlds-and-export', '', 'A new map’s size, heights and version, and the world written for the game you play.'],
    ['g', 'ores', '', 'Ores laid in the rock as Minecraft lays them, their bands set on a cut through the map.'],
    ['g', 'survival-check', '', 'The map read as a survival player would: what breaks it, what is missing, how to fix it.'],
    ['g', 'autosave', '', 'A copy every few minutes, offered back after a crash.'],
    ['g', 'save-points', '', 'Keep the map under a name, to restore it, open it or see what changed since.'],
  ]],
  ['Reference', [
    ['g', 'settings', '', 'Theme, accent, sounds, memory, autosave and the other settings, section by section.'],
    ['g', 'new-versions', '', 'How Massif tells you a new version is out, and what it sends to ask.'],
    ['g', 'in-the-finder-and-explorer', '', 'Your maps shown as their files’ icons.'],
    ['g', 'if-massif-does-not-start', '', 'Where the log is, what it says, and how to choose another graphics API.'],
  ]],
];
