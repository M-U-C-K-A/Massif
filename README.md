# Massif

Un éditeur de cartes Minecraft en Rust, successeur moderne de WorldPainter : on
sculpte et on peint en 2D sur une **surface multi-niveaux** (sol, surplombs,
îles flottantes, grottes), on prévisualise en 3D, et on exporte un monde
Minecraft Java (`.mca` / NBT). Les mondes sont enregistrés au format
**`.massif`**, un conteneur indexé de tuiles compressées indépendamment.

## Lancer

```bash
cargo run --release -p massif-app
```

Ouvrir directement un monde : `cargo run --release -p massif-app -- chemin/monde.massif`
(un `.world` de WorldPainter est importé de la même façon).

Outil en ligne de commande (générer, inspecter, vérifier, exporter sans ouvrir
l'éditeur) :

```bash
cargo run --release -p massif-cli -- new monde.massif --size 125x125 --landform mountains
cargo run --release -p massif-cli -- info monde.massif
cargo run --release -p massif-cli -- export monde.massif ~/Library/Application\ Support/minecraft/saves/MonMonde
cargo run --release -p massif-cli -- import ancien.world monde.massif
```

La version de Rust est épinglée dans `rust-toolchain.toml` (egui 0.37 demande
Rust ≥ 1.95) ; `rustup` l'installe tout seul, la toolchain par défaut du
système n'est pas modifiée.

## Organisation

| Crate | Rôle |
|---|---|
| `massif-core` | Modèle du monde : coordonnées signées, plans RLE, tuiles, niveaux, composition des colonnes, pinceaux, outils, annulation, génération procédurale. Aucune dépendance graphique. |
| `massif-format` | Lecture / écriture du format `.massif`. |
| `massif-anvil` | Export Minecraft Java : NBT, fichiers région `.mca`, `level.dat`. |
| `massif-legacy` | Import des mondes WorldPainter `.world` (lecteur de sérialisation Java écrit en Rust, sans JVM). |
| `massif-app` | L'éditeur (egui + wgpu) : zones façon Blender, carte 2D, vue 3D, panneaux. |
| `massif-cli` | Outil en ligne de commande. |

## Le modèle : une surface multi-niveaux

Une dimension est une **pile de niveaux**. Chaque niveau est une grille éparse
de tuiles de 128×128 colonnes ; chaque colonne a une hauteur haute, une hauteur
basse (sauf pour le sol), un terrain, un niveau d'eau et un biome.

- **Ground** : la heightmap classique, pleine du bas du monde jusqu'à sa surface.
- **Solid** : ajoute de la matière entre ses deux surfaces → surplombs, arches,
  îles flottantes.
- **Carve** : retire de la matière entre ses deux surfaces → grottes, tunnels,
  fosses.

Les niveaux s'appliquent dans l'ordre ; le plus haut dans la liste gagne. Une
colonne composée devient une courte liste de segments pleins le long de Y,
c'est-à-dire un RLE vertical. Tous les outils de niveaux restent des pinceaux
2D en O(surface), quel que soit le niveau édité.

### La vraie 3D : les volumes sculptés

Là où l'on sculpte en 3D (grottes de n'importe quelle forme, surplombs au-delà
de la verticale, arêtes de falaise arrondies), la dimension garde un **chunk de
voxels** de 16³ blocs : une densité de 0 à 255 par bloc (plein à partir de 128)
qui remplace ce que les niveaux composent dans ce cube. Un chunk naît comme une
copie exacte de la composition (la hauteur fractionnaire de la surface devient
la densité des blocs du dessus), donc sculpter ne change rien à côté du
pinceau. Partout ailleurs, le monde reste en colonnes RLE : une carte de 16 000
blocs ne paie que pour les cubes sculptés.

- **Build (3D)** ajoute de la matière dans une boule, **Dig (3D)** en retire,
  le bouton droit inverse ; dans la vue 3D la boule se pose au point visé, sur
  la carte à la surface.
- **Smooth (3D)** floute la densité en 3D (flou séparable, deux passes, proche
  d'une gaussienne) : une arête convexe s'arrondit, un creux vif se comble, une
  paroi plane reste exactement en place. Ce n'est pas un lissage couche par
  couche à la WorldEdit.
- La matière sculptée à l'air libre prend le terrain de la surface de la
  colonne (herbe, terre puis pierre) ; sous autre chose, c'est de la roche.
- Tout passe par l'annulation (les chunks sont partagés en copie-sur-écriture),
  l'enregistrement, l'export Minecraft, la carte et la vue 3D.

Côté mémoire, chaque plan d'une tuile est `Uniform` (aucune allocation), `Rle`
(valeurs et longueurs en flux séparés, lecture en O(log R) sans décompression)
ou `Dense`, et repasse en RLE après chaque modification. Une tuile est
« chaude » (décodée, éditable) ou « froide » (un bloc zstd) ; seules les
tuiles touchées récemment restent décodées. Exemple mesuré : une carte de
**16 000 × 16 000 blocs** en montagnes se génère en 5,6 s, pèse 182 Mo sur
disque (0,74 octet par colonne), et l'outil en ligne de commande qui la
génère et l'enregistre plafonne à 236 Mo de RAM.

## Le format `.massif`

Petit-boutiste, conçu pour l'accès aléatoire :

```text
en-tête (64 octets)  magic "MASSIF\x1a\n", version, version minimale de lecteur,
                     offset et CRC32C du répertoire, longueur du fichier,
                     CRC32C de l'en-tête
frames de tuiles     une frame zstd par tuile, indépendante
sections             META : réglages, registre et niveaux (JSON compressé)
                     TIDX : index des tuiles d'un niveau, 24 octets par tuile
                            (x, z, offset, longueur, CRC32C)
                     VOXL : chunks sculptés d'une dimension (x, y, z puis
                            4 096 densités par chunk, zstd)
répertoire           32 octets par section (type, id, offset, longueur, CRC32C)
```

- Chaque tuile a son propre checksum : une corruption est localisée, jamais
  silencieuse.
- Une frame est exactement la forme « froide » d'une tuile en mémoire :
  enregistrer copie des octets, ouvrir ne décode rien.
- Les types de sections et de plans inconnus sont ignorés ou conservés tels
  quels : le format peut évoluer sans casser les anciens fichiers.
- L'enregistrement est atomique (fichier temporaire puis renommage).
- Les hauteurs sont stockées au 1/256 de bloc avec un prédicteur de gradient :
  un terrain lisse coûte moins d'un octet par colonne avant zstd.

### Par rapport à la spécification `.rworld`

Repris de ta spec : index (TOC) en fin de fichier pointé par l'en-tête, frames
zstd indépendantes et décodage parallèle, coordonnées `i32` signées avec
`div_euclid` / `rem_euclid`, RLE split-stream avec palette locale et repli
dense, valeurs uniformes quasi gratuites, CRC32C.

Changé :

- **Des chunks voxels 16³ seulement là où l'on sculpte**, au lieu d'un monde
  entier en chunks avec une sentinelle `INHERIT_2D` : la surface multi-niveaux
  reste 2D à l'édition, et la section `VOXL` porte les volumes sculptés en 3D.
- **Métadonnées en JSON compressé** plutôt qu'en postcard : quelques kilo-octets,
  lisibles par n'importe quel outil.
- **Nom** : `.massif`, du nom de l'application. Changer l'extension tient en une
  constante (`massif_format::EXTENSION`).

## Import des mondes WorldPainter

Fichier ▸ Import WorldPainter World… (ou Ouvrir, ou glisser le chemin en
argument) convertit un `.world` : un flux gzip (dont l'en-tête porte la
miniature JPEG) contenant un `ObjectOutputStream` Java de `World2`. Massif le lit
avec son propre lecteur du protocole de sérialisation Java, sans JVM, et
convertit chaque tuile dès qu'elle est lue, en parallèle : un monde de 108 Mo
(2 304 tuiles) s'importe en moins d'une seconde.

Sont repris : les hauteurs (au 1/256 de bloc), les terrains (ceux de WorldPainter
réutilisent les terrains Massif équivalents ou en créent, les terrains
personnalisés sont reconstruits depuis leurs `MixedMaterial` avec leur nom, leurs
blocs et leur couleur), le niveau d'eau, les biomes peints, la surface, le Nether
et l'End, la graine, le point d'apparition et les limites de hauteur. Ne sont pas
encore repris : les calques (arbres, végétation, givre, grottes…), la lave, les
dimensions de plafond, de sol de grotte et de sol flottant ; le rapport
d'import les liste avec le nombre de tuiles concernées.

## Export Minecraft

L'export écrit `region/r.X.Z.mca` et `level.dat` au format de chunk introduit en
1.18 (`DataVersion` 2975, Minecraft 1.18.2) : toutes les versions plus récentes
l'ouvrent et le mettent à jour au chargement. Les régions sont construites en
parallèle (4 096 chunks en 0,15 s). Autour de la carte, Minecraft génère soit
le terrain vanilla de la graine, soit du vide. Les tests relisent les régions
exportées avec un lecteur Anvil indépendant (`fastanvil`) et comparent chaque
bloc au modèle.

## L'éditeur

- **Zones** : chaque zone affiche un éditeur au choix (carte, vue 3D, outils,
  réglages, réglages du pinceau, pinceaux, pinceaux perso, terrain, biomes,
  niveaux, infos). On les divise, les ferme, les
  agrandit (Ctrl+Espace), on les redimensionne en tirant les séparateurs et on
  les déplace en faisant glisser leur titre sur une autre zone.
- **Espaces de travail** : Layout, Sculpting, Painting, 3D, Maps, Minimal ;
  dupliquer avec `+`, supprimer par clic droit. La disposition est mémorisée.
- **Carte** : modes Terrain, Height, Slope, Biome, Relief, courbes de niveau,
  grille, niveaux de détail jusqu'à 1 texel pour 1 024 blocs.
- **Pinceaux, comme dans WorldPainter** : les douze pinceaux symétriques
  (cercle ou carré × Constant, Linear, Sine, Plateau, Spike, Dome, avec les
  formules de WorldPainter), 22 tampons de relief générés (pic, chaîne, volcan,
  mesa, cirque glaciaire…) et les pinceaux perso : des images en niveaux de gris
  (blanc = peint, noir = rien) étirées sur le diamètre. Le dossier `brushes` de
  Massif (Pinceaux ▸ Open the folder) en est la source, chaque sous-dossier est
  un groupe ; ceux de WorldPainter (`WorldPainter [MODERNISED]/brushes`,
  `WorldPainter/brushes`) sont lus directement. Le contour du pinceau sur la
  carte suit la forme de l'image, tournée. Taille jusqu'à 4 096 de rayon
  (molette ×1,1 par cran comme WorldPainter), intensité, rotation par pas de 15°
  dans Brush Settings. Les outils s'appliquent en parallèle, tuile par tuile,
  et le lissage travaille tuile par tuile : la mémoire reste faible même avec
  un pinceau de 8 000 blocs de large.
- **Vue 3D en détail complet** : chaque colonne est maillée au bloc près, quelle
  que soit la distance de vue (qui ne fait que limiter l'étendue, jusqu'à 3 072
  blocs). Un chunk par tuile, faces de même hauteur et couleur fusionnées,
  12 octets par face instanciée ; les chunks sont gardés sur le GPU et
  reconstruits seulement là où le monde change, les plus proches d'abord, dans
  un budget de temps par image. Sur Alpine Valley (montagnes) : 768 blocs →
  700 k faces en 24 ms ; 2 048 blocs → 6,5 M faces (77 Mo) en 0,16 s.
- **Souris et clavier, comme WorldPainter** : bouton gauche = outil, bouton
  droit (ou Alt + gauche) = son inverse (abaisser, effacer, assécher…) ; molette
  = taille du pinceau (Shift : bloc par bloc) ; ⌘, Ctrl ou Alt + molette, ou
  pincement = zoom ; bouton du milieu ou Espace + glisser = déplacer la carte ;
  `−` `+` taille ; `1`…`0` intensité ; `[` `]` rotation du pinceau ; `R` `L`
  rotation de la lumière ; ⌥H ⌥A ⌥S ⌥P ⌥R ⌥F ⌥Y les outils ; ⌘− ⌘+ ⌘0 le zoom ;
  Échap annule le trait en cours ; ⌘Z / ⇧⌘Z, ⌘S, ⌘O, ⌘N, ⌘E.
- **Dans la vue 3D** : toute la carte est visible. Autour de la caméra, sur la
  distance de vue, chaque bloc est dessiné ; au-delà, un relief lisse (un point
  tous les 16 blocs, aux couleurs moyennes des terrains) montre le reste de la
  carte. Glisser à gauche tourne, à droite déplace, molette ou pincement pour
  s'approcher ; W A S D (ou les flèches) volent, Q / E descendent et montent,
  Shift accélère ; un double-clic vise le point sous le curseur. La caméra ne
  colle plus au sol : elle ne suit la hauteur du terrain qu'en douceur quand
  elle suit le centre de la carte. Avec Build, Dig ou Smooth (3D), le bouton
  gauche sculpte au point visé et le droit inverse ; le bouton du milieu (ou
  Alt + glisser) tourne, Shift + milieu déplace.
- **Zones, comme dans le WorldPainter modernisé** : le menu ⋮ de chaque zone
  (diviser verticalement ou horizontalement, joindre avec la zone voisine,
  agrandir, fermer) ; avec Shift enfoncé, une corbeille ferme la zone d'un
  clic. Tirer le coin haut-droit d'une zone vers l'intérieur la divise à
  l'endroit relâché (vers la gauche : côte à côte, vers le bas : l'une sur
  l'autre).

## Tests

```bash
cargo test --workspace
```

Avec `MASSIF_SNAPSHOT_DIR=<dossier>`, les tests de l'éditeur rendent aussi la
fenêtre complète hors écran (egui_kittest + wgpu), la carte, la vue 3D et les
panneaux de pinceaux en PNG dans ce dossier, simulent un coup de pinceau à la
souris et une sculpture dans la vue 3D. `MASSIF_WORLD_SAMPLES=<monde.world>`
ajoute l'import d'un vrai monde WorldPainter et la mesure du maillage 3D.

## Suite prévue

- Calques façon WorldPainter (arbres, végétation, minerais, routes), et leur
  import depuis les `.world`.
- Filtres de pinceau façon WorldPainter (« seulement sur », « sauf sur »,
  au-dessus / en dessous d'une hauteur).
- Sculpture 3D : aplanir selon un plan, pinceaux images en 3D, matériau choisi
  pour la matière ajoutée, nettoyage des chunks redevenus identiques aux
  niveaux.
- Raccourcis personnalisables, plus de réglages d'export.
- Sélection, copier-coller, import de heightmaps.
- Édition des dimensions Nether et End depuis l'interface.
