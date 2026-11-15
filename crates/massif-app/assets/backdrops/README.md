# Backdrops

The pictures behind the home screen. Massif takes one at random each time it
opens and dithers it, in its own palette of sixteen colours, darker where the
settings of a new map are written; the dither moves about once a second.

Every `.jpg`, `.jpeg` or `.png` in this folder is built into the app (the build
script lists them): add one, remove one, rebuild. Landscape pictures about
1920 × 1080 work best; what sits low in them stays in view on wide windows.
The layers of a scene — names ending in `_ciel` or `_sky` (its sky),
`_relief` or `_land` (its land cut out) — are not shown alone.

- `01-canyon_complet.png`: a canyon at dusk, its river far below.
- `02-alpes_complet.png`: snowy peaks under the stars.
- `03-iles-volcaniques_complet.png`: a volcano over a calm sea at night.
- `04-avatar_complet.png`: stone pillars standing in the mist.
- `05-dunes_complet.png`: dunes under a night sky.

Each scene also has its layers (`_ciel`, `_relief`), kept beside it for
whoever works on the pictures; they are not built in.
