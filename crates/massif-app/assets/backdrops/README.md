# Backdrops

The pictures behind the home screen. Massif takes one at random each time it
opens and dithers it, in its own palette of sixteen colours, darker where the
settings of a new map are written; the dither moves about once a second.

Every `.jpg`, `.jpeg` or `.png` in this folder is built into the app (the build
script lists them): add one, remove one, rebuild. Landscape pictures about
1920 × 1080 work best; what sits low in them stays in view on wide windows.

- `massif-dusk.jpg`: a massif at dusk, drawn by Massif itself.
- `floating-mountains.jpg`, `archipelago-sunset.jpg`, `karst-twilight.jpg`,
  `canyon-sunset.jpg`: templates of the terrain graph, rendered by its viewport.

Render them again with:

```sh
MASSIF_BACKDROP_DIR="$PWD/crates/massif-app/assets/backdrops" cargo test -p massif-app -- render_the_home_backdrops draw_the_massif_at_dusk
```
