#!/usr/bin/env python3
"""Check in Minecraft itself that the water of a Massif export stays still.

Minecraft keeps the water of a world still until a block beside it changes:
then it runs wherever it can. This script exports a .massif world, reads the
water blocks of a few boxes of it, starts that version's own server on the
export (as tools/minecraft_check.py does), and makes every block of each box
change and change back: glass in all of its air, then air again, so that the
game updates each water block the way a player passing by would. It lets the
game run, saves, and reads the boxes again: every water block that is not
as the export wrote it is counted, and the first few are listed (the game
writes default properties back as it saves, and leaves decay: those are
counted apart).

Running a Minecraft server means accepting Mojang's EULA
(https://aka.ms/MinecraftEULA): the script does not start the server
unless --accept-eula is given.

  tools/water_check.py --world my.massif --box -1300,60,-960 --version 1.21.11 \\
      --minecraft "/Volumes/T7 Shield/minecraft/Install" --accept-eula

With --auto N, it also picks N boxes itself: where the map's water has the
most steps, falls and spills (as `massif-cli water` finds them), the places
where the export writes running water and the game would move it if
anything were wrong.
"""

import argparse
import io
import os
import queue
import re
import shutil
import struct
import subprocess
import sys
import tempfile
import threading
import time
import zlib
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from minecraft_check import classpath, find_java, free_port  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent
SIDE = 32


# ---------------------------------------------------------------- NBT ----

def read_nbt(data: bytes):
    f = io.BytesIO(data)

    def payload(tag):
        if tag == 1:
            return struct.unpack(">b", f.read(1))[0]
        if tag == 2:
            return struct.unpack(">h", f.read(2))[0]
        if tag == 3:
            return struct.unpack(">i", f.read(4))[0]
        if tag == 4:
            return struct.unpack(">q", f.read(8))[0]
        if tag == 5:
            return struct.unpack(">f", f.read(4))[0]
        if tag == 6:
            return struct.unpack(">d", f.read(8))[0]
        if tag == 7:
            n = struct.unpack(">i", f.read(4))[0]
            return f.read(n)
        if tag == 8:
            n = struct.unpack(">H", f.read(2))[0]
            return f.read(n).decode("utf-8", "replace")
        if tag == 9:
            t = f.read(1)[0]
            n = struct.unpack(">i", f.read(4))[0]
            return [payload(t) for _ in range(n)]
        if tag == 10:
            out = {}
            while True:
                t = f.read(1)[0]
                if t == 0:
                    return out
                n = struct.unpack(">H", f.read(2))[0]
                name = f.read(n).decode("utf-8", "replace")
                out[name] = payload(t)
        if tag == 11:
            n = struct.unpack(">i", f.read(4))[0]
            return list(struct.unpack(f">{n}i", f.read(4 * n)))
        if tag == 12:
            n = struct.unpack(">i", f.read(4))[0]
            return list(struct.unpack(f">{n}q", f.read(8 * n)))
        raise ValueError(f"tag {tag}")

    t = f.read(1)[0]
    n = struct.unpack(">H", f.read(2))[0]
    f.read(n)
    return payload(t)


def chunk(region_dir: Path, cx: int, cz: int):
    path = region_dir / f"r.{cx >> 5}.{cz >> 5}.mca"
    if not path.exists():
        return None
    with open(path, "rb") as f:
        head = f.read(4096)
        i = 4 * ((cx & 31) + (cz & 31) * 32)
        off = int.from_bytes(head[i:i + 3], "big")
        if off == 0:
            return None
        f.seek(off * 4096)
        length = struct.unpack(">i", f.read(4))[0]
        kind = f.read(1)[0]
        data = f.read(length - 1)
    return read_nbt(zlib.decompress(data) if kind == 2 else data)


def state_name(entry) -> str:
    props = entry.get("Properties") or {}
    name = entry["Name"]
    if props:
        name += "[" + ",".join(f"{k}={v}" for k, v in sorted(props.items())) + "]"
    return name


def blocks_of(region_dir: Path, box):
    """The block states of `box` (x0, y0, z0, x1, y1, z1, exclusive), by
    position."""
    x0, y0, z0, x1, y1, z1 = box
    out = {}
    for cz in range(z0 >> 4, ((z1 - 1) >> 4) + 1):
        for cx in range(x0 >> 4, ((x1 - 1) >> 4) + 1):
            c = chunk(region_dir, cx, cz)
            if c is None:
                continue
            for s in c.get("sections", []):
                sy = s["Y"]
                if (sy + 1) * 16 <= y0 or sy * 16 >= y1:
                    continue
                bs = s.get("block_states")
                if not bs:
                    continue
                palette = [state_name(p) for p in bs["palette"]]
                data = bs.get("data")
                bits = max(4, (len(palette) - 1).bit_length())
                per = 64 // bits
                for ly in range(16):
                    y = sy * 16 + ly
                    if not y0 <= y < y1:
                        continue
                    for lz in range(16):
                        z = cz * 16 + lz
                        if not z0 <= z < z1:
                            continue
                        for lx in range(16):
                            x = cx * 16 + lx
                            if not x0 <= x < x1:
                                continue
                            if data is None:
                                out[(x, y, z)] = palette[0]
                                continue
                            i = (ly * 16 + lz) * 16 + lx
                            v = (data[i // per] & ((1 << 64) - 1)) >> ((i % per) * bits) & ((1 << bits) - 1)
                            out[(x, y, z)] = palette[v]
    return out


def is_water(state: str) -> bool:
    return state is not None and state.startswith("minecraft:water")


def water_key(state):
    """What a block is, as far as water goes: water of a level, or any other
    block by its name (the game writes default properties back on save, and
    leaves decay, but neither is the water's doing)."""
    if state is None:
        return None
    if is_water(state):
        level = "0"
        if "[" in state:
            for prop in state[state.index("[") + 1:-1].split(","):
                k, _, v = prop.partition("=")
                if k == "level":
                    level = v
        return f"water level={level}"
    return state.split("[", 1)[0]


def auto_boxes(cli: Path, world: Path, n: int):
    """The `n` boxes holding the most of the places where the water of
    `world` steps, falls or spills, as `massif-cli water` lists them: a box
    at a time, the points it holds left out of the next."""
    out = subprocess.run([str(cli), "water", str(world)], env={**os.environ, "MASSIF_WATER_WHERE": "all"}, capture_output=True, text=True, check=True).stdout
    points = []
    for line in out.splitlines():
        m = re.match(r"(\w+) at (-?\d+) (-?\d+) (-?\d+)", line)
        if m:
            points.append((m[1], int(m[2]), int(m[3]), int(m[4])))
    kinds = {}
    for k, *_ in points:
        kinds[k] = kinds.get(k, 0) + 1
    print(f"the map's water: {kinds or 'no steps, falls or spills'}")
    boxes = []
    left = points
    while left and len(boxes) < n:
        count = {}
        for _, x, y, z in left:
            for ox in (0, 16):
                for oy in (0, 16):
                    for oz in (0, 16):
                        key = (((x - ox) // SIDE) * SIDE + ox, ((y - 8 - oy) // SIDE) * SIDE + oy + 8, ((z - oz) // SIDE) * SIDE + oz)
                        count[key] = count.get(key, 0) + 1
        (bx, by, bz), _ = max(count.items(), key=lambda kv: (kv[1], kv[0]))
        box = (bx, by, bz, bx + SIDE, by + SIDE, bz + SIDE)
        inside = [p for p in left if bx <= p[1] < bx + SIDE and by <= p[2] < by + SIDE and bz <= p[3] < bz + SIDE]
        what = {}
        for k, *_ in inside:
            what[k] = what.get(k, 0) + 1
        print(f"box {box[:3]} holds {what}")
        boxes.append(box)
        left = [p for p in left if p not in inside]
    return boxes


# --------------------------------------------------------------- main ----

def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--world", required=True, type=Path)
    ap.add_argument("--box", action="append", default=[], help="a box of 32 blocks a side from its lowest corner, as x,y,z")
    ap.add_argument("--auto", type=int, default=0, help="also the N boxes with the most steps, falls and spills of the map's water")
    ap.add_argument("--version", default="1.21.11")
    ap.add_argument("--minecraft", type=Path, default=Path.home() / "Library/Application Support/minecraft")
    ap.add_argument("--java")
    ap.add_argument("--wait", type=float, default=30, help="seconds the game runs after the updates")
    ap.add_argument("--env", action="append", default=[], help="NAME=VALUE for the export")
    ap.add_argument("--show-other", action="store_true", help="list the first few other blocks that changed too")
    ap.add_argument("--accept-eula", action="store_true", help="you accept Mojang's EULA for this local server")
    args = ap.parse_args()

    boxes = []
    for b in args.box:
        x, y, z = (int(v) for v in b.split(","))
        boxes.append((x, y, z, x + SIDE, y + SIDE, z + SIDE))
    cli = ROOT / "target/release/massif-cli"
    if args.auto:
        boxes += auto_boxes(cli, args.world, args.auto)
    if not boxes:
        ap.error("no box to check: give --box or --auto")
    work = Path(tempfile.mkdtemp(prefix="massif-water-check-"))
    world_dir = work / "massif-check"
    env = dict(os.environ)
    for e in args.env:
        k, v = e.split("=", 1)
        env[k] = v
    subprocess.run([str(cli), "export", str(args.world), str(world_dir), "--version", args.version, "--void", "--overwrite"], check=True, env=env)
    region = world_dir / "region"
    before = [blocks_of(region, b) for b in boxes]
    for b, blocks in zip(boxes, before):
        water = sum(1 for s in blocks.values() if is_water(s))
        flowing = sum(1 for s in blocks.values() if is_water(s) and "level=0" not in s and s != "minecraft:water")
        print(f"box {b[:3]}: {water} water blocks as exported, {flowing} of them flowing")

    if not args.accept_eula:
        print("\nNot starting the server: running one means accepting Mojang's EULA (https://aka.ms/MinecraftEULA).")
        return 2
    (work / "eula.txt").write_text("eula=true\n")
    port = free_port()
    (work / "server.properties").write_text(
        f"level-name=massif-check\nonline-mode=false\nserver-port={port}\nspawn-protection=0\nmax-players=1\n"
        "enable-rcon=false\nenable-query=false\nview-distance=4\nsimulation-distance=4\nsync-chunk-writes=true\n"
    )
    java = args.java or find_java(args.minecraft)
    cp = os.pathsep.join(classpath(args.minecraft, args.version))
    cmd = [java, "-Xmx3G", "-cp", cp, "net.minecraft.server.Main", "--nogui", "--universe", str(work), "--world", "massif-check"]
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
    for b in boxes:
        send(f"forceload add {b[0] - 16} {b[2] - 16} {b[3] + 16} {b[5] + 16}")
    time.sleep(6)
    # Every block of each box changes, and changes back.
    for b in boxes:
        x0, y0, z0, x1, y1, z1 = b
        send(f"fill {x0} {y0} {z0} {x1 - 1} {y1 - 1} {z1 - 1} minecraft:glass replace minecraft:air")
    time.sleep(2)
    for b in boxes:
        x0, y0, z0, x1, y1, z1 = b
        send(f"fill {x0} {y0} {z0} {x1 - 1} {y1 - 1} {z1 - 1} minecraft:air replace minecraft:glass")
    time.sleep(args.wait)
    send("save-all flush")
    wait_for("Saved the game", 120)
    send("stop")
    try:
        server.wait(timeout=120)
    except subprocess.TimeoutExpired:
        server.kill()

    total = 0
    for b, was in zip(boxes, before):
        now = blocks_of(region, b)
        changed = [(p, was.get(p), now.get(p)) for p in was if water_key(was.get(p)) != water_key(now.get(p))]
        wet = [c for c in changed if is_water(c[1]) or is_water(c[2])]
        new_sources = sum(1 for _, a, c in wet if water_key(c) == "water level=0" and water_key(a) != "water level=0")
        total += len(wet)
        print(f"box {b[:3]}: {len(wet)} water blocks changed once updated ({new_sources} new sources), {len(changed) - len(wet)} other blocks")
        for p, a, c in wet[:8]:
            print(f"   {p}: {a} -> {c}")
        if args.show_other:
            others = {}
            for _, a, c in changed:
                if not (is_water(a) or is_water(c)):
                    others[(a, c)] = others.get((a, c), 0) + 1
            for (a, c), n in sorted(others.items(), key=lambda kv: -kv[1])[:6]:
                print(f"   other: {n} × {a} -> {c}")
    problems = [l for l in log if any(w in l for w in ("ERROR", "Exception", "Couldn't load", "Failed to load", "Recreating"))]
    for l in problems[:10]:
        print("  game:", l)
    shutil.rmtree(work, ignore_errors=True)
    return 0 if total == 0 and not problems else 1


if __name__ == "__main__":
    sys.exit(main())
