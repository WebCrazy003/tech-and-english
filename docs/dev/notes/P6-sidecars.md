# P6 T1 — bundled engines (2026-09-30)

`scripts/build-sidecars.sh` builds both engines as single static files for Apple Silicon and puts them in `src-tauri/binaries/` (git-ignored). Tauri copies them to `Tech English.app/Contents/MacOS/` (`externalBin` in `src-tauri/tauri.bundle.conf.json`).

| Engine | Tag | Built file | Size | Links to |
|---|---|---|---|---|
| llama.cpp `llama-server` | **v0.5.0** (build 11146, commit 7fe450e) | `llama-server-aarch64-apple-darwin` | 12 MB | macOS system libraries and frameworks only (Accelerate, Metal, MetalKit, Foundation, libc++) |
| whisper.cpp `whisper-server` | **v1.9.4** | `whisper-server-aarch64-apple-darwin` | 4.4 MB | the same |

## Choices

- **Tags.** P2 used the official nightly build b11256 (0.5.0-dev, 110 commits after v0.5.0). The bundle pins the stable tag **v0.5.0** instead; the P2/P5 checks were repeated on the built files (below).
- **CMake options** (checked against the sources of these tags): `BUILD_SHARED_LIBS=OFF`, `GGML_METAL=ON`, `GGML_METAL_EMBED_LIBRARY=ON`, `CMAKE_OSX_DEPLOYMENT_TARGET=13.3`; llama: `LLAMA_BUILD_TESTS/EXAMPLES/APP/UI=OFF`, `LLAMA_USE_PREBUILT_UI=OFF` (no download during the build), `LLAMA_OPENSSL=OFF` (the server only listens on 127.0.0.1; no HTTPS and no OpenSSL dependency); whisper: `WHISPER_BUILD_SERVER=ON`, `WHISPER_SDL2=OFF`. The spec's `LLAMA_CURL=OFF` no longer exists.
- **macOS 13.3** is the minimum: ggml's BLAS backend uses `cblas_sgemm` from the new Accelerate interface (13.3+). `minimumSystemVersion` was raised from 13.0.
- **Sources.** By default the script clones the tags from GitHub. `LLAMA_SRC` / `WHISPER_SRC` point it at local copies (a folder or a `.tar.gz`). This Mac's build used the sources Homebrew had already downloaded, so nothing new was fetched.
- **Not in `tauri.conf.json`:** `externalBin` would make every `cargo build` / `cargo test` fail on a checkout without the built engines. The bundle-only settings live in `tauri.bundle.conf.json`, passed with `--config` for release builds (the Start script does this).
- **First start:** a new Metal binary compiles its shaders once (whisper: about 30 s). The first-run AI step runs `whisper-server --help` and `llama-server --version`, which warms this up before the first talk.

## Checks on the built files

| Check | Result |
|---|---|
| `otool -L`: only `/usr/lib` and `/System/Library` | ✅ both |
| `llama-server --version` | `0.5.0 (build 11146, commit 7fe450e19)` |
| Golden correction suite (`TE_LLAMA_SERVER=…`) | **28/30 = 93 %**, 0 disfluencies corrected, first sentence p50 1.25 s |
| Live voice session (`TE_LLAMA_SERVER`, `TE_WHISPER_SERVER`), 18 spoken turns | first sentence **p50 2.0 s, p90 3.0 s**; STT p50 0.16 s; all phases worked; review by the LLM |
| JSON schema key order, `anyOf` null/object, prompt cache | ✅ (covered by the two tests above: `reply` streams first, corrections parse, turns 2+ answer in ~1.2 s) |
