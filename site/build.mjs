// Builds Massif's website into site/dist from the repository itself: the
// guide from docs/*.md, "What's new" from .github/RELEASE_NOTES.md, the
// figures and renders from docs/assets, the downloads from GitHub's API.
//
//   node build.mjs            build into dist/
//   node build.mjs --serve    build, then serve dist/ on http://localhost:4321
//
// SITE_BASE sets the path the site lives under (the Pages workflow passes
// "/Massif" for https://m-u-c-k-a.github.io/Massif/, "" for a domain).
// GITHUB_TOKEN, when set, is sent to GitHub's API to raise its rate limit.

import fs from 'node:fs';
import path from 'node:path';
import http from 'node:http';
import { fileURLToPath } from 'node:url';
import { Marked } from 'marked';
import config from './site.config.mjs';
import { home } from './src/home.mjs';
import { faq } from './src/faq.mjs';
import { groups } from './src/guide.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, '..');
const out = path.join(here, 'dist');
const BASE = (process.env.SITE_BASE || '').replace(/\/+$/, '');
const REPO = `https://github.com/${config.repo}`;
const url = (p) => BASE + p;
const read = (p) => fs.readFileSync(path.join(root, p), 'utf8');

const esc = (s) => String(s).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
const slug = (s) => s.toLowerCase().replace(/<[^>]+>/g, '').replace(/[^\p{L}\p{N}\s-]/gu, '').trim().replace(/\s/g, '-');
const plain = (md) => md.replace(/\*\*|`|\*/g, '').replace(/\[([^\]]+)\]\([^)]+\)/g, '$1');

// ── Releases ────────────────────────────────────────────────────────────

async function releases() {
  try {
    const headers = { 'User-Agent': 'massif-site-build', Accept: 'application/vnd.github+json' };
    if (process.env.GITHUB_TOKEN) headers.Authorization = `Bearer ${process.env.GITHUB_TOKEN}`;
    const res = await fetch(`https://api.github.com/repos/${config.repo}/releases?per_page=50`, { headers });
    if (!res.ok) throw new Error(`GitHub answered ${res.status}`);
    const list = (await res.json()).filter((r) => !r.draft && !r.prerelease);
    return list.map((r) => ({
      version: r.tag_name.replace(/^v/, ''),
      date: r.published_at,
      url: r.html_url,
      assets: r.assets.map((a) => ({ name: a.name, size: a.size, url: a.browser_download_url })),
      notes: releaseSection(r.body || '', r.tag_name.replace(/^v/, '')),
    }));
  } catch (e) {
    console.warn(`No releases from GitHub (${e.message}): the download buttons link to the releases page.`);
    return [];
  }
}

// A release's own part of its notes: the release publishes the whole notes
// file, newest first, so its section is the one named after it.
function releaseSection(body, version) {
  const parts = body.split(/^(?=## )/m);
  const own = parts.find((p) => p.startsWith(`## New in ${version}`)) || parts.find((p) => p.startsWith('## In this version'));
  return own ? own.replace(/^## .*\n/, '').trim() : '';
}

// What a release offers, by system, for the list of every version.
function files(r) {
  const mb = (a) => `${Math.round(a.size / 1e6)} MB`;
  const kinds = [
    ['Windows', /windows.*\.zip$/i], ['macOS', /^Massif-.*macos\.dmg$/i], ['macOS .zip', /^Massif-.*macos\.zip$/i],
    ['Linux', /linux.*\.tar\.gz$/i], ['CLI for macOS', /^massif-cli-.*macos\.zip$/i],
  ];
  return kinds.map(([label, re]) => { const a = r.assets.find((x) => re.test(x.name)); return a && { label, url: a.url, size: mb(a), name: a.name }; }).filter(Boolean);
}

const cmp = (a, b) => {
  const x = a.split('.').map(Number), y = b.split('.').map(Number);
  for (let i = 0; i < 3; i++) if ((x[i] || 0) !== (y[i] || 0)) return (x[i] || 0) - (y[i] || 0);
  return 0;
};

// The files a visitor downloads, by system.
function downloads(latest) {
  const pick = (re) => latest?.assets.find((a) => re.test(a.name));
  const all = `${REPO}/releases/latest`;
  const mb = (a) => (a ? `${Math.round(a.size / 1e6)} MB` : '');
  const win = pick(/windows.*\.zip$/i), dmg = pick(/^Massif-.*macos\.dmg$/i), mzip = pick(/^Massif-.*macos\.zip$/i), lin = pick(/linux.*\.tar\.gz$/i), cli = pick(/^massif-cli-.*macos\.zip$/i);
  return {
    version: latest?.version || '',
    page: latest?.url || all,
    windows: { label: 'Windows', file: win?.name || 'Massif-…-windows-x86_64.zip', url: win?.url || all, size: mb(win) },
    macos: { label: 'macOS', file: dmg?.name || 'Massif-…-macos.dmg', url: dmg?.url || all, size: mb(dmg), zip: mzip?.url, cli: cli?.url },
    linux: { label: 'Linux', file: lin?.name || 'Massif-…-linux-x86_64.tar.gz', url: lin?.url || all, size: mb(lin) },
  };
}

// ── Markdown ────────────────────────────────────────────────────────────

// docs/*.md split into pages at their `##` headings: the text before the
// first one is the document's own page.
function splitDoc(file, base, title) {
  const src = read(file)
    .replace(/^# .*\n/, '')
    .replace(/^\[← Back to the README\]\(.*\)\n/m, '');
  const parts = src.split(/^(?=## )/m);
  const intro = parts.shift().replace(/^(- \[.*\]\(#.*\)\n)+/m, '').trim();
  const pages = [{ title, slug: '', path: base, md: intro, file }];
  for (const p of parts) {
    const t = p.match(/^## (.*)/)[1].trim();
    pages.push({ title: t, slug: slug(t), path: `${base}${slug(t)}/`, md: p.replace(/^## .*\n/, '').trim(), file });
  }
  return pages;
}

const guide = splitDoc('docs/guide.md', '/guide/', 'Using Massif');
const graphs = splitDoc('docs/terrain-graphs.md', '/guide/terrain-graphs/', 'Terrain graphs');
// In the order and the groups of src/guide.mjs; a section not listed there
// still gets its page, under "More".
const docPages = [];
{
  const all = [...guide, ...graphs], listed = new Set();
  for (const [group, items] of groups) for (const [d, s, title, summary] of items) {
    const p = all.find((q) => q.file.endsWith(d === 'g' ? '/guide.md' : '/terrain-graphs.md') && q.slug === s);
    if (!p) { console.warn(`src/guide.mjs names ${d}:${s}, which the docs no longer have.`); continue; }
    Object.assign(p, { group, summary }, title ? { title } : {});
    docPages.push(p);
    listed.add(p);
  }
  for (const p of all) if (!listed.has(p)) docPages.push(Object.assign(p, { group: 'More', summary: '' }));
}
const hasGallery = config.gallery.length > 0;

// Where every heading of the docs ended up: "guide.md#rivers" → its page.
const anchors = new Map();
for (const p of docPages) {
  const doc = path.basename(p.file);
  if (p.slug) anchors.set(`${doc}#${p.slug}`, p.path);
  else anchors.set(doc, p.path);
  for (const m of p.md.matchAll(/^#{3,4} (.*)/gm)) {
    const k = `${doc}#${slug(m[1])}`;
    if (!anchors.has(k)) anchors.set(k, `${p.path}#${slug(m[1])}`);
  }
}
const pageFor = (s) => docPages.find((p) => p.file === 'docs/guide.md' && p.slug === s);

function resolveLink(href, file) {
  if (/^(https?:|mailto:)/.test(href)) return href;
  const doc = path.basename(file);
  if (href.startsWith('#')) return url(anchors.get(`${doc}#${href.slice(1)}`) || anchors.get(doc));
  const [p, hash] = href.split('#');
  const target = path.normalize(path.join(path.dirname(file), p));
  if (target === 'README.md') return url('/');
  if (target.startsWith('docs/') && anchors.has(path.basename(target))) {
    const key = hash ? `${path.basename(target)}#${hash}` : path.basename(target);
    return url(anchors.get(key) || anchors.get(path.basename(target)));
  }
  return `${REPO}/${/\.\w+$/.test(target) ? 'blob' : 'tree'}/main/${target}${hash ? '#' + hash : ''}`;
}

function markdown(md, file = 'docs/guide.md', opts = {}) {
  const m = new Marked({ gfm: true });
  m.use({
    renderer: {
      heading({ tokens, depth, text }) {
        const inner = this.parser.parseInline(tokens);
        const level = Math.min(6, depth + (opts.shift || 0));
        const id = slug(text);
        return `<h${level} id="${id}"><a class="anchor" href="#${id}" aria-hidden="true" tabindex="-1">#</a>${inner}</h${level}>\n`;
      },
      link({ href, title, tokens }) {
        const h = resolveLink(href, file);
        const ext = /^https?:/.test(h) && !h.startsWith(config.url || '\0');
        return `<a href="${esc(h)}"${title ? ` title="${esc(title)}"` : ''}${ext ? ' rel="noopener"' : ''}>${this.parser.parseInline(tokens)}</a>`;
      },
    },
  });
  return m.parse(md).replace(/<table>/g, '<div class="table-wrap"><table>').replace(/<\/table>/g, '</table></div>');
}

// ── Release notes ───────────────────────────────────────────────────────

function releaseNotes() {
  const src = read('.github/RELEASE_NOTES.md');
  const sections = src.split(/^(?=## )/m).slice(1);
  const versions = [];
  let earlier = '';
  for (const s of sections) {
    const head = s.match(/^## (.*)/)[1].trim();
    const body = s.replace(/^## .*\n/, '').trim();
    const v = head.match(/^New in (\d+\.\d+\.\d+)/);
    if (v) {
      const entries = body.split(/\n\s*\n/).map((para) => {
        const m = para.match(/^\*\*([^*]+?[.!?])\*\*\s*([\s\S]*)$/);
        return m ? { title: m[1].replace(/\.$/, ''), md: m[2] } : { title: '', md: para };
      });
      versions.push({ version: v[1], entries });
    } else if (head === 'Earlier') earlier = body;
  }
  return { versions, earlier };
}

// ── Page shell ──────────────────────────────────────────────────────────

const icon = {
  github: '<svg viewBox="0 0 16 16" width="16" height="16" aria-hidden="true"><path fill="currentColor" d="M8 0C3.58 0 0 3.58 0 8a8 8 0 0 0 5.47 7.59c.4.07.55-.17.55-.38v-1.33c-2.23.48-2.7-1.07-2.7-1.07-.36-.92-.89-1.17-.89-1.17-.73-.5.05-.49.05-.49.8.06 1.23.83 1.23.83.72 1.22 1.88.87 2.33.66.07-.52.28-.87.5-1.07-1.78-.2-3.65-.89-3.65-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.22 2.2.82a7.6 7.6 0 0 1 4 0c1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48v2.2c0 .21.15.46.55.38A8 8 0 0 0 16 8c0-4.42-3.58-8-8-8Z"/></svg>',
  sun: '<svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" aria-hidden="true"><circle cx="12" cy="12" r="4"/><path d="M12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4"/></svg>',
  moon: '<svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M20 14.5A8 8 0 0 1 9.5 4a8 8 0 1 0 10.5 10.5Z"/></svg>',
  menu: '<svg class="i-menu" viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" aria-hidden="true"><path d="M4 7h16M4 12h16M4 17h16"/></svg>',
  close: '<svg class="i-close" viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" aria-hidden="true"><path d="M6 6l12 12M18 6 6 18"/></svg>',
  list: '<svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M9 6h11M9 12h11M9 18h11M4.5 6h.01M4.5 12h.01M4.5 18h.01"/></svg>',
  search: '<svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" aria-hidden="true"><circle cx="11" cy="11" r="6.5"/><path d="m16 16 4.5 4.5"/></svg>',
  chevron: '<svg viewBox="0 0 16 16" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m4 6 4 4 4-4"/></svg>',
  arrow: '<svg viewBox="0 0 16 16" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M3 8h10M9 4l4 4-4 4"/></svg>',
  heart: '<svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M12 20s-7.5-4.6-7.5-10A4.3 4.3 0 0 1 12 7.3 4.3 4.3 0 0 1 19.5 10c0 5.4-7.5 10-7.5 10Z"/></svg>',
  down: '<svg viewBox="0 0 16 16" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M8 2v9M4 7.5 8 11.5l4-4M3 14h10"/></svg>',
};

const nav = [
  ['Features', '/#features'],
  ...(config.gallery.length ? [['Gallery', '/gallery/']] : []),
  ['Guide', '/guide/'],
  ["What's new", '/whats-new/'],
  ['FAQ', '/faq/'],
];

function shell({ title, description, active = '', body, wide = false }) {
  const full = title ? `${title} · Massif` : 'Massif · The map editor for Minecraft';
  const desc = description || 'A map editor for Minecraft Java worlds in the spirit of WorldPainter, with a node-based terrain generator in the spirit of Gaea, written in Rust.';
  // The menu of narrow screens opens the guide at each group's first page.
  const guideGroups = docPages.filter((p, i) => i === 0 || docPages[i - 1].group !== p.group);
  const navLinks = nav.map(([t, h]) => `<a href="${url(h)}"${active === t ? ' aria-current="page"' : ''}>${t}</a>`).join('');
  return `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>${esc(full)}</title>
<meta name="description" content="${esc(desc)}">
<meta property="og:title" content="${esc(full)}">
<meta property="og:description" content="${esc(desc)}">
<meta property="og:type" content="website">
${config.url ? `<meta property="og:image" content="${config.url}/assets/renders/canyon.jpg">\n` : ''}<meta name="theme-color" content="#0a0a0b" media="(prefers-color-scheme: dark)">
<meta name="theme-color" content="#ffffff" media="(prefers-color-scheme: light)">
<link rel="icon" href="${url('/assets/icon.png')}" type="image/png">
<link rel="preload" href="${url('/assets/fonts/Geist-Variable.woff2')}" as="font" type="font/woff2" crossorigin>
<link rel="stylesheet" href="${url('/assets/style.css')}">
<script>try{var t=localStorage.getItem('massif-theme');if(t)document.documentElement.dataset.theme=t}catch(e){}</script>
<script src="${url('/assets/main.js')}" defer></script>
</head>
<body>
<a class="skip" href="#main">Skip to content</a>
<header class="top">
  <div class="top-in${wide ? ' wide' : ''}">
    <a class="brand" href="${url('/')}"><img src="${url('/assets/icon.png')}" alt="" width="24" height="24">Massif</a>
    <nav class="links" aria-label="Main">${navLinks}</nav>
    <div class="actions">
      <a class="icon-btn" href="${REPO}" aria-label="Massif on GitHub" rel="noopener">${icon.github}</a>
      <button class="icon-btn theme-btn" type="button" aria-label="Switch between light and dark">${icon.sun}${icon.moon}</button>
      <a class="pill pill-solid small hide-sm" href="${url('/download/')}">Download</a>
      <button class="pill pill-ghost small menu-btn" type="button" aria-expanded="false" aria-controls="sheet">${icon.menu}${icon.close}Menu</button>
    </div>
  </div>
  <nav class="sheet" id="sheet" aria-label="Main menu" hidden>
    <div class="sheet-links">${navLinks}</div>
    ${guideGroups.length ? `<div class="sheet-guide"><h2>In the guide</h2><div>${guideGroups.map((g) => `<a href="${url(g.path)}">${esc(g.group)}</a>`).join('')}</div></div>` : ''}
    <a class="pill pill-solid big" href="${url('/download/')}">${icon.down}Download Massif</a>
    <div class="sheet-more"><a href="${REPO}" rel="noopener">${icon.github}GitHub</a>${config.kofi ? `<a href="${esc(config.kofi)}" rel="noopener">${icon.heart}Ko-fi</a>` : ''}${config.discord ? `<a href="${esc(config.discord)}" rel="noopener">Discord</a>` : ''}</div>
  </nav>
</header>
<main id="main">
${body}
</main>
${footer()}
</body>
</html>
`;
}

function footer() {
  const community = [
    ['GitHub', REPO],
    ['Discord', config.discord],
    ['Ko-fi', config.kofi],
  ].map(([t, h]) => (h ? `<a href="${esc(h)}" rel="noopener">${t}</a>` : `<span class="soon" title="Link to come">${t} <em>soon</em></span>`)).join('');
  return `<footer class="foot">
  <div class="foot-in">
    <div class="foot-brand">
      <a class="brand" href="${url('/')}"><img src="${url('/assets/icon.png')}" alt="" width="24" height="24">Massif</a>
      <p>A map editor for Minecraft Java worlds, written in Rust. Free and open source under the GPL-3.0.</p>
    </div>
    <div class="foot-cols">
      <div><h2>Product</h2><a href="${url('/#features')}">Features</a>${hasGallery ? `<a href="${url('/gallery/')}">Gallery</a>` : ''}<a href="${url('/whats-new/')}">What's new</a><a href="${url('/download/')}">Download</a></div>
      <div><h2>Learn</h2><a href="${url('/guide/')}">Guide</a><a href="${url('/guide/terrain-graphs/')}">Terrain graphs</a><a href="${url('/faq/')}">FAQ</a><a href="${REPO}/blob/main/docs/internals.md" rel="noopener">How it works</a></div>
      <div><h2>Community</h2>${community}<a href="${REPO}/issues" rel="noopener">Report a bug</a></div>
    </div>
  </div>
  <div class="foot-base"><span>Massif is not affiliated with Mojang or Microsoft. Minecraft is a trademark of Mojang Synergies AB.</span></div>
</footer>`;
}

// A Hairline figure of docs/assets, in both themes: the solids are filled
// with the colour of GitHub's page, here with the colour of the card.
function figure(name, alt) {
  return `<span class="fig"><img class="on-light" src="${url(`/assets/fig/${name}-light.svg`)}" alt="${esc(alt)}" decoding="async" width="400" height="270"><img class="on-dark" src="${url(`/assets/fig/${name}-dark.svg`)}" alt="${esc(alt)}" decoding="async" width="400" height="270"></span>`;
}

// ── Pages ───────────────────────────────────────────────────────────────

// The guide's pages by group, with the search; in the phone's drawer, the
// current page's headings ("On this page") come first.
function sidebar(current, toc = '') {
  const names = [...new Set(docPages.map((p) => p.group))];
  const group = (g) => {
    const ps = docPages.filter((p) => p.group === g);
    return `<details class="side-group"${ps.includes(current) ? ' open' : ''}><summary>${esc(g)}</summary><div>${ps.map((p) => `<a href="${url(p.path)}"${p === current ? ' aria-current="page"' : ''}>${esc(p.title)}</a>`).join('')}</div></details>`;
  };
  return `<nav class="side" aria-label="Guide">
    <div class="search"><input type="search" placeholder="Search the guide" aria-label="Search the guide" autocomplete="off" spellcheck="false" data-search><kbd>/</kbd><div class="results" hidden></div></div>
    ${toc ? `<details class="side-group side-toc"><summary>On this page</summary><div>${toc}</div></details>` : ''}
    ${names.map(group).join('')}
  </nav>`;
}

function guidePage(p, i) {
  const prev = docPages[i - 1], next = docPages[i + 1];
  const toc = [...p.md.matchAll(/^### (.*)/gm)].map((m) => `<a href="#${slug(m[1])}">${esc(plain(m[1]))}</a>`).join('');
  const body = `<div class="docs">
  <div class="guide-bar">
    <button class="guide-open" type="button" aria-expanded="false" aria-controls="guide-drawer">${icon.list}<span class="guide-where"><span class="crumb">${esc(p.group)}</span><span class="here">${esc(p.title)}</span></span><span class="guide-all">All pages${icon.chevron}</span></button>
    <button class="icon-btn guide-find" type="button" aria-label="Search the guide" aria-controls="guide-drawer">${icon.search}</button>
  </div>
  <div class="drawer" id="guide-drawer" hidden>
    <div class="drawer-panel" role="dialog" aria-modal="true" aria-label="Guide">
      <div class="drawer-head"><strong>Guide</strong><button class="icon-btn drawer-close" type="button" aria-label="Close">${icon.close}</button></div>
      ${sidebar(p, toc)}
    </div>
  </div>
  ${sidebar(p)}
  <article class="prose">
    <p class="eyebrow">${esc(p.group)}</p>
    <h1>${esc(p.title)}</h1>
    ${p.summary ? `<p class="summary">${esc(p.summary)}</p>` : ''}
    ${markdown(p.md, p.file)}
    <nav class="pager" aria-label="Pages">
      ${prev ? `<a class="prev" href="${url(prev.path)}"><span>Previous</span>${esc(prev.title)}</a>` : '<span></span>'}
      ${next ? `<a class="next" href="${url(next.path)}"><span>Next${next.group !== p.group ? ` · ${esc(next.group)}` : ''}</span>${esc(next.title)}</a>` : '<span></span>'}
    </nav>
    <p class="edit"><a href="${REPO}/blob/main/${p.file}" rel="noopener">Edit this page on GitHub</a></p>
  </article>
  ${toc ? `<aside class="toc"><h2>On this page</h2>${toc}</aside>` : '<aside class="toc"></aside>'}
</div>`;
  return shell({ title: p.title, active: 'Guide', body, wide: true, description: p.summary || plain(p.md.split('\n\n')[0]).slice(0, 180) });
}

// What the guide's search box looks through.
function searchIndex() {
  return docPages.map((p) => ({
    t: p.title, g: p.group, u: url(p.path), s: p.summary,
    h: [...p.md.matchAll(/^#{3,4} (.*)/gm)].map((m) => [plain(m[1]), slug(m[1])]),
    x: plain(p.md).replace(/^#+ .*$/gm, '').replace(/\|/g, ' ').replace(/\s+/g, ' ').trim(),
  }));
}

function whatsNew(notes, rels) {
  const dates = new Map(rels.map((r) => [r.version, r]));
  const fmt = (d) => new Date(d).toLocaleDateString('en-GB', { day: 'numeric', month: 'long', year: 'numeric' });
  const items = notes.versions.map((v) => {
    const r = dates.get(v.version);
    const meta = r ? `<a href="${r.url}" rel="noopener">${fmt(r.date)}</a>` : '<span class="tag">In progress</span>';
    return `<section class="release" id="v${v.version}">
  <div class="release-head"><h2><a href="#v${v.version}">${v.version}</a></h2>${meta}</div>
  <div class="release-body">${v.entries.map((e) => `<div class="entry">${e.title ? `<h3>${markdown(e.title).replace(/^<p>|<\/p>\n?$/g, '')}</h3>` : ''}${markdown(e.md, '.github/RELEASE_NOTES.md')}</div>`).join('')}</div>
</section>`;
  }).join('\n');
  const body = `<div class="page narrow">
  <header class="page-head"><p class="eyebrow">Changelog</p><h1>What's new</h1><p class="lede">Every release of Massif, newest first, as its release notes tell it. The one at the top is still being made when it says so.</p></header>
  <div class="releases">${items}
  <section class="release" id="earlier"><div class="release-head"><h2><a href="#earlier">Earlier</a></h2></div><div class="release-body prose">${markdown(notes.earlier, '.github/RELEASE_NOTES.md')}</div></section>
  </div>
</div>`;
  return shell({ title: "What's new", active: "What's new", body, description: 'Every release of Massif, newest first.' });
}

function gallery() {
  const items = config.gallery.map((g, i) => `<figure class="shot"><button type="button" class="shot-open" data-full="${url('/assets/renders/' + g.file)}" data-alt="${esc(g.alt)}" aria-label="Enlarge: ${esc(g.alt)}"><img src="${url('/assets/renders/' + g.file)}" alt="${esc(g.alt)}" loading="${i < 2 ? 'eager' : 'lazy'}"></button><figcaption><strong>${esc(g.title)}</strong><span>${esc(g.alt)}</span></figcaption></figure>`).join('');
  const body = `<div class="page">
  <header class="page-head"><p class="eyebrow">Gallery</p><h1>Made with Massif</h1><p class="lede">Maps made and rendered in Massif.</p></header>
  <div class="gallery">${items}</div>
</div>
<dialog class="lightbox"><form method="dialog"><button class="icon-btn close" aria-label="Close">✕</button></form><img alt=""></dialog>`;
  return shell({ title: 'Gallery', active: 'Gallery', body, description: 'Maps made and rendered in Massif.' });
}

function downloadPage(dl, rels) {
  const notStart = docPages.find((p) => p.slug === 'if-massif-does-not-start');
  const card = (os, steps) => `<section class="os-card" id="${os}" data-os="${os}">
  <div class="os-head"><h2>${dl[os].label}</h2><span class="muted">${{ windows: 'Windows 10 and 11, 64-bit', macos: 'macOS 11 or later, Apple silicon and Intel', linux: 'x86-64' }[os]}</span></div>
  <a class="pill pill-solid" href="${dl[os].url}">${icon.down}Download for ${dl[os].label}</a>
  <p class="file"><code>${esc(dl[os].file)}</code>${dl[os].size ? ` · ${dl[os].size}` : ''}</p>
  <ol class="steps">${steps}</ol>
</section>`;
  const body = `<div class="page mid">
  <header class="page-head"><p class="eyebrow">Download</p><h1>Get Massif${dl.version ? ` ${dl.version}` : ''}</h1><p class="lede">Free, for Windows, macOS and Linux. Massif reads the real Minecraft blocks from the game installed on your computer (official launcher, CurseForge, Prism); nothing from the game is included.</p>
  <p class="head-links"><a href="${dl.page}" rel="noopener">Release page ${icon.arrow}</a><a href="${url('/whats-new/')}">What's new ${icon.arrow}</a></p></header>
  <div class="os-cards">
  ${card('windows', `<li>Unzip the file, then run <code>Massif.exe</code>.</li>
    <li>Massif is not signed by Microsoft. If SmartScreen stops it, choose <strong>More info ▸ Run anyway</strong>.</li>
    <li>On Windows 11 with <strong>Smart App Control</strong> on, Windows blocks apps that are not signed and offers no “Run anyway”. Massif can only run there with Smart App Control off, in <strong>Windows Security ▸ App &amp; browser control ▸ Smart App Control settings</strong>. That choice protects the whole computer: make it knowingly.</li>`)}
  ${card('macos', `<li>Open the <code>.dmg</code> and drag Massif to Applications${dl.macos.zip ? ` (or take the <a href="${dl.macos.zip}">.zip</a>)` : ''}.</li>
    <li>Massif is not notarized by Apple. The first time, right-click Massif ▸ <strong>Open</strong>, then <strong>Open</strong> again; or run <code>xattr -dr com.apple.quarantine /Applications/Massif.app</code>.</li>
    <li>One app for both Apple silicon and Intel Macs.</li>`)}
  ${card('linux', `<li>Extract the archive, then run <code>massif</code>.</li>
    <li>Massif draws with Vulkan or OpenGL: keep the graphics driver up to date.</li>`)}
  </div>
  ${config.kofi ? `<section class="support">
    <span class="support-icon">${icon.heart}</span>
    <div><h2>Support Massif</h2><p>Massif is free and made in the open. If it helps your maps, a coffee on Ko-fi helps it grow.</p></div>
    <a class="pill pill-ghost" href="${esc(config.kofi)}" rel="noopener">Support on Ko-fi ${icon.arrow}</a>
  </section>` : ''}
  <section class="versions" id="all-versions"${rels.length ? '' : ' data-empty'}>
    <div class="versions-head"><h2>Every version</h2><a href="${REPO}/releases" rel="noopener">On GitHub ${icon.arrow}</a></div>
    <div class="versions-list">${rels.map((r, i) => {
      const fmt = new Date(r.date).toLocaleDateString('en-GB', { day: 'numeric', month: 'short', year: 'numeric' });
      return `<details class="version"${i === 0 ? ' open' : ''}>
        <summary><span class="v-name">${esc(r.version)}</span>${i === 0 ? '<span class="tag accent">Latest</span>' : ''}<span class="v-date">${fmt}</span></summary>
        <div class="v-body">
          <div class="v-files">${files(r).map((f) => `<a class="file-pill" href="${f.url}" title="${esc(f.name)}">${icon.down}${f.label}<span>${f.size}</span></a>`).join('')}</div>
          ${r.notes ? `<details class="v-notes"><summary>Release notes</summary><div class="prose">${markdown(r.notes, '.github/RELEASE_NOTES.md')}</div></details>` : ''}
          <a class="v-link" href="${r.url}" rel="noopener">Release ${esc(r.version)} on GitHub ${icon.arrow}</a>
        </div>
      </details>`;
    }).join('')}</div>
  </section>
  <section class="prose block">
    <h2 id="command-line">The command line</h2>
    <p><code>massif-cli</code> creates, inspects, verifies, imports and exports worlds without opening the editor. It comes with the Windows and Linux builds${dl.macos.cli ? `, and <a href="${dl.macos.cli}">separately for macOS</a>` : ', and separately for macOS'}.</p>
    <h2 id="build-from-source">Build from source</h2>
    <p>Massif needs a Rust toolchain; the exact version is pinned in <code>rust-toolchain.toml</code> and <code>rustup</code> installs it.</p>
    <pre><code>git clone ${REPO}.git
cd Massif
cargo run --release -p massif-app</code></pre>
  </section>
  ${notStart ? `<section class="prose block"><h2 id="if-massif-does-not-start">If Massif does not start</h2>${markdown(notStart.md, notStart.file, { shift: 1 })}</section>` : ''}
</div>`;
  return shell({ title: 'Download', body, description: 'Download Massif for Windows, macOS and Linux.' });
}

function faqPage() {
  const body = `<div class="page narrow">
  <header class="page-head"><p class="eyebrow">FAQ</p><h1>Questions</h1><p class="lede">The short answers. The <a href="${url('/guide/')}">guide</a> has the long ones.</p></header>
  <div class="faq">${faq({ url, REPO, config }).map(([q, a], i) => `<details${i === 0 ? ' open' : ''}><summary>${esc(q)}</summary><div class="prose">${markdown(a)}</div></details>`).join('')}</div>
</div>`;
  return shell({ title: 'FAQ', active: 'FAQ', body, description: 'Short answers about Massif.' });
}

function notFound() {
  return shell({ title: 'Not found', body: `<div class="page narrow center"><p class="eyebrow">404</p><h1>No block here</h1><p class="lede">This page does not exist, or not any more.</p><p><a class="pill pill-solid" href="${url('/')}">Back home</a></p></div>` });
}

// ── Write it all ────────────────────────────────────────────────────────

function write(p, html) {
  const file = path.join(out, p.endsWith('/') ? p + 'index.html' : p);
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, html);
}

function copyAssets() {
  const a = path.join(out, 'assets');
  fs.mkdirSync(path.join(a, 'fig'), { recursive: true });
  fs.mkdirSync(path.join(a, 'renders'), { recursive: true });
  fs.mkdirSync(path.join(a, 'fonts'), { recursive: true });
  for (const f of fs.readdirSync(path.join(root, 'docs/assets'))) {
    if (!f.endsWith('.svg')) continue;
    let svg = read(`docs/assets/${f}`);
    // The figures hide what is behind their solids with the page's colour.
    if (!f.startsWith('hero')) svg = svg.replaceAll('#0d1117', '#0f0f11');
    fs.writeFileSync(path.join(a, 'fig', f), svg);
  }
  for (const f of fs.readdirSync(path.join(root, 'docs/assets/renders'))) fs.copyFileSync(path.join(root, 'docs/assets/renders', f), path.join(a, 'renders', f));
  for (const f of fs.readdirSync(path.join(here, 'src/fonts'))) fs.copyFileSync(path.join(here, 'src/fonts', f), path.join(a, 'fonts', f));
  fs.copyFileSync(path.join(root, 'crates/massif-app/assets/icon.png'), path.join(a, 'icon.png'));
  fs.writeFileSync(path.join(a, 'style.css'), fs.readFileSync(path.join(here, 'src/style.css'), 'utf8').replaceAll('url(/assets/', `url(${BASE}/assets/`));
  fs.copyFileSync(path.join(here, 'src/main.js'), path.join(a, 'main.js'));
}

async function build() {
  const t0 = Date.now();
  fs.rmSync(out, { recursive: true, force: true });
  const rels = await releases();
  const latest = rels.slice().sort((a, b) => cmp(b.version, a.version))[0];
  const dl = downloads(latest);
  const notes = releaseNotes();
  copyAssets();
  const ctx = { url, esc, REPO, icon, figure, dl, latest, cmp, config, markdown, guideLink: (s) => (pageFor(s) ? url(pageFor(s).path) : '') };
  write('/', shell({ body: home(ctx) }));
  if (hasGallery) write('/gallery/', gallery());
  fs.writeFileSync(path.join(out, 'assets/search.json'), JSON.stringify(searchIndex()));
  write('/whats-new/', whatsNew(notes, rels));
  write('/download/', downloadPage(dl, rels.slice().sort((a, b) => cmp(b.version, a.version))));
  write('/faq/', faqPage());
  docPages.forEach((p, i) => write(p.path, guidePage(p, i)));
  write('/404.html', notFound());
  // The download button, kept for main.js to update without a rebuild.
  fs.writeFileSync(path.join(out, 'assets/release.json'), JSON.stringify(dl));
  fs.writeFileSync(path.join(out, '.nojekyll'), '');
  console.log(`Built ${docPages.length + 6} pages into site/dist in ${Date.now() - t0} ms${latest ? ` (latest release ${latest.version})` : ''}.`);
}

await build();

if (process.argv.includes('--serve')) {
  const types = { '.html': 'text/html; charset=utf-8', '.css': 'text/css', '.js': 'text/javascript', '.svg': 'image/svg+xml', '.png': 'image/png', '.jpg': 'image/jpeg', '.json': 'application/json', '.woff2': 'font/woff2', '.txt': 'text/plain' };
  const port = Number(process.env.PORT || 4321);
  http.createServer((req, res) => {
    let p = decodeURIComponent(new URL(req.url, 'http://x').pathname);
    if (BASE && p.startsWith(BASE)) p = p.slice(BASE.length) || '/';
    let f = path.join(out, p);
    if (!f.startsWith(out)) return res.writeHead(403).end();
    if (fs.existsSync(f) && fs.statSync(f).isDirectory()) f = path.join(f, 'index.html');
    const ok = fs.existsSync(f);
    if (!ok) f = path.join(out, '404.html');
    res.writeHead(ok ? 200 : 404, { 'Content-Type': types[path.extname(f)] || 'application/octet-stream', 'Cache-Control': 'no-store' });
    fs.createReadStream(f).pipe(res);
  }).listen(port, () => console.log(`Serving http://localhost:${port}${BASE}/`));
}
