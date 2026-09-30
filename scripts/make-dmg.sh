#!/usr/bin/env bash
# Makes "Tech English <version>.dmg" from the built app (P6): the app plus a link to Applications.
# Usage: scripts/make-dmg.sh [path to "Tech English.app"]   (default: the release bundle)
set -euo pipefail
cd "$(dirname "$0")/.."
app="${1:-src-tauri/target/release/bundle/macos/Tech English.app}"
[[ -d "$app" ]] || { echo "Build the app first: $app not found" >&2; exit 1; }
version="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$app/Contents/Info.plist")"
out="$(dirname "$app")/Tech English $version.dmg"
stage="$(mktemp -d -t techenglish-dmg)"
trap 'rm -rf "$stage"' EXIT
ditto "$app" "$stage/Tech English.app"
ln -s /Applications "$stage/Applications"
rm -f "$out"
hdiutil create -volname "Tech English $version" -srcfolder "$stage" -ov -format UDZO "$out" >/dev/null
echo "$out ($(du -h "$out" | cut -f1))"
