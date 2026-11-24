#!/usr/bin/env python3
"""Check the caves of a Massif export in Minecraft itself.

Paints a cave layer as the game's over a .massif world, with lush caves,
dripstone caves and the deep dark in thirds of it (`massif-cli caves
--biomes`), exports it for a Minecraft version, starts that version's own
server on it without a window (as tools/minecraft_check.py does), and has
the game confirm the places `massif-cli caves` lists: the water and lava of
the caves where they were written, the sculk sensors, shriekers and
catalysts with their block entities, a sensor turning active when a
snowball lands beside it, stalactites ending in a tip, cave vines. The water of the
caves staying still once blocks beside it change is tools/water_check.py's
part: run it with the boxes this script prints.

Running a Minecraft server means accepting Mojang's EULA
(https://aka.ms/MinecraftEULA): the script does not start the server
unless --accept-eula is given.

  tools/cave_check.py --world my.massif --version 26.3 \\
      --minecraft "/Volumes/T7 Shield/minecraft/Install" --accept-eula
"""

import argparse
import json
import os
import queue
import shutil
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import minecraft_check as mc  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent

EXPECT = {
    "water": "minecraft:water",
    "lava": "minecraft:lava",
    "sculk_sensor": "minecraft:sculk_sensor",
    "sculk_shrieker": "minecraft:sculk_shrieker",
    "sculk_catalyst": "minecraft:sculk_catalyst",
    "stalactite_tip": "minecraft:pointed_dripstone[thickness=tip,vertical_direction=down]",
    "cave_vines": "minecraft:cave_vines",
}


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--world", required=True, type=Path)
    ap.add_argument("--version", default="26.3")
    ap.add_argument("--minecraft", type=Path, default=Path.home() / "Library/Application Support/minecraft")
    ap.add_argument("--java")
    ap.add_argument("--accept-eula", action="store_true", help="you accept Mojang's EULA for this local server")
    ap.add_argument("--keep", action="store_true", help="check the world's own caves (a terrain graph's) instead of painting the game's")
    ap.add_argument("--probe", action="append", default=[], metavar="X,Y,Z,BLOCK", help="also check that block X Y Z is BLOCK (minecraft:air in a carved tunnel, say)")
    args = ap.parse_args()

    cli = ROOT / "target/release/massif-cli"
    if not cli.exists():
        subprocess.run(["cargo", "build", "--release", "-p", "massif-cli"], cwd=ROOT, check=True)
    work = Path(tempfile.mkdtemp(prefix="massif-cave-check-"))
    caves = work / "caves.massif"
    out = subprocess.run([str(cli), "caves", str(args.world), str(caves)] + (["--keep"] if args.keep else ["--biomes"]), check=True, capture_output=True, text=True).stdout
    places = [json.loads(line) for line in out.splitlines() if line.startswith("{")]
    for probe in args.probe:
        x, y, z, block = probe.split(",", 3)
        EXPECT[block] = block
        places.append({"kind": block, "x": int(x), "y": int(y), "z": int(z)})
    print(f"{len(places)} places to check: " + ", ".join(sorted({p['kind'] for p in places})))
    world_dir = work / "massif-check"
    subprocess.run([str(cli), "export", str(caves), str(world_dir), "--version", args.version, "--void", "--overwrite"], check=True)
    boxes = sorted({(p["x"] - 16, p["y"] - 16, p["z"] - 16) for p in places if p["kind"] in ("water", "lava")})[:6]
    print("boxes for tools/water_check.py: " + " ".join(f"--box {x},{y},{z}" for x, y, z in boxes))

    if not args.accept_eula:
        print("\nNot starting the server: running one means accepting Mojang's EULA (https://aka.ms/MinecraftEULA).")
        print("Run again with --accept-eula if you accept it. The export is in", world_dir)
        return 2
    (work / "eula.txt").write_text("eula=true\n")
    port = mc.free_port()
    (work / "server.properties").write_text(
        f"level-name=massif-check\nonline-mode=false\nserver-port={port}\nspawn-protection=0\nmax-players=1\n"
        "enable-rcon=false\nenable-query=false\nview-distance=4\nsimulation-distance=4\nsync-chunk-writes=true\n"
    )
    java = args.java or mc.find_java(args.minecraft)
    cp = os.pathsep.join(mc.classpath(args.minecraft, args.version))
    cmd = [java, "-Xmx3G", "-cp", cp, "net.minecraft.server.Main", "--nogui", "--universe", str(work), "--world", "massif-check"]
    print("starting", args.version, "with", java)
    server = subprocess.Popen(cmd, cwd=work, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, bufsize=1)
    lines: queue.Queue = queue.Queue()
    log = []

    def read():
        for line in server.stdout:
            log.append(line.rstrip())
            lines.put(line.rstrip())

    threading.Thread(target=read, daemon=True).start()

    def wait_for(text, timeout):
        end = time.time() + timeout
        while time.time() < end:
            try:
                line = lines.get(timeout=0.5)
            except queue.Empty:
                if server.poll() is not None:
                    return None
                continue
            if text in line:
                return line
        return None

    def send(command):
        server.stdin.write(command + "\n")
        server.stdin.flush()

    if not wait_for("Done (", 300):
        print("the server did not start:\n" + "\n".join(log[-40:]))
        server.kill()
        return 1
    for p in places:
        send(f"forceload add {p['x']} {p['z']}")
    time.sleep(6)
    for k, p in enumerate(places):
        x, y, z, want = p["x"], p["y"], p["z"], EXPECT[p["kind"]]
        send(f"execute if block {x} {y} {z} {want} run say MASSIF {k} block ok")
        send(f"execute unless block {x} {y} {z} {want} run say MASSIF {k} block BAD")
        if p["kind"].startswith("sculk"):
            # Its block entity: the game prints its data, or says there is none.
            send(f"data get block {x} {y} {z}")
    send("say MASSIF blocks done")
    wait_for("MASSIF blocks done", 120)
    # A sensor hears a snowball land beside it (a vibration, as a block set
    # by a command is not), and turns active.
    sensors = [(k, p) for k, p in enumerate(places) if p["kind"] == "sculk_sensor"][:4]
    for k, p in sensors:
        x, y, z = p["x"], p["y"], p["z"]
        send(f"summon minecraft:snowball {x + 1.5} {y + 1.2} {z + 0.5} {{Motion:[0.0d,-0.4d,0.0d]}}")
        time.sleep(1.0)
        send(f"execute if block {x} {y} {z} minecraft:sculk_sensor[sculk_sensor_phase=active] run say MASSIF {k} hears ok")
        send(f"execute if block {x} {y} {z} minecraft:sculk_sensor[sculk_sensor_phase=cooldown] run say MASSIF {k} hears ok")
        send(f"execute if block {x} {y} {z} minecraft:sculk_sensor[sculk_sensor_phase=inactive] run say MASSIF {k} hears BAD")
    # Some time for anything that would run to run, then the fluids again.
    time.sleep(8)
    for k, p in enumerate(places):
        if p["kind"] in ("water", "lava"):
            x, y, z, want = p["x"], p["y"], p["z"], EXPECT[p["kind"]] + "[level=0]"
            send(f"execute if block {x} {y} {z} {want} run say MASSIF {k} still ok")
            send(f"execute unless block {x} {y} {z} {want} run say MASSIF {k} still BAD")
    send("say MASSIF end")
    wait_for("MASSIF end", 120)
    send("stop")
    try:
        server.wait(timeout=120)
    except subprocess.TimeoutExpired:
        server.kill()
    results = [l.split("MASSIF ", 1)[1] for l in log if "] MASSIF " in l and "done" not in l and "end" not in l.split("MASSIF ", 1)[1]]
    bad = [r for r in results if r.endswith("BAD")]
    entities = sum(1 for l in log if "has the following block data" in l)
    no_entity = sum(1 for l in log if "is not a block entity" in l)
    problems = [l for l in log if any(w in l for w in ("ERROR", "Exception", "Couldn't load", "Failed to load", "Failed to read", "Recreating")) and "DragonFight" not in l]
    sculk = sum(1 for p in places if p["kind"].startswith("sculk"))
    print(f"\n{len(results) - len(bad)} checks passed, {len(bad)} failed, over {len(places)} places")
    print(f"block entities: {entities} of {sculk} sculk blocks have one, {no_entity} have none")
    for r in bad:
        k = int(r.split()[0])
        print("  failed:", r, json.dumps(places[k]))
    if problems:
        print(f"\n{len(problems)} lines of errors in the game's log:")
        for l in problems[:30]:
            print("  ", l)
    shutil.rmtree(work, ignore_errors=True)
    return 0 if not bad and not problems and no_entity == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
