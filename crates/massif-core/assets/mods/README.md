# Mod profiles

What Massif knows of a block mod beyond what its jar says, as data: one
folder per mod, named after the namespace of its blocks (`conquest` for
`conquest:old_red_bricks`). Nothing here is read unless that mod's blocks
are read, from its jar or its modpack's instance.

- `profile.json`: `name` (the mod's name, as players know it), `namespace`,
  and `minecraft`, the version of the game the mod is made for, when the
  map's summary and the export should default to it.
- `palette.json` (optional): a block palette (see
  `massif_core::materials::Palette`) of the mod's blocks, offered in
  Info ▸ World ▸ Blocks and on the home screen once the mod is read.

Shipped: `conquest` (Conquest Reforged), `biomesoplenty` (Biomes O' Plenty),
`regions_unexplored` (Regions Unexplored), `biomeswevegone` (Oh The Biomes
We've Gone). Check a profile against its mod's jar:
`MASSIF_MOD_JARS=<jar> cargo test -p massif-assets mod_profiles_match_their_mods -- --nocapture`
(it prints the colours to keep). `docs/guide.md` ▸ *Mod profiles* has the
format with examples.

The folders are found when Massif is built (`build.rs`): adding a mod is
adding a folder, without code.
