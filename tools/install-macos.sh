#!/bin/sh
# Build Massif for this Mac and install it as an application.
#
#   tools/install-macos.sh            → /Applications/Massif.app
#   tools/install-macos.sh ~/Applications
#
# Run it again after pulling changes: it replaces the installed copy.
set -e
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"
dest="${1:-/Applications}"
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)

cargo build --release -p massif-app

app=target/Massif.app
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp target/release/massif "$app/Contents/MacOS/massif"
cp crates/massif-app/assets/icon.icns "$app/Contents/Resources/Massif.icns"
cat > "$app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Massif</string>
  <key>CFBundleDisplayName</key><string>Massif</string>
  <key>CFBundleIdentifier</key><string>io.github.m-u-c-k-a.massif</string>
  <key>CFBundleExecutable</key><string>massif</string>
  <key>CFBundleIconFile</key><string>Massif</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>${version}</string>
  <key>CFBundleVersion</key><string>${version}</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.graphics-design</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
EOF
# The .massif file type, the Quick Look extension that shows a map's
# picture as its file's icon, and the signature (ad hoc, so that Apple
# silicon runs it).
tools/macos/finish-app.sh "$app" "$version" "$(uname -m)"

mkdir -p "$dest"
rm -rf "$dest/Massif.app"
ditto "$app" "$dest/Massif.app"
# Let Finder and Spotlight see the new copy at once, its file types and
# its Quick Look extension too.
touch "$dest/Massif.app"
/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f "$dest/Massif.app" || true
qlmanage -r >/dev/null 2>&1 || true
qlmanage -r cache >/dev/null 2>&1 || true
echo "Installed $dest/Massif.app (Massif $version)"
