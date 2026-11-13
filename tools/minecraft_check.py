#!/usr/bin/env python3
"""Check a Massif export in Minecraft itself.

Exports a .massif world for a Minecraft version, starts that version's own
server on it without a window (from the game installed on this computer:
its client jar and libraries, its Java), and has the game confirm, column
by column, what Massif says it wrote there: the top block, the water over
it, the biome. Reports what matches, what does not, and any error the game
logged while reading the chunks.

Running a Minecraft server means accepting Mojang's EULA
(https://aka.ms/MinecraftEULA): the script does not start the server
unless --accept-eula is given.

  tools/minecraft_check.py --world my.massif --version 1.21.11 \\
      --minecraft "/Volumes/T7 Shield/minecraft/Install" --accept-eula
"""

import argparse
import json
import os
import queue
import random
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def find_java(minecraft: Path) -> str:
    runtimes = [Path.home() / "Library/Application Support/minecraft/runtime", minecraft / "runtime"]
    for base in runtimes:
        for name in ("java-runtime-delta", "java-runtime-epsilon", "java-runtime-gamma"):
            for java in base.glob(f"{name}/**/bin/java"):
                if java.is_file():
                    return str(java)
    return "java"


def classpath(minecraft: Path, version: str) -> list[str]:
    vdir = minecraft / "versions" / version
    meta = json.loads((vdir / f"{version}.json").read_text())
    libraries = minecraft / "libraries"
    if not libraries.is_dir():
        libraries = Path.home() / "Library/Application Support/minecraft/libraries"
    paths = []
    for lib in meta.get("libraries", []):
        rules = lib.get("rules")
        if rules:
            allowed = False
            for rule in rules:
                os_name = rule.get("os", {}).get("name")
                if os_name in (None, "osx"):
                    allowed = rule["action"] == "allow"
            if not allowed:
                continue
        artifact = lib.get("downloads", {}).get("artifact")
        if artifact and (libraries / artifact["path"]).is_file():
            paths.append(str(libraries / artifact["path"]))
    paths.append(str(vdir / f"{version}.jar"))
    return paths


def free_port() -> int:
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--world", required=True, type=Path)
    ap.add_argument("--version", default="1.21.11")
    ap.add_argument("--minecraft", type=Path, default=Path.home() / "Library/Application Support/minecraft")
    ap.add_argument("--java")
    ap.add_argument("--probes", type=int, default=40, help="random columns to check")
    ap.add_argument("--at", nargs="*", default=[], help="more columns to check, as x,z")
    ap.add_argument("--accept-eula", action="store_true", help="you accept Mojang's EULA for this local server")
    args = ap.parse_args()

    cli = ROOT / "target/release/massif-cli"
    if not cli.exists():
        subprocess.run(["cargo", "build", "--release", "-p", "massif-cli"], cwd=ROOT, check=True)
    work = Path(tempfile.mkdtemp(prefix="massif-mc-check-"))
    world_dir = work / "massif-check"
    print(f"exporting {args.world} for {args.version} into {world_dir}")
    subprocess.run([str(cli), "export", str(args.world), str(world_dir), "--version", args.version, "--void", "--overwrite"], check=True)

    coords = []
    for xz in args.at:
        x, z = xz.split(",")
        coords += [x, z]
    probes = []
    out = subprocess.run([str(cli), "probe", str(args.world), *coords, "--random", str(args.probes)], check=True, capture_output=True, text=True).stdout
    for line in out.splitlines():
        p = json.loads(line)
        if p.get("top") is not None:
            probes.append(p)
    print(f"{len(probes)} columns to check")

    if not args.accept_eula:
        print("\nNot starting the server: running one means accepting Mojang's EULA (https://aka.ms/MinecraftEULA).")
        print("Run again with --accept-eula if you accept it. The export is in", world_dir)
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
    for p in probes:
        send(f"forceload add {p['x']} {p['z']}")
    time.sleep(4)
    for k, p in enumerate(probes):
        x, y, z = p["x"], p["top"], p["z"]
        send(f"execute if block {x} {y} {z} {p['block']} run say MASSIF {k} top ok")
        send(f"execute unless block {x} {y} {z} {p['block']} run say MASSIF {k} top BAD")
        if p.get("water"):
            wy = p["water"][1] - 1
            send(f"execute if block {x} {wy} {z} minecraft:water run say MASSIF {k} water ok")
            send(f"execute unless block {x} {wy} {z} minecraft:water run say MASSIF {k} water BAD")
        if p.get("biome"):
            send(f"execute if biome {x} {y} {z} {p['biome']} run say MASSIF {k} biome ok")
            send(f"execute unless biome {x} {y} {z} {p['biome']} run say MASSIF {k} biome BAD")
    send("say MASSIF end")
    wait_for("MASSIF end", 120)
    send("stop")
    try:
        server.wait(timeout=120)
    except subprocess.TimeoutExpired:
        server.kill()
    results = [l.split("MASSIF ", 1)[1] for l in log if "] MASSIF " in l and "end" not in l.split("MASSIF ", 1)[1]]
    bad = [r for r in results if r.endswith("BAD")]
    problems = [l for l in log if any(w in l for w in ("ERROR", "Exception", "Couldn't load", "Failed to load", "Failed to read", "Recreating"))]
    print(f"\n{len(results) - len(bad)} checks passed, {len(bad)} failed, over {len(probes)} columns")
    for r in bad:
        k = int(r.split()[0])
        print("  failed:", r, json.dumps(probes[k]))
    if problems:
        print(f"\n{len(problems)} lines of errors in the game's log:")
        for l in problems[:30]:
            print("  ", l)
    shutil.rmtree(work, ignore_errors=True)
    return 0 if not bad and not problems else 1


if __name__ == "__main__":
    sys.exit(main())
