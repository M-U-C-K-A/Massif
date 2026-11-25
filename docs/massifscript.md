# MassifScript

MassifScript is the terrain graph written as text. A script builds exactly what the Terrain Generator's editor builds — nodes, links, settings, frames, composites, exposed settings and the Map Output's inputs — and the result opens in the editor as an ordinary graph, to move, change and extend by hand. Any graph can be written back as a script.

It is not the Lua scripting of **File ▸ Run Script…** (see [scripting.md](scripting.md)), which acts on the map. MassifScript only describes the node graph that makes the terrain.

```text
project "Alpine Mountains" {
    seed = 4268;

    group "Base Terrain" {
        color = #347a52;
        base = FractalNoise(scale: 2600, height: 700, base: 40, octaves: 9);
        ridges = RidgedNoise(scale: 1400, height: 500, seed: 7);
        mountains = Add(base, ridges, strength: 0.8);
    }

    group "Geology" {
        eroded = StreamErosion(Warp(mountains));
    }

    slope = SlopeMask(eroded);
    output height = eroded;
    output mask.steep = slope;
}
```

## In the editor

The **script** button at the top right of the graph (a page with `<>`) shows the **Script** pane beside the graph (in its place when the window is narrow).

- It starts as the graph written as a script, and follows the graph while you have not typed in it.
- What you type is checked as you type: mistakes are underlined in red, warnings in grey, each listed under the text with its line and column.
- **Apply** makes the graph from the script, as one step to undo. Nodes that were there already keep their places; new ones are laid out left to right, each group in a frame of its own.
- **Graph → script** (the circular arrow) writes the graph as it is now, replacing what is typed.
- **Copy for AI** (the sparkles) copies a guide to paste into any AI chat (ChatGPT, Claude, Mistral…): what MassifScript is, its grammar, every node with its inputs, outputs and settings, examples, and the graph open now as a script. Paste it, then say what you want ("an alpine range around a glacial valley, with rivers"); paste the script it answers into the pane and Apply.
- The file menu of the pane opens and saves `.mscript` files. A script that imports others (`import "lib.mscript";`) reads them beside it.

## The language

### Project and terrain

A script is one `project "Name" { … }`; statements may also stand at the top without it. Inside, `seed = 4268;` is the terrain's seed, `size = 8192;` its side in blocks (the map's when left out) and `sea_level = 62;` its sea level.

Comments are `// to the end of the line` and `/* between these */`.

### Nodes

```text
name = NodeType(input, other_input, setting: value, …);
```

- **Inputs** come first, in the order the catalogue lists them, or by name (`mask: m`).
- **Settings** are always named. Those left out keep their defaults.
- **`name`** becomes the node's id. Other nodes take it as an input: `eroded` (its first output), or `eroded.flow` for another of its outputs.
- A node can be written inside another's arguments: `forest = Blend(a, b, mask: InvertMask(slope));`. It is named after both: `forest_invert_mask`.
- A setting a field can drive (marked * in the catalogue) takes a node: `FractalNoise(height: mountains_mask)`.
- Attributes after the call give the node a name shown in the editor, a note, or bypass it:

  ```text
  soft = Blur(hills) { name: "Soft hills"; note: "for the coast"; bypass: true; };
  ```

Every node of the registry has a CamelCase name, made from the live catalogue: `FractalNoise` (noise.fbm), `StreamErosion`, `SlopeMask`, `Levels`, `MountainRange`, `AlpineRange` (Massif's composite)…

Aliases read like the words people use. Some give settings of their own:

- `Multiply`, `Add`, `Max`… are Blend in that mode.
- `RidgedNoise` is FractalNoise in its ridged style.
- `Remap` is Levels; `DomainWarp` is Warp; `HydraulicErosion` is StreamErosion.

A type can always be called by its id (`noise.fbm(…)`, `"massif:mountain"(…)`), and an older version of a node by `@`: `FractalNoise@1(…)`.

### Values

- **Numbers**, with arithmetic: `+ - * / % ^`, parentheses, comparisons, `&& || !`, `c ? a : b`. The functions are `min max abs sqrt floor ceil round pow clamp lerp sin cos tan exp ln`, and `pi`.
- **Texts**: `"ridged"`, joined with `+`.
- **Booleans**: `true` and `false`.
- **Colours**: `#347a52`.
- **Lists** (`[1, 2, 3]`) and **objects** (`{ x: 0.3, z: 0.6 }`). Points and curves are lists of objects whose `x` and `z` are fractions of the terrain's side, from 0 to 1.

Every calculation is done once, when the script is applied: the graph holds its results.

### Variables, conditions and loops

```text
const ranges = 3;
let height = 400;
for i in 0..ranges {
    ridge[i] = MountainRange(height: height, seed: i);
    height = height + 150;
}
if ranges > 2 { tallest = Max(ridge[1], ridge[2]); } else { tallest = Blur(ridge[0]); }
```

- `const` values cannot change; `let` values change with `name = …;`.
- `for v in 0..4` and `for v in [a, b, c]` repeat their block. In a loop, nodes are named with an index: `ridge[i]` makes `ridge_0`, `ridge_1`… and `ridge[i - 1]` reads one of them.
- Loops and conditions only decide what is built. The graph has no loops of its own.

### Groups

```text
group "Geology" {
    color = #8a6a3a;
    description = "Erosion and rivers";
    …
}
```

A group is a frame round the nodes written inside it. Groups nest. A group can be written again further down with its id (`group geology "Geology" { … }`) to add nodes to it; the ids are made from the titles (`geology`, `base_terrain`).

### Outputs

`output height = eroded;` feeds the Map Output's Height. The map takes:

- `height` and `water`;
- `albedo`, `terrain`, `biome`, `temperature` and `humidity`;
- `cover`, `objects`, `understory`, `pockets`, `caves`, `cave_biomes` and `annotations`;
- `structures` and `roads`;
- `volume.carve` and `volume.add`;
- any mask as `mask.<name>`.

A few common words are understood too: `terrain` given a relief is the height, `vegetation` the objects.

### Components

A component is a composite node written in the script:

```text
component MountainGenerator(relief: height?, seed: seed = 42, height: float = 800, roughness: float = 0.5) {
    label = "Mountain generator";
    summary = "Two noises, the higher of each.";
    expose height { label: "Mountain height"; min: 0; max: 2000; unit: "blocks"; }

    base = FractalNoise(seed: seed, height: height, roughness: roughness);
    ridges = RidgedNoise(seed: seed, scale: 900);
    terrain = Max(base, ridges);
    output terrain;
}

west = MountainGenerator(height: 900, seed: 1);
```

- Parameters typed as a socket are its **inputs**: height, mask, color, vector, terrain, biome, layer, points, splines, footprints, instances, density. A `?` makes one optional.
- Parameters typed `float`, `int`, `bool`, `seed`, `choice`, `string` or `value` are **settings** it exposes. Its nodes take them as values. One setting can feed several nodes' settings, as `seed` above.
- `expose name { … }` says how a setting shows: `label`, `help`, `min`, `max`, `soft` (the slider's range, `[0, 500]`), `unit`, `control`, `category`, `advanced`, `default`. On an input it gives the input's label and help.
- `label`, `category`, `summary` describe the component in the library.
- `output name;` (or `output name = node;`) gives it an output. Use it as any node, and read another output with `west.name`.

It becomes a composite of the document (`project:MountainGenerator`), which the editor opens and edits like one made with ⌘G.

### Imports

`import "lib.mscript";` runs another script first, relative to the one importing it: its components, constants and nodes become this one's. A file is read once, however many times it is imported.

## Errors

Mistakes say where they are and what was meant:

```text
3:12: unknown node “FractalNoize” (did you mean “FractalNoise”?)
5:30: Levels has no setting “gama” (did you mean “gamma”?)
7:15: wrong type: “layers” gives a terrain, but Blur's input “in” takes a height
9:1: expected “;” at the end of the statement, found “b”
```

A setting outside its range is a warning: it is brought back into it, as the editor does.

## The same graph every time

The same script always makes the same graph: node ids are the script's names, and nodes made inside arguments or loops are named from where they are written. Writing a graph as a script and applying that script gives the same graph again: its nodes, settings, links, names, notes, frames and composites. A script written by hand may use loops, constants and imports: the graph it makes holds their results, and writing that graph back gives a script without them.

## On the command line

```text
massif-cli mscript guide [--graph graph.json]   # the guide to paste into an AI chat
massif-cli mscript guide --reference            # the language and the catalogue alone
massif-cli mscript build in.mscript out.json    # a script made into a terrain graph
massif-cli mscript check in.mscript             # what is wrong in a script, line:column
massif-cli mscript decompile graph.json         # a graph written as a script
```

[massifscript-reference.md](massifscript-reference.md) is the reference the guide holds: every node by its script name, written from the registry.

## For tools

`massif_terrain::mscript` is built to be called by other programs. An **MCP server** that would let an AI assistant read, build and change the open graph in Massif directly is planned, not shipped: for now, **Copy for AI** and pasting the answer into the Script pane is the way, and `massif-cli mscript` serves tools on the command line. What such a server will call is here already:

- `compile` gives the graph, the steps it is made of, and the errors with their line and column.
- `decompile` gives a graph's text.
- `guide` and `reference` give the documentation.
- `names` maps CamelCase names to node types and back.

The steps are the edit operations of `massif_terrain::format::Op` that the editor and `massif-cli terrain patch` use, so a script, the editor and a model all change the same graph in the same way.
