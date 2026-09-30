#!/bin/zsh -l
# Start Tech English (release build). Builds it first if needed or if the code changed.
set -eu

script_path="${0:A}"
project_directory="${script_path:h:h}"
cd "$project_directory"

app="src-tauri/target/release/bundle/macos/Tech English.app"
binary="$app/Contents/MacOS/tech-english"

if pgrep -x tech-english >/dev/null; then
  echo "Tech English is already running."
  exit 0
fi

needs_build=false
if [[ ! -x "$binary" ]]; then
  needs_build=true
elif [[ -n "$(find src src-tauri/src src-tauri/resources src-tauri/migrations src-tauri/binaries \
  src-tauri/Cargo.toml src-tauri/tauri.conf.json src-tauri/tauri.bundle.conf.json src-tauri/Info.plist \
  index.html widget.html -newer "$binary" -type f -print -quit 2>/dev/null)" ]]; then
  needs_build=true
fi

if $needs_build; then
  # The AI engines are built once and then bundled inside the app (P6).
  triple=aarch64-apple-darwin
  if [[ ! -x "src-tauri/binaries/llama-server-$triple" || ! -x "src-tauri/binaries/whisper-server-$triple" ]]; then
    echo "Building the AI engines (once, about 5 minutes; needs cmake)…"
    scripts/build-sidecars.sh
  fi
  echo "Building Tech English (takes 2–4 minutes)…"
  npm install --silent
  npm run tauri build -- --bundles app --config src-tauri/tauri.bundle.conf.json
fi

open "$app"
echo "Tech English started. Look for the widget at the top-right and the icon in the menu bar."
