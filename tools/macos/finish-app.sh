#!/usr/bin/env bash
#
# Finishes Massif.app once its executable and Info.plist are in place:
# declares the .massif file type (and .world, which Massif opens too), adds
# the Quick Look extension that shows the map of a .massif file as its icon
# in the Finder, as WorldPainter's fork does for .world files, and signs the
# application ad hoc, the extension in its sandbox.
#
# Usage: tools/macos/finish-app.sh PATH/TO/Massif.app VERSION [ARCH...]
#   ARCH: arm64, x86_64 (default: both)
#
# Requirements: the Xcode command line tools (swiftc, lipo, codesign).

set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
APP="$1"
VERSION="$2"
shift 2
ARCHS=("$@")
if [ ${#ARCHS[@]} -eq 0 ]; then
    ARCHS=(arm64 x86_64)
fi

BUILD="$(mktemp -d)"
trap 'rm -rf "$BUILD"' EXIT
# PlistBuddy takes the path inside its command: one without spaces.
cp "$HERE/document-types.plist" "$BUILD/types.plist"
/usr/libexec/PlistBuddy -c "Merge $BUILD/types.plist" "$APP/Contents/Info.plist" >/dev/null

EXTENSION="$APP/Contents/PlugIns/MassifThumbnails.appex"
rm -rf "$EXTENSION"
mkdir -p "$EXTENSION/Contents/MacOS"
sed "s/@VERSION@/$VERSION/g" "$HERE/thumbnails/Info.plist" > "$EXTENSION/Contents/Info.plist"
SDK="$(xcrun --sdk macosx --show-sdk-path)"
PARTS=()
for ARCH in "${ARCHS[@]}"; do
    # An extension has no main function of its own: NSExtensionMain starts it.
    xcrun swiftc -O -module-name MassifThumbnails -parse-as-library -application-extension \
        -target "$ARCH-apple-macos11" -sdk "$SDK" -Xlinker -e -Xlinker _NSExtensionMain \
        "$HERE/thumbnails/ThumbnailProvider.swift" -o "$BUILD/$ARCH"
    PARTS+=("$BUILD/$ARCH")
done
lipo -create "${PARTS[@]}" -output "$EXTENSION/Contents/MacOS/MassifThumbnails"

# Extensions run in the sandbox. Signed ad hoc, like the application.
codesign --force --sign - --entitlements "$HERE/thumbnails/entitlements.plist" "$EXTENSION"
codesign --force --sign - "$APP"
