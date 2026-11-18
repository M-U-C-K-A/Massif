# Working on one map together (beta)

Several people can work on the same map and join their work, the way
programmers join their code: one person, the **owner**, keeps the map; the
others, the **contributors**, start from a version of it, work on their own
copy, and send their changes back as a small file. The owner sees on the map
what each contribution changes, accepts or rejects it area by area, and
settles the places where both changed the same land. Nothing changes in the
map until the owner applies what they accepted, and applying it is one step
to undo and a new version, so the map can always go back.

The same review brings back a **zone edited in another tool**: a part of the
map sent to World Machine or Gaea as a heightmap and masks, and brought back
in its place.

Everything happens in the **Versions** panel (under World in an area's list
of editors). This is a beta: the files and the steps may still change
before Massif 1.1.

## Versions

A map's save points are its **versions** (see [Save points](guide.md#save-points)).
Each version knows the version it was made from, who made it and when, so
that copies of a map sent to other people can be told apart and joined
again. The first version gives the map an identifier of its own, the same in
every copy of it. Save points made with Massif 0.6 to 1.0 become versions
when their map is opened: each one stands on the one before.

The Versions panel draws the versions as a **timeline**, the newest on top:
the map's own line on the left, a lane in the accent for each branch, a
curve where a branch leaves a version and where it is merged back, a ring
on a merge. Each version says who made it and when; **Current** marks the
one the map stands on (the last one made or put back), its branch's name
the branch it is on. Click a version for its picture and what to do with
it: **Restore**, **Compare**, **Open as a copy**, **Open read-only**,
**Rename…**, **Delete…**.

The **Versions** button at the right of the top bar names the version the
map stands on (with its branch) and opens the panel; a dot on it says that
a contribution waits in the map's shared folder or is under review.

A map without a version yet shows what working together is about, with
**Start a version** (its first version), **Import a contribution…** and
**Export a zone…**.

## A zone through another tool

1. **File ▸ Export Region…** (or **Export a zone…** in the Versions panel)
   writes a part of the map as images: the heightmap and an image for each
   terrain and layer found there (see [Import mask](guide.md#import-mask)).
   With **Review it when it comes back** on, as it is unless turned off,
   Massif also keeps a version of the map as it is (when it changed since its
   last version) and writes `zone.json` beside the images: which map, which
   version, which blocks.
2. Edit the images in the other tool: erosion in World Machine, a mask of
   rock or trees redrawn, keeping their names and their size.
3. Bring them back with **Import a contribution…** in the Versions panel
   (pick the `zone.json`, or any file of the folder), or with
   **Edit ▸ Import Mask…** and the images. The dialog shows the images as
   Import Mask does, with **Review it as a contribution** on: **Review…**
   paints them onto the version the zone left from, not onto the map, and
   compares the result with that version.
4. The review opens (see below). What the other tool did not change comes
   back through a 16-bit heightmap a hair off at most; those tiles are left
   out, so that only the land really changed is shown.

Without `zone.json`, or with the review turned off, Import Mask paints the
images onto the map at once, as one step to undo.

## Contributions from other copies

A contributor opens the map (a copy of the owner's file, with its versions),
works on it, and makes versions of their own. Then **Export my changes…**
(**Send my changes…** with a shared folder) writes a `.massifpatch`: the
tiles changed since a version they choose (by default the last one someone
else made), each with what it was in that version, the sculpted chunks over
them, the settings changed, the terrains and layers they use, a name and a
message. On a real map of 5,888 × 5,888 blocks made in WorldPainter (a
22.6 MB file), terraforming 1,000 × 1,000 blocks changed 96 tiles: a
contribution of 2.0 MB, written in 7 ms, read and classified in 9 ms.

The owner brings it in with **Import a contribution…**. A contribution to
another map is refused. Terrains, biomes, ground covers and layers the
contributor added are added to the map; one of the same name is the map's.

## The review

The panel shows the contribution (who, the version it starts from — **In
this map** when the map has it —, how many tiles, the message) and its
**areas**: the changed tiles that touch each other, numbered from the north.
Over the 2D map its tiles are drawn in colour:

- green: **new**, where the map had nothing;
- the accent: **changed**, by the contributor only, since the version they
  started from;
- red: **in conflict**, changed both by the contributor and in the map
  since then.

Click an area in the list to see it on the map. **Accept** or **Reject** an
area; **Accept all without conflict** accepts every area without a conflict.
**Undo** beside an area takes the choice back. **Resolve…** opens an area in
conflict: three pictures, the map's tiles (**Mine**), the contribution's
(**Theirs**) and the result, and **Keep**: Mine, Theirs, or **Tile by tile**,
a choice for each tile in conflict. The other tiles of the area, changed by
the contributor alone, are taken too unless turned off.

The settings of the map the contribution changes (sea level, seed, ores,
gradients…) are listed apart, each with its old and new value and a switch:
a setting changes only if switched on.

**Apply** merges what was accepted as one step to undo, then makes a
version: **Merged: message** (with two parents, the map's version and the
contributor's), or **Zone brought back**. If the map changed during the
review, what was accepted is classified again first, and the review shows
the new conflicts. **Cancel** or **Close** leaves the map as it is.

## A shared folder

A folder shared with Google Drive, Dropbox, iCloud or a Git working copy can
hold the map and its contributions, so that nobody sends files by hand:

1. The owner chooses the folder with **Shared folder ▸ Choose…** and
   **Publish**es the map into it: the map, with its versions, as the version
   it stands on (or a new one, **Published**, if it changed since).
2. A contributor opens the map from the folder (File ▸ Open…). Massif knows
   the folder; their work is saved as a map of their own, never over the
   owner's, and the versions they make go on a branch of their name (the
   name they sign their contributions with).
   **Send my changes…** puts their contribution in the folder.
3. The owner sees the contributions waiting under the panel's Share part,
   with **Review** and **Set aside**. A contribution merged or set aside
   waits no more; it stays in the folder.

Each file of the folder has one writer: the owner writes `depot.json` and
the map, contributors only add files to `contributions/`, and a contribution
is never rewritten. A sync service never has two versions of one file to
choose from. With Git, keep the `.massif` file in Git LFS: it is large, and
Git cannot join two of them.

```text
Alpes/                    the shared folder
  depot.json              the map, its owner, the version published, the contributions merged
  Alpes.massif            the map as published
  contributions/
    Jack 3f9a12c0.massifpatch
```

## The files

- **`.massifpatch`**: a contribution, in the container of the `.massif`
  file: the changed tiles (each distinct one once), a manifest (map,
  version, author, message, settings), the registry its numbers refer to and
  the sculpted chunks. A damaged or forged file is refused whole.
- **`zone.json`**: beside the images of a region, the map, version,
  dimension and blocks it was taken from.
- **`depot.json`**: in a shared folder.

Every tile is known by its **fingerprint**, 128 bits of the BLAKE3 hash of
its compressed bytes: a contribution records, for each tile it changes, the
fingerprint of what it was. If the owner's tile still has that fingerprint,
only the contributor changed it; if not, both did. Fingerprints are worked
out when needed, never stored: those of every tile of a map of 16,384 ×
16,384 blocks (16,384 tiles, 105 MB) take 0.014 s on an M4 (0.13 s of
processor).

The figures above come from
`cargo test --release -p massif-format --test patch_perf -- --ignored --nocapture`
(`MASSIF_PATCH_WORLD` names the `.massif` of a real map).

## Limits of the beta

- One owner merges; contributors do not merge each other's work.
- A conflict is settled by whole tiles (128 × 128 blocks): mine, theirs or
  tile by tile. Blending at the seams, and joining the columns each side
  changed within a tile, come later; a seam can be smoothed with a brush.
- Running the terrain graph again changes every tile: a contribution should
  carry changes made by hand.
- Contributors need this version of Massif or a later one.
