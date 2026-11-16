// The home page: short lines, the animated figures of docs/assets, and the
// details left to the guide. Its facts come from the README and the release
// notes.

const ic = (d) => `<svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${d}</svg>`;

// The figure cards. `wide` cards take half the row, the others a third.
// `since` marks what a release brings: "Coming in X" until X is out, then
// "New in X" while it is the latest.
export const showcase = [
  { fig: 'brush', wide: true, guide: 'brushes', title: 'Paint from above', text: "WorldPainter's brushes, stamps, mouse and keys. Your brushes folder, read as it is.",
    alt: 'A brush raises a ridge across a massif of columns, then the stroke is undone' },
  { fig: 'graph', wide: true, graphs: true, title: 'Grow it from a graph', text: 'About sixty nodes in the manner of Gaea: plates, erosion, rivers, climate.',
    alt: 'A terrain graph: pulses run through a combine and an erosion to the output, whose terrain rises' },
  { fig: 'islands', guide: '3d-sculpt', title: 'Sculpt in 3D', text: 'Overhangs, arches, floating islands. Walk them as a player.',
    alt: 'Two islands of rock float over a field of columns' },
  { fig: 'rivers', guide: 'rivers', title: 'Rivers', text: 'Click a source: the water finds its way to the sea.',
    alt: 'A river runs from its source down a valley of columns to the sea' },
  { fig: 'roads', guide: 'roads', since: '0.6.0', title: 'Roads', text: 'Graded into the land, humps cut and hollows filled.',
    alt: 'A road graded through a field of hills' },
  { fig: 'select', guide: 'selections', since: '0.6.0', title: 'Selections', text: 'Lasso, brush or slope: every tool works only inside.',
    alt: 'A selection drawn on a field: only the columns inside it rise' },
  { fig: 'relief', guide: 'real-places', since: '0.6.0', title: 'Real places', text: 'The relief of anywhere on Earth, as a world or in your map.',
    alt: 'A flat field rising into the relief of a real range' },
  { fig: 'blocks', guide: 'packs', title: 'Packs', text: "The game's own blocks, trees and plants, ready to paint and share.",
    alt: 'Six blocks of the game on a slab, picked one after another' },
];

export const more = [
  { title: 'Caves', since: '0.6.0', guide: 'caves', text: 'Tunnels, ravines and caverns, from a seed.',
    icon: ic('<path d="M3 20h18"/><path d="M5 20c0-6.5 3-10.5 7-10.5s7 4 7 10.5"/><path d="M9.5 20c0-3 1.1-5 2.5-5s2.5 2 2.5 5"/>') },
  { title: 'Save points', since: '0.6.0', guide: 'save-points', text: 'Named versions to go back to, and autosave.',
    icon: ic('<path d="M3.5 12a8.5 8.5 0 1 0 2.6-6.1L3.5 8.5"/><path d="M3.5 3.5v5h5"/><path d="M12 7.5V12l3 2"/>') },
  { title: 'Ores', guide: 'ores', text: 'Laid as Minecraft lays them, and a survival check.',
    icon: ic('<path d="M12 3 19 10 12 21 5 10Z"/><path d="M5 10h14M9 3.8 12 10l3-6.2"/>') },
  { title: 'Export', guide: 'new-worlds-and-export', text: 'Every version from 1.7.10 to 26.3.',
    icon: ic('<path d="M12 3v11M7.5 9.5 12 14l4.5-4.5"/><path d="M4 15v3a3 3 0 0 0 3 3h10a3 3 0 0 0 3-3v-3"/>') },
  { title: 'WorldPainter import', guide: 'the-home-screen', text: '.world files, no Java: 100 MB in a second.',
    icon: ic('<path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8Z"/><path d="M14 3v5h5M12 11v6M9 14l3 3 3-3"/>') },
  { title: 'Huge maps', guide: '', text: '60,000 × 50,000 blocks, all of it in 3D.',
    icon: ic('<rect x="3" y="3" width="18" height="18" rx="3"/><path d="M3 9h18M3 15h18M9 3v18M15 3v18"/>') },
  { title: 'Auto biomes', guide: 'automatic-biomes-and-paint', text: 'Read from the land, painted by rules.',
    icon: ic('<path d="M5 19c0-8 5-14 15-14 0 10-6 15-14 15"/><path d="M5 19c3-4 6-7 10-9"/>') },
  { title: 'Command line', guide: '', text: 'New, inspect, import, export, no window.',
    icon: ic('<rect x="3" y="4" width="18" height="16" rx="3"/><path d="m7 9 3 3-3 3M12.5 15H17"/>') },
];

const stats = [
  ['60k × 50k', 'blocks, the largest map'],
  ['1.7.10 → 26.3', 'versions it exports to'],
  ['~60', 'terrain nodes'],
  ['0', 'garbage-collector pauses'],
];

export function home({ url, esc, REPO, icon, figure, latest, cmp, guideLink }) {
  const v = latest?.version;
  const badge = (since) => {
    if (!since || !v) return '';
    const short = since.replace(/\.0$/, '');
    if (cmp(v, since) < 0) return `<span class="tag accent">Coming in ${short}</span>`;
    if (cmp(v, since) === 0) return `<span class="tag accent">New in ${short}</span>`;
    return '';
  };
  const link = (f) => (f.graphs ? url('/guide/terrain-graphs/') : f.guide ? guideLink(f.guide) : '');
  const tile = (f) => {
    const h = link(f);
    const inner = `<div class="tile-fig">${figure(f.fig, f.alt)}</div>
      <div class="tile-text"><div class="tile-title"><h3>${f.title}</h3>${badge(f.since)}</div><p>${f.text}</p>${h ? `<span class="more">Learn more ${icon.arrow}</span>` : ''}</div>`;
    return h ? `<a class="tile${f.wide ? ' wide' : ''}" href="${h}">${inner}</a>` : `<div class="tile${f.wide ? ' wide' : ''}">${inner}</div>`;
  };
  const chip = (f) => {
    const h = f.guide ? guideLink(f.guide) : f.title === 'Command line' ? url('/download/#command-line') : '';
    const inner = `<span class="chip-icon">${f.icon}</span><span class="chip-text"><strong>${f.title}</strong>${badge(f.since)}<span>${f.text}</span></span>`;
    return h ? `<a class="chip" href="${h}">${inner}</a>` : `<div class="chip">${inner}</div>`;
  };

  return `<section class="hero">
  <div class="hero-in">
    ${v ? `<a class="announce" href="${url('/whats-new/')}"><span class="dot"></span>Massif ${v} is out<span class="sep"></span>What's new ${icon.arrow}</a>` : ''}
    <h1>The map editor<br class="br-sm"> for Minecraft.</h1>
    <p class="lede">WorldPainter's brushes, a terrain graph like Gaea's and 3D sculpting, for maps tens of thousands of blocks wide.</p>
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
  <header class="section-head"><p class="eyebrow">Features</p><h2>Paint it. Grow it. Sculpt it.</h2><p class="lede">WorldPainter's way of working, with the room to go further.</p></header>
  <div class="tiles">${showcase.map(tile).join('')}</div>
</section>

<section class="section">
  <header class="section-head"><p class="eyebrow">And more</p><h2>Everything else a map needs.</h2></header>
  <div class="chips">${more.map(chip).join('')}</div>
</section>

<section class="section">
  <div class="final">
    <h2>Start your next map.</h2>
    <p>Free, for Windows, macOS and Linux.</p>
    <div class="cta center">
      <a class="pill pill-solid big dl-auto" href="${url('/download/')}">${icon.down}<span class="dl-label">Download</span></a>
      <a class="pill pill-ghost big" href="${REPO}" rel="noopener">View on GitHub</a>
    </div>
  </div>
</section>`;
}
