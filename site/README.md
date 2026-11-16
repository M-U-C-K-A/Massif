# Massif's website

A static site built from the repository: the guide from `docs/guide.md` and
`docs/terrain-graphs.md`, "What's new" from `.github/RELEASE_NOTES.md`, the
figures and renders from `docs/assets`, the downloads from the latest GitHub
release. `.github/workflows/pages.yml` publishes it on GitHub Pages on every
push to `main` that touches them, and on every release.

```bash
cd site
npm ci
node build.mjs --serve     # http://localhost:4321
```

- `site.config.mjs`: the Discord and Ko-fi links, the site's address, the
  gallery's renders and their captions.
- `src/home.mjs`: the home page's text and features; `src/faq.mjs`: the FAQ.
- `src/style.css`, `src/main.js`: the look and the little the browser does.
- `build.mjs`: the Markdown to pages, the links between them, the releases.

The fonts are Geist and Geist Mono (SIL Open Font License, `src/fonts/OFL.txt`).
