# Backdrops

The pictures behind the home screen. Massif takes one at random each time it
opens and dithers it, in its own palette of sixteen colours, darker where the
settings of a new map are written; the dither moves about once a second.

Every `.jpg`, `.jpeg` or `.png` in this folder is built into the app (the build
script lists them): add one, remove one, rebuild. Landscape pictures about
1920 × 1080 work best; what sits low in them stays in view on wide windows.
(The layers of a scene — names ending in `_ciel` or `_sky`, `_relief` or
`_land` — are left out of the app.)

- `01-canyon_complet.png`: a canyon at dusk, its river far below.
- `02-alpes_complet.png`: snowy peaks under the stars.
- `03-iles-volcaniques_complet.png`: a volcano over a calm sea at night.
- `04-avatar_complet.png`: stone pillars standing in the mist.
- `05-dunes_complet.png`: dunes under a night sky.
- `06-fjord-aurore_complet.png`: a fjord under the northern lights.
- `07-rizieres_complet.png`: terraced fields at dusk, a path between them.
- `08-falaises-craie_complet.png`: chalk cliffs over the sea in the moonlight.
- `09-foret-lac_complet.png`: a lake between pines, the moon rising.
- `10-exoplanete_complet.png`: a desert of another world, a ringed planet over it.

The README's banner is drawn from the first five, with their `_relief` layers,
which are kept with its generator in
[`docs/assets/src/scenes`](../../../../docs/assets/src/scenes).
