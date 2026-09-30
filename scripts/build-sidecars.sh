#!/usr/bin/env bash
# Builds static llama-server and whisper-server for the app bundle (P6 dev spec §1).
# Output: src-tauri/binaries/{llama-server,whisper-server}-aarch64-apple-darwin (Tauri externalBin).
# Needs cmake and the Xcode Command Line Tools.
#
# Sources: cloned from GitHub at the pinned tags, or taken from local copies:
#   LLAMA_SRC=/path/to/llama.cpp  WHISPER_SRC=/path/to/whisper.cpp(.tar.gz)  scripts/build-sidecars.sh
set -euo pipefail
cd "$(dirname "$0")/.."

LLAMA_TAG="${LLAMA_TAG:-v0.5.0}"
WHISPER_TAG="${WHISPER_TAG:-v1.9.4}"
TRIPLE=aarch64-apple-darwin
BUILD="$PWD/build/sidecars"
OUT="$PWD/src-tauri/binaries"
JOBS="$(sysctl -n hw.ncpu)"
mkdir -p "$BUILD" "$OUT"

COMMON=(
  -DCMAKE_BUILD_TYPE=Release
  -DCMAKE_OSX_ARCHITECTURES=arm64
  -DCMAKE_OSX_DEPLOYMENT_TARGET=13.3
  -DBUILD_SHARED_LIBS=OFF
  -DGGML_METAL=ON
  -DGGML_METAL_EMBED_LIBRARY=ON
)

# $1 = name, $2 = local source (dir or .tar.gz, may be empty), $3 = repo, $4 = tag → prints the source dir
source_dir() {
  local name="$1" src="$2" repo="$3" tag="$4" dir="$BUILD/$1"
  if [[ -n "$src" && -d "$src" ]]; then
    echo "$src"
  elif [[ -n "$src" && -f "$src" ]]; then
    if [[ ! -d "$dir" ]]; then
      mkdir -p "$dir" && tar xzf "$src" -C "$dir" --strip-components 1
    fi
    echo "$dir"
  else
    [[ -d "$dir" ]] || git clone --depth 1 --branch "$tag" "$repo" "$dir" >&2
    echo "$dir"
  fi
}

echo "== llama-server ($LLAMA_TAG)"
LLAMA_DIR="$(source_dir llama.cpp "${LLAMA_SRC:-}" https://github.com/ggml-org/llama.cpp "$LLAMA_TAG")"
cmake -S "$LLAMA_DIR" -B "$BUILD/llama-build" "${COMMON[@]}" \
  -DLLAMA_BUILD_IS_DEV=OFF -DLLAMA_BUILD_TESTS=OFF -DLLAMA_BUILD_EXAMPLES=OFF -DLLAMA_BUILD_APP=OFF \
  -DLLAMA_BUILD_UI=OFF -DLLAMA_USE_PREBUILT_UI=OFF -DLLAMA_OPENSSL=OFF >"$BUILD/llama-configure.log"
cmake --build "$BUILD/llama-build" --target llama-server -j "$JOBS" >"$BUILD/llama-build.log"
cp "$BUILD/llama-build/bin/llama-server" "$OUT/llama-server-$TRIPLE"

echo "== whisper-server ($WHISPER_TAG)"
WHISPER_DIR="$(source_dir whisper.cpp "${WHISPER_SRC:-}" https://github.com/ggml-org/whisper.cpp "$WHISPER_TAG")"
cmake -S "$WHISPER_DIR" -B "$BUILD/whisper-build" "${COMMON[@]}" \
  -DWHISPER_BUILD_IS_DEV=OFF -DWHISPER_BUILD_TESTS=OFF -DWHISPER_BUILD_EXAMPLES=ON -DWHISPER_BUILD_SERVER=ON \
  -DWHISPER_SDL2=OFF >"$BUILD/whisper-configure.log"
cmake --build "$BUILD/whisper-build" --target whisper-server -j "$JOBS" >"$BUILD/whisper-build.log"
cp "$BUILD/whisper-build/bin/whisper-server" "$OUT/whisper-server-$TRIPLE"

# The license texts go into the bundle (About window).
cp "$LLAMA_DIR/LICENSE" "$OUT/LICENSE-llama.cpp.txt"
cp "$WHISPER_DIR/LICENSE" "$OUT/LICENSE-whisper.cpp.txt"

echo "== check: only system libraries"
for f in "$OUT"/*-"$TRIPLE"; do
  strip -x "$f"
  bad="$(otool -L "$f" | tail -n +2 | grep -v -E '^\s*(/usr/lib/|/System/Library/)' || true)"
  if [[ -n "$bad" ]]; then
    echo "$f links to non-system libraries:" >&2
    echo "$bad" >&2
    exit 1
  fi
  ls -lh "$f" | awk '{print $5, $9}'
done
echo "Sidecars are ready in $OUT"
