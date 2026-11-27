#!/usr/bin/env bash
# Intel Open Image Denoise for a build of Massif: the official archive of a
# pinned version, its SHA-256 checked, its libraries copied where the app
# looks for them (crates/massif-render/src/oidn.rs), with its licence and
# third-party notices. Apache-2.0.
#
#   tools/oidn.sh macos   <Massif.app>       # Contents/Frameworks, both architectures
#   tools/oidn.sh windows <folder of Massif.exe>
#   tools/oidn.sh linux   <folder of massif> # lib/
set -euo pipefail

VERSION=2.5.1
BASE="https://github.com/RenderKit/oidn/releases/download/v${VERSION}"
# The archives' SHA-256, as GitHub gives them for the release.
sha() {
  case $1 in
    arm64.macos.tar.gz) echo 98e0aca8e7ab69e9f4f0191582a500fd8b0d9085d68662f18ab6469e934a6efd ;;
    x86_64.macos.tar.gz) echo a4341639005a33ce0d944bd7c771ce34de1465be7eb3189b7c3b5b0064429832 ;;
    x64.windows.zip) echo f11f91bc072a5e3a564515724cb72ab8fcfbc445c84a84c197ff9b16cc01396f ;;
    x86_64.linux.tar.gz) echo 743c3e2aff8c220d5d70fe6cb970fb3d36f2702d2693c61d1d148e404cf37cd6 ;;
  esac
}

platform=$1
dest=$2
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# Fetch one archive into $work and unpack it; prints its folder.
fetch() {
  local kind=$1 file="oidn-${VERSION}.$1"
  curl -fsSL --retry 3 -o "$work/$file" "$BASE/$file"
  local got
  if command -v sha256sum >/dev/null; then got=$(sha256sum "$work/$file" | cut -d' ' -f1); else got=$(shasum -a 256 "$work/$file" | cut -d' ' -f1); fi
  if [ "$got" != "$(sha "$kind")" ]; then
    echo "oidn.sh: $file has SHA-256 $got, not $(sha "$kind")" >&2
    exit 1
  fi
  case $file in
    *.zip) (cd "$work" && unzip -q "$file") ;;
    *) tar -C "$work" -xzf "$work/$file" ;;
  esac
  echo "$work/oidn-${VERSION}.${kind%.tar.gz}" | sed 's/\.zip$//'
}

notices() {
  local from=$1 to=$2
  mkdir -p "$to"
  # The notices sit at the archive's root or in its doc/ folder (Linux, Windows).
  local docs=$from
  [ -e "$docs/LICENSE.txt" ] || docs=$from/doc
  cp "$docs/LICENSE.txt" "$to/OpenImageDenoise-LICENSE.txt"
  for f in "$docs"/third-party-programs*.txt; do
    [ -e "$f" ] && cp "$f" "$to/OpenImageDenoise-$(basename "$f")"
  done
}

case $platform in
  macos)
    arm=$(fetch arm64.macos.tar.gz)
    intel=$(fetch x86_64.macos.tar.gz)
    frameworks="$dest/Contents/Frameworks"
    mkdir -p "$frameworks"
    for lib in "$arm"/lib/libOpenImageDenoise*.dylib; do
      name=$(basename "$lib")
      [ -L "$lib" ] && continue
      if [ -e "$intel/lib/$name" ]; then
        lipo -create -output "$frameworks/$name" "$lib" "$intel/lib/$name"
      else
        # The Metal device runs on Apple silicon only.
        cp "$lib" "$frameworks/$name"
      fi
      codesign --force --sign - "$frameworks/$name"
    done
    # The names the libraries link each other by.
    (cd "$arm/lib" && for l in libOpenImageDenoise*.dylib; do [ -L "$l" ] && cp -a "$l" "$frameworks/"; done) || true
    notices "$arm" "$dest/Contents/Resources"
    ;;
  windows)
    dir=$(fetch x64.windows.zip)
    cp "$dir"/bin/OpenImageDenoise*.dll "$dest/"
    notices "$dir" "$dest"
    ;;
  linux)
    dir=$(fetch x86_64.linux.tar.gz)
    mkdir -p "$dest/lib"
    cp -a "$dir"/lib/libOpenImageDenoise*.so* "$dest/lib/"
    notices "$dir" "$dest"
    ;;
  *)
    echo "usage: tools/oidn.sh macos|windows|linux <destination>" >&2
    exit 2
    ;;
esac
