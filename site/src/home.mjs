// The home page. Its facts come from the README and the release notes.

const ic = (d) => `<svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${d}</svg>`;

// The big sections, each with a Hairline figure of docs/assets.
export const showcase = [
  {
    fig: 'brush', alt: 'A brush raises a ridge across a massif of columns, then the stroke is undone', guide: 'brushes',
    kicker: 'Brushes', title: 'Paint from above, as in WorldPainter',
    text: "WorldPainter's way of working, kept: its twelve brushes and their profiles, 22 terrain stamps (peaks, ranges, volcanoes, mesas…), your own greyscale images, a radius up to 4,096 blocks, and its mouse and keyboard. Your WorldPainter brushes folder is read as it is.",
    points: ['A band of heights, a terrain or a biome', 'Steep or gentle slopes, the side a slope faces', 'Hollows or bumps'],
  },
  {
    fig: 'graph', alt: 'A terrain graph: pulses run from two sources through a combine and an erosion to the output, whose terrain rises', guide: '', graphs: true,
    kicker: 'Terrain graph', title: 'Grow the land from a node graph',
    text: 'The Terrain side is a node-based generator in the manner of Gaea: about sixty nodes (tectonic plates, mountains, volcanoes, canyons, dunes, erosion, snow, rivers, climate, biomes, colour), landscapes to start from, and a viewport with a sun, clouds, haze and a sea.',
    points: ['Builds up to 16,384 × 16,384', 'Colours become the closest real blocks', 'Can start from the map you painted'],
  },
  {
    fig: 'islands', alt: 'Two islands of rock float over a field of columns', guide: '3d-sculpt',
    kicker: '3D Sculpt', title: 'Rock over the void, sculpted in 3D',
    text: 'A map is a stack of levels: solid levels add rock (overhangs, arches, floating islands), carve levels take it away. Where that is not enough, 3D Sculpt pushes, smooths and paints the matter itself, and Player walks the land with the game\'s own blocks and light, driven as in Axiom.',
    points: ['Sparse 16³ voxel chunks', 'The player\'s view looks like Minecraft', 'Axiom\'s mouse and keys'],
  },
  {
    fig: 'rivers', alt: 'A river runs from its source down a valley of columns to the sea', guide: 'rivers',
    kicker: 'Rivers', title: 'Rivers that follow the land',
    text: 'Click a source and the water finds its own way down, filling the hollows it meets, to the sea. Drag its course to bend it, and it is dug with banks that ease into the land, from a brook to hundreds of blocks wide. Or let a graph trace whole river systems in the valleys of a map.',
    points: ['Lakes in the hollows', 'Banks eased into the land', 'Water painted in WorldPainter kept'],
  },
  {
    fig: 'blocks', alt: 'Six blocks of the game on a slab, picked one after another', guide: 'packs',
    kicker: 'Packs and layers', title: "The game's own blocks, ready to paint",
    text: 'Every block of the Minecraft version you have installed (1,288 in 26.3) is read from the game itself, with its textures and colours. The Packs tab holds sixteen kinds of trees, twenty-two presets of plants and thirty-nine mixes of blocks; the Layers panel lists every cover, object and mix of the map.',
    points: ['Trees from WorldEdit, Axiom, Litematica…', 'Cliff strata in horizontal beds', 'Hide a layer, keep it in the export'],
  },
];

// The smaller cards. `since` marks what a release brings: shown as
// "Coming in X" until X is out, then "New in X" while it is the latest.
export const features = [
  { title: 'Roads', since: '0.6.0', guide: 'roads', icon: ic('<path d="M5 21c1-6 4-8 7-9s6-3 7-9"/><path d="M9 21c1-4 3-6 6-7" opacity=".45"/><path d="M3 18c1.5-5 4.5-7.5 7.5-8.5S16 7 17 3" opacity=".45"/>'),
    text: 'Points clicked on the map make a smooth road, graded into the land: it cuts the humps and fills the hollows so it never climbs steeper than you allow, and crosses water on a bridge.' },
  { title: 'Caves', since: '0.6.0', guide: 'caves', icon: ic('<path d="M3 20h18"/><path d="M5 20c0-6.5 3-10.5 7-10.5s7 4 7 10.5"/><path d="M9.5 20c0-3 1.1-5 2.5-5s2.5 2 2.5 5"/>'),
    text: 'Winding tunnels with pillared chambers, deep ravines cut open to the sky, caverns holding a lake of water or lava. The same seed always carves the same caves.' },
  { title: 'Selections', since: '0.6.0', guide: 'selections', icon: ic('<rect x="4" y="4" width="16" height="16" rx="3" stroke-dasharray="3 3"/><path d="M9 12h6M12 9v6"/>'),
    text: 'Rectangle, ellipse, lasso or brush, or select by terrain, height or slope, with a feathered edge. Every tool then works only inside, and the export can write only the selection.' },
  { title: 'Ores and the survival check', since: '0.5.0', guide: 'ores', icon: ic('<path d="M12 3 19 10 12 21 5 10Z"/><path d="M5 10h14M9 3.8 12 10l3-6.2"/>'),
    text: 'Ores laid as Minecraft lays them, their bands dragged on a cut through the map. Analyse the map reads it as a survival player would: what breaks survival, what is missing.' },
  { title: 'Real places', since: '0.6.0', guide: 'real-places', icon: ic('<circle cx="12" cy="12" r="9"/><path d="M3 12h18M12 3c2.5 2.6 3.8 5.6 3.8 9s-1.3 6.4-3.8 9c-2.5-2.6-3.8-5.6-3.8-9S9.5 5.6 12 3Z"/>'),
    text: 'A world from the relief of any place on Earth, ocean floors included, or that relief put into your map: search it by name, then choose the blocks a metre and how tall it stands.' },
  { title: 'Autosave and save points', since: '0.6.0', guide: 'autosave', icon: ic('<path d="M3.5 12a8.5 8.5 0 1 0 2.6-6.1L3.5 8.5"/><path d="M3.5 3.5v5h5"/><path d="M12 7.5V12l3 2"/>'),
    text: 'A copy every few minutes, offered back after a crash. Save points keep the map under a name, to restore it later or see on the map what changed since.' },
  { title: 'Export for the version you play', guide: 'new-worlds-and-export', icon: ic('<path d="M12 3v11M7.5 9.5 12 14l4.5-4.5"/><path d="M4 15v3a3 3 0 0 0 3 3h10a3 3 0 0 0 3-3v-3"/>'),
    text: 'From 1.7.10 to 26.3 and whatever is installed, each version in the chunk format it reads, worlds up to 2,032 blocks high, with ores laid in the rock.' },
  { title: 'WorldPainter import', guide: 'the-home-screen', icon: ic('<path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8Z"/><path d="M14 3v5h5M12 11v6M9 14l3 3 3-3"/>'),
    text: '.world files read directly, without Java: a 100 MB world in about a second. Your brushes come along too.' },
  { title: 'Huge maps', guide: '', icon: ic('<rect x="3" y="3" width="18" height="18" rx="3"/><path d="M3 9h18M3 15h18M9 3v18M15 3v18"/>'),
    text: '60,000 × 50,000 blocks, generated in a minute and edited at once, and the whole map in 3D, drawn by a GPU ray tracer at any distance.' },
  { title: 'Automatic biomes and paint', guide: 'automatic-biomes-and-paint', icon: ic('<path d="M5 19c0-8 5-14 15-14 0 10-6 15-14 15"/><path d="M5 19c3-4 6-7 10-9"/>'),
    text: 'Biomes read from the shape of the land, the ground painted from its biome, by rules you can change.' },
];

const stats = [
  ['60,000 × 50,000', 'blocks, the largest map'],
  ['1.7.10 → 26.3', 'versions it exports to'],
  ['~60', 'nodes in the terrain graph'],
  ['1,288', 'real blocks, read from the game'],
];

export function home({ url, esc, REPO, icon, figure, dl, latest, cmp, config, guideLink }) {
  const v = latest?.version;
  const badge = (since) => {
    if (!since || !v) return '';
    if (cmp(v, since) < 0) return `<span class="tag accent">Coming in ${since.replace(/\.0$/, '')}</span>`;
    if (cmp(v, since) === 0) return `<span class="tag accent">New in ${since.replace(/\.0$/, '')}</span>`;
    return '';
  };
  const showcaseHtml = showcase.map((s, i) => {
    const link = s.graphs ? url('/guide/terrain-graphs/') : guideLink(s.guide);
    return `<section class="show${i % 2 ? ' flip' : ''}">
  <div class="show-fig">${figure(s.fig, s.alt)}</div>
  <div class="show-text">
    <p class="eyebrow">${s.kicker}</p>
    <h3>${s.title}</h3>
    <p>${s.text}</p>
    <ul class="ticks">${s.points.map((p) => `<li>${p}</li>`).join('')}</ul>
    ${link ? `<a class="more" href="${link}">Read in the guide ${icon.arrow}</a>` : ''}
  </div>
</section>`;
  }).join('\n');
  const cards = features.map((f) => {
    const link = f.guide ? guideLink(f.guide) : '';
    const inner = `<div class="card-top"><span class="card-icon">${f.icon}</span>${badge(f.since)}</div><h3>${f.title}</h3><p>${f.text}</p>`;
    return link ? `<a class="card" href="${link}">${inner}</a>` : `<div class="card">${inner}</div>`;
  }).join('');
  const shots = config.gallery.slice(0, 3).map((g) => `<a class="shot-mini" href="${url('/gallery/')}"><img src="${url('/assets/renders/' + g.file)}" alt="${esc(g.alt)}" loading="lazy"></a>`).join('');

  return `<section class="hero">
  <div class="hero-in">
    ${v ? `<a class="announce" href="${url('/whats-new/')}"><span class="dot"></span>Massif ${v} is out<span class="sep"></span>What's new ${icon.arrow}</a>` : ''}
    <h1>The map editor for Minecraft,<br class="br-lg"> made for huge worlds.</h1>
    <p class="lede">The successor to WorldPainter, for maps tens of thousands of blocks wide. Paint from above with its brushes, grow terrain from a node graph in the manner of Gaea, sculpt in 3D, and export a world Minecraft Java opens. Written in Rust.</p>
    <div class="cta">
      <a class="pill pill-solid big dl-auto" href="${url('/download/')}">${icon.down}<span class="dl-label">Download</span></a>
      <a class="pill pill-ghost big" href="${url('/guide/')}">Read the guide</a>
    </div>
    <p class="fine"><span class="dl-meta">${v ? `Version ${v} · ` : ''}Windows, macOS and Linux</span> · Free and open source</p>
  </div>
  <div class="hero-art">
    <div class="frame">
      <img class="on-dark" src="${url('/assets/fig/hero-dark.svg')}" alt="Massif's home screen: night scenes of mountains, and Massif's name in blocks" width="1200" height="400">
      <img class="on-light" src="${url('/assets/fig/hero-light.svg')}" alt="Massif's home screen: scenes of mountains at dawn, and Massif's name in blocks" width="1200" height="400">
    </div>
  </div>
</section>

<section class="stats" aria-label="In numbers">
  ${stats.map(([n, l]) => `<div><strong>${n}</strong><span>${l}</span></div>`).join('')}
</section>

<section class="section" id="features">
  <header class="section-head"><p class="eyebrow">Features</p><h2>WorldPainter's tools, a terrain generator, and the room for both.</h2><p class="lede">Shape and paint a map from above, build what a height map cannot hold, and see every block of it in 3D.</p></header>
  <div class="shows">${showcaseHtml}</div>
</section>

<section class="section">
  <header class="section-head"><p class="eyebrow">And more</p><h2>Everything else a map needs.</h2></header>
  <div class="cards">${cards}</div>
</section>

<section class="section">
  <header class="section-head row"><div><p class="eyebrow">Gallery</p><h2>Made with Massif.</h2></div><a class="more" href="${url('/gallery/')}">See the gallery ${icon.arrow}</a></header>
  <div class="shots">${shots}</div>
</section>

<section class="section">
  <div class="final">
    <h2>Start your next map.</h2>
    <p>Massif opens on its home screen: make a new map, open one, or import a WorldPainter world. F1 opens the help.</p>
    <div class="cta center">
      <a class="pill pill-solid big dl-auto" href="${url('/download/')}">${icon.down}<span class="dl-label">Download</span></a>
      <a class="pill pill-ghost big" href="${REPO}" rel="noopener">View on GitHub</a>
    </div>
  </div>
</section>`;
}
