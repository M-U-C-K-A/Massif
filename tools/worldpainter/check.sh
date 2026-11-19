#!/bin/bash
# Checks .world files Massif writes against WorldPainter's own code.
#
#   tools/worldpainter/check.sh prepare <WorldPainter git checkout> [<ref>]
#       Compiles WorldPainter's WPCore at <ref> (default upstream/master, the
#       official WorldPainter) into $MASSIF_WP_JAVA (default: massif-build/
#       wp-java on the build disk, or ~/.massif-wp-java). Needs a JDK 17+ and
#       Maven with WorldPainter's dependencies in ~/.m2 (build WorldPainter
#       once with Maven).
#   tools/worldpainter/check.sh tables
#       Regenerates crates/massif-legacy/src/wp_classes.rs from those classes.
#   tools/worldpainter/check.sh world <file.world> [--expect <columns.tsv>] [--save <out.world>] [--export <folder>]
#       Opens the world with WorldPainter's loading code (WorldCheck.java).
#
# WorldPainter runs with a home of its own, so the user's WorldPainter
# configuration is never read or changed.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
disk="${MASSIF_BUILD_DISK:-/Volumes/T7 Shield}"
if [ -z "${MASSIF_WP_JAVA:-}" ]; then
    if [ -d "$disk" ]; then MASSIF_WP_JAVA="$disk/massif-build/wp-java"; else MASSIF_WP_JAVA="$HOME/.massif-wp-java"; fi
fi
java_home="${JAVA_HOME:-}"
if [ -z "$java_home" ] || ! "$java_home/bin/java" -version 2>&1 | grep -qE 'version "(1[7-9]|2[0-9])'; then
    for candidate in /opt/homebrew/opt/openjdk@21 /opt/homebrew/opt/openjdk /usr/local/opt/openjdk@21; do
        if [ -x "$candidate/bin/java" ]; then java_home="$candidate"; break; fi
    done
fi
java="$java_home/bin/java"
javac="$java_home/bin/javac"
classes="$MASSIF_WP_JAVA/classes"
deps() { cat "$MASSIF_WP_JAVA/wpcore.cp"; }

case "${1:-}" in
prepare)
    checkout="$2"
    ref="${3:-upstream/master}"
    root="$(git -C "$checkout" rev-parse --show-toplevel)"
    prefix="$(git -C "$checkout" rev-parse --show-prefix)"
    mkdir -p "$MASSIF_WP_JAVA"
    rm -rf "$MASSIF_WP_JAVA/src" "$classes"
    mkdir -p "$MASSIF_WP_JAVA/src" "$classes"
    git -C "$root" archive --format=tar "$ref" "${prefix}WPCore" "${prefix}pom.xml" "${prefix}lombok.config" | tar -x -C "$MASSIF_WP_JAVA/src"
    src="$MASSIF_WP_JAVA/src/${prefix}WPCore"
    (cd "$checkout/WPCore" && JAVA_HOME="$java_home" mvn -o -q dependency:build-classpath -Dmdep.outputFile="$MASSIF_WP_JAVA/wpcore.cp")
    # Quoted: the paths may hold spaces.
    find "$src/src/main/java" -name '*.java' ! -name '._*' | sed 's/.*/"&"/' > "$MASSIF_WP_JAVA/sources.txt"
    lombok="$(deps | tr ':' '\n' | grep lombok)"
    "$javac" -nowarn -encoding UTF-8 -J-Xmx2g -cp "$(deps)" -processorpath "$lombok" -d "$classes" @"$MASSIF_WP_JAVA/sources.txt" 2> "$MASSIF_WP_JAVA/javac.log" || { cat "$MASSIF_WP_JAVA/javac.log"; exit 1; }
    cp -R "$src/src/main/resources/." "$classes/"
    find "$classes" -name '._*' -delete
    # Maven fills these in when it builds WorldPainter.
    version="$(git -C "$root" show "$ref:${prefix}pom.xml" | sed -n 's:.*<version>\(.*\)</version>.*:\1:p' | head -1)"
    printf 'org.pepsoft.worldpainter.version=%s\norg.pepsoft.worldpainter.build=%s\n' "${version%-SNAPSHOT}" "$(date +%Y%m%d%H%M%S)" > "$classes/org.pepsoft.worldpainter.properties"
    sed -i.bak "s/\${project.version}/${version%-SNAPSHOT}/" "$classes/org.pepsoft.worldpainter.plugins" && rm -f "$classes/org.pepsoft.worldpainter.plugins.bak"
    echo "WorldPainter ${version%-SNAPSHOT} ($ref) compiled into $classes"
    ;;
tables)
    mkdir -p "$MASSIF_WP_JAVA/tools" "$MASSIF_WP_JAVA/home"
    "$javac" -proc:none -nowarn -d "$MASSIF_WP_JAVA/tools" -cp "$classes:$(deps)" "$here/SerialSpec.java"
    "$java" -Duser.home="$MASSIF_WP_JAVA/home" -cp "$MASSIF_WP_JAVA/tools:$classes:$(deps)" SerialSpec "$here/classes.txt" > "$repo/crates/massif-legacy/src/wp_classes.rs"
    echo "wrote crates/massif-legacy/src/wp_classes.rs"
    ;;
world)
    shift
    mkdir -p "$MASSIF_WP_JAVA/tools" "$MASSIF_WP_JAVA/home"
    "$javac" -proc:none -nowarn -d "$MASSIF_WP_JAVA/tools" -cp "$classes:$(deps)" "$here/WorldCheck.java"
    exec "$java" -Xmx8g ${JAVA_OPTS:-} -Djava.awt.headless=true -Duser.home="$MASSIF_WP_JAVA/home" -cp "$MASSIF_WP_JAVA/tools:$classes:$(deps)" WorldCheck "$@"
    ;;
*)
    sed -n '2,17p' "$0"
    exit 1
    ;;
esac
