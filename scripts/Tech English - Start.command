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
elif [[ -n "$(find src src-tauri/src src-tauri/resources src-tauri/migrations index.html widget.html -newer "$binary" -type f -print -quit)" ]]; then
  needs_build=true
fi

if $needs_build; then
  echo "Building Tech English (takes 2–4 minutes)…"
  npm install --silent
  npm run tauri build -- --bundles app
fi

open "$app"
echo "Tech English started. Look for the widget at the top-right and the icon in the menu bar."
