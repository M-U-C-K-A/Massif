#!/bin/sh
# The relief bench's real landforms: Copernicus GLO-30 elevation tiles
# (public, 1° × 1°, 30 m), fetched once into a folder outside the
# repository, then read by the bench:
#
#   tools/relief_bench.sh fetch    download the tiles (about 25–40 MB each)
#   tools/relief_bench.sh run      measure them against Massif's nodes and
#                                  draw the side-by-side board
#
# MASSIF_BENCH_DIR says where the tiles go (default
# ~/Library/Caches/Massif/relief-bench); MASSIF_BENCH_OUT where the report
# and the board are written (default the same folder). Copernicus DEM ©
# DLR e.V. 2010–2014 and © Airbus Defence and Space GmbH 2014–2018,
# provided under COPERNICUS by the European Union and ESA.
set -eu
here=$(cd "$(dirname "$0")/.." && pwd)
dir=${MASSIF_BENCH_DIR:-"$HOME/Library/Caches/Massif/relief-bench"}

# The tiles of the cases in crates/massif-terrain/src/bench.rs.
tiles="N46_00_E007_00 N42_00_E000_00 N36_00_W113_00 N38_00_W082_00 N38_00_W029_00"

case "${1:-}" in
fetch)
    mkdir -p "$dir"
    for t in $tiles; do
        name="Copernicus_DSM_COG_10_${t}_DEM"
        if [ -s "$dir/$name.tif" ]; then
            echo "$name: already here"
            continue
        fi
        echo "$name: downloading"
        curl -fsSL --retry 3 -o "$dir/$name.tif.part" "https://copernicus-dem-30m.s3.amazonaws.com/$name/$name.tif"
        mv "$dir/$name.tif.part" "$dir/$name.tif"
    done
    echo "Tiles in $dir."
    ;;
run)
    export MASSIF_BENCH_DIR="$dir"
    export MASSIF_BENCH_OUT="${MASSIF_BENCH_OUT:-$dir}"
    "$here/tools/task.sh" cargo test --release -p massif-terrain --test bench -- --ignored --nocapture
    ;;
*)
    echo "usage: tools/relief_bench.sh fetch | run" >&2
    exit 2
    ;;
esac
