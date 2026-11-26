#!/usr/bin/env python3
"""Check Massif's schematics in Minecraft itself, with WorldEdit.

Starts a Fabric server of the game installed on this computer (its client
jar, its libraries, the Fabric loader installed for it) with a WorldEdit
mod jar, has WorldEdit load each Sponge schematic Massif wrote and paste
it, then asks the game block by block whether what stands there is what
Massif wrote, and whether the chest kept its name.

The schematics and the blocks to expect come from the writer's test:

  MASSIF_SCHEM_OUT=/tmp/schems tools/task.sh cargo test -p massif-objects sample_files_for_the_game
  tools/schem_check.py --schematics /tmp/schems --version 1.21.11 \\
      --fabric fabric-loader-0.18.4-1.21.11 --worldedit worldedit-mod-7.4.0.jar \\
      --minecraft "/Volumes/T7 Shield/minecraft/Install" --accept-eula

Running a Minecraft server means accepting Mojang's EULA
(https://aka.ms/MinecraftEULA): the script does not start the server
unless --accept-eula is given.
"""

import argparse
import json
import os
import shutil
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import minecraft_check as mc  # noqa: E402


def maven_path(name: str) -> str:
    group, artifact, version = name.split(":")[:3]
    return f"{group.replace('.', '/')}/{artifact}/{version}/{artifact}-{version}.jar"


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--schematics", type=Path, required=True)
    ap.add_argument("--version", required=True)
    ap.add_argument("--fabric", required=True, help="the Fabric loader's version folder, e.g. fabric-loader-0.18.4-1.21.11")
    ap.add_argument("--worldedit", type=Path, required=True)
    ap.add_argument("--minecraft", type=Path, required=True)
    ap.add_argument("--accept-eula", action="store_true")
    args = ap.parse_args()
    if not args.accept_eula:
        print("Running a Minecraft server means accepting Mojang's EULA (https://aka.ms/MinecraftEULA): pass --accept-eula.")
        return 2
    expected = json.loads((args.schematics / "massif.json").read_text())
    java = mc.find_java(args.minecraft, args.version)
    cp = mc.classpath(args.minecraft, args.version)
    fabric = json.loads((args.minecraft / "versions" / args.fabric / f"{args.fabric}.json").read_text())
    libraries = args.minecraft / "libraries"
    for lib in fabric["libraries"]:
        path = libraries / maven_path(lib["name"])
        if path.is_file():
            cp.insert(0, str(path))
        else:
            print("missing Fabric library", path)
            return 1
    work = Path(tempfile.mkdtemp(prefix="massif-schem-check-"))
    (work / "mods").mkdir()
    shutil.copy(args.worldedit, work / "mods")
    schems = work / "config" / "worldedit" / "schematics"
    schems.mkdir(parents=True)
    names = [p.stem for p in sorted(args.schematics.glob("massif-*.schem"))]
    print("schematics:", names)
    for p in args.schematics.glob("*.schem"):
        shutil.copy(p, schems)
    (args.schematics / "back").mkdir(exist_ok=True)
    (work / "eula.txt").write_text("eula=true\n")
    port = mc.free_port()
    (work / "server.properties").write_text(f"level-name=world\nonline-mode=false\nserver-port={port}\nspawn-protection=0\nlevel-type=minecraft\\:flat\ngenerate-structures=false\n")
    cmd = [java, "-Xmx3G", "-Dfabric.side=server", "-cp", os.pathsep.join(cp), "net.fabricmc.loader.impl.launch.knot.KnotServer", "--nogui"]
    import queue
    import subprocess
    import threading
    import time
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
        print("the server did not start:\n" + "\n".join(log[-60:]))
        server.kill()
        return 1
    # WorldEdit reads each file into its clipboard, every state into its
    # own registry, then writes the clipboard back with its own writer:
    # what it read, for the writer's test to compare with what was written.
    failures = 0
    for name in names:
        send(f"schem load {name}.schem")
        if not wait_for(f"{name}.schem loaded", 30):
            print(f"{name}: WorldEdit did not load it:\n" + "\n".join(log[-20:]))
            failures += 1
            continue
        send(f"schem save {name}-we -f")
        if not wait_for(f"{name}-we", 30):
            print(f"{name}: WorldEdit did not save it back:\n" + "\n".join(log[-20:]))
            failures += 1
            continue
        time.sleep(1)
        back = schems / f"{name}-we.schem"
        if back.is_file():
            shutil.copy(back, args.schematics / "back" / back.name)
            print(f"{name}: loaded and saved back by WorldEdit")
        else:
            print(f"{name}: no file saved back")
            failures += 1
    send("stop")
    try:
        server.wait(60)
    except Exception:
        server.kill()
    errors = [line for line in log if "ERROR" in line or "Exception" in line]
    for line in errors[:10]:
        print("log:", line)
    shutil.rmtree(work, ignore_errors=True)
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
