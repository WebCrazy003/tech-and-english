# P6 — Packaging & polish (dev spec)

- **Goal:** a self-contained `Tech English.app` / `.dmg` that runs on a clean Mac with **no Homebrew**.
  - The sidecars are bundled.
  - A first-run AI setup screen is added.
  - The licenses are reviewed.
  - An accessibility pass and a local-only error report are added.
- **SPEC sections:** §18 P6, §15, §16
- **Branch:** `p6/packaging` → tag `v1.0.0`

## 0. Scope

**In scope:**
- Static builds of `llama-server` and `whisper-server`, bundled as Tauri `externalBin`
- Code signing and notarization (if a Developer ID is available), otherwise an ad-hoc signed local build
- First-run AI setup
- About window and license notices
- Accessibility pass
- Crash/diagnostics dialog
- Final measurement of every performance budget
- Clean-machine test

**Out of scope:**
- An auto-updater
- The Mac App Store (it would require the App Sandbox, which conflicts with the sidecars)
- Intel/universal builds (the target is Apple Silicon only; these can be added later)

---

## 1. Sidecar builds (`scripts/build-sidecars.sh`)

Prerequisite: `brew install cmake` (build machine only).

```bash
LLAMA_TAG=<pinned tag>      # record in docs/dev/notes/P6-sidecars.md
WHISPER_TAG=<pinned tag>
TRIPLE=aarch64-apple-darwin

# llama.cpp → llama-server (static, Metal shaders embedded)
git clone --depth 1 --branch $LLAMA_TAG https://github.com/ggml-org/llama.cpp build/llama.cpp
cmake -S build/llama.cpp -B build/llama.cpp/build -DCMAKE_BUILD_TYPE=Release \
      -DBUILD_SHARED_LIBS=OFF -DGGML_METAL=ON -DGGML_METAL_EMBED_LIBRARY=ON -DLLAMA_CURL=OFF
cmake --build build/llama.cpp/build --target llama-server -j
cp build/llama.cpp/build/bin/llama-server src-tauri/binaries/llama-server-$TRIPLE

# whisper.cpp → whisper-server
git clone --depth 1 --branch $WHISPER_TAG https://github.com/ggml-org/whisper.cpp build/whisper.cpp
cmake -S build/whisper.cpp -B build/whisper.cpp/build -DCMAKE_BUILD_TYPE=Release \
      -DBUILD_SHARED_LIBS=OFF -DGGML_METAL=ON -DGGML_METAL_EMBED_LIBRARY=ON
cmake --build build/whisper.cpp/build --target whisper-server -j
cp build/whisper.cpp/build/bin/whisper-server src-tauri/binaries/whisper-server-$TRIPLE

# verify: only system libraries / frameworks
otool -L src-tauri/binaries/*-$TRIPLE | grep -v -E "/usr/lib/|/System/Library/" && exit 1 || true
```

- Check the exact CMake option and target names against each repository's build docs for the pinned tags.
- **Pin the tags to the versions P2 and P5 were tested with.** Re-run the P2 T0 and P5 T0 spike checks against the built binaries.
- `src-tauri/binaries/` is git-ignored. CI or release builds run the script first.
- `tauri.conf.json`:

```json
"bundle": {
  "externalBin": ["binaries/llama-server", "binaries/whisper-server"],
  "macOS": { "minimumSystemVersion": "13.0", "entitlements": "Entitlements.plist", "infoPlist": "Info.plist" }
}
```

Tauri copies the sidecars to `Contents/MacOS/`, next to the main executable. This matches step 2 of the binary resolution order in P2 §8.3, so no code change is needed. A log line at startup records which path was resolved.

---

## 2. Signing & notarization

### 2.1 With an Apple Developer ID (preferred for sharing)

- **`Entitlements.plist`** (hardened runtime, no App Sandbox):
  - `com.apple.security.device.audio-input` = true
  - any entitlement Tauri's WKWebView template requires (check the current Tauri macOS signing guide)
- **Environment variables.** The **developer sets these in their own shell or keychain.** The assistant never enters credentials.
  - `APPLE_SIGNING_IDENTITY`
  - notarization: `APPLE_ID` + `APPLE_PASSWORD` (app-specific password) + `APPLE_TEAM_ID`, **or** the App Store Connect API key variables
- `npm run tauri build` signs the app and its sidecars, then notarizes and staples.
- **Verify:**
  - `codesign --verify --deep --strict --verbose=2 "Tech English.app"`
  - `spctl -a -vv "Tech English.app"` → `accepted source=Notarized Developer ID`
  - `codesign -d --entitlements - "…/Contents/MacOS/llama-server"` (the sidecars are signed)

### 2.2 Without a Developer ID (personal use)

- Ad-hoc signing: `"signingIdentity": "-"`.
- The first launch needs right-click → Open, or System Settings › Privacy & Security › "Open Anyway". Document this in the README.
- Microphone permission works with ad-hoc signing. However, **every rebuild may cause macOS to ask again**, because the signature changes.

---

## 3. First-run AI setup (onboarding step 5)

This step comes after the existing onboarding steps. It can be skipped.

```
AI features (optional, runs on this Mac)
  ☑ Language model — <name> · 2.6 GB · License: <name> (link)
  ☑ Speech recognition — Whisper base.en · 150 MB · License: MIT (link)
  Disk space available: 690 GB
  [Download now]   [Later]
```

- The downloads continue in the background, with progress shown in the widget footer ("Downloading AI 42 %").
- **Later** → AI buttons show the setup card from P2 §12.
- **Before** offering the downloads, check the sidecar binaries: run `llama-server --version` and `whisper-server --help`. If one fails, show "AI engine missing — reinstall the app".

---

## 4. Licenses & About

- `docs/licenses.md` and the **About** window (tray › About Tech English) list the following:
  - the app version, commit, and build date
  - third-party Rust crates: generated with `cargo about` (or `cargo license`) into `resources/THIRD_PARTY_RUST.txt`
  - npm packages: generated with `npx license-checker --production --summary` plus the full text into `resources/THIRD_PARTY_JS.txt`
  - llama.cpp (MIT) and whisper.cpp (MIT), with copyright notices
  - **NGSL 1.2 — CC BY-SA 4.0**, with its attribution line
  - **Models are not bundled.** Each model's license is shown before download and listed in About once it is downloaded.
- **Review gate.** Check that no dependency uses a license that conflicts with how the app will be distributed. See Q7 — the app's own license and distribution are not decided yet.

---

## 5. Accessibility pass

- Every interactive element has an accessible name. VoiceOver can read the widget pick card, the quiz cards and the talk status.
- Full keyboard use: Tab order; visible focus rings; the shortcuts (⌘1–4, Space/1/2/3 in the quiz, Space push-to-talk) are listed in Help › Keyboard Shortcuts.
- Contrast is ≥ 4.5:1 in both themes (check with the Accessibility Inspector).
- With `prefers-reduced-motion`, the pulsing status dot and the typing cursor become static.
- Text size: the Reader font setting also scales the quiz and talk transcripts.

---

## 6. Errors & diagnostics (local only)

- A Rust panic hook writes `~/Library/Logs/com.techenglish.app/crash-<ts>.txt` (message, backtrace, version). **It contains no user content.**
- On the next launch after a crash → a dialog: "Tech English closed unexpectedly." with **[Copy diagnostics]** and [OK].
- **Settings › About › Copy diagnostics** copies:
  - the version
  - the macOS version and chip
  - the mode
  - the sidecar states and resolved paths
  - the model ids
  - feed error counts
  - the last 200 log lines (the logs contain no content; see README §2.7)
- A React error boundary on each page shows "Something went wrong on this page" with [Reload page] [Copy diagnostics].
- **Nothing is ever sent over the network.**

---

## 7. Final performance run

- Re-measure every row of SPEC §16 on a **release build** of the bundled app.
- Add a "v1.0.0" section to `docs/perf.md`, comparing it with the earlier phases.
- If any budget is missed, create a follow-up task with the measured value and its cause.

---

## 8. Tasks

| # | Task | Depends on | Done when |
|---|---|---|---|
| T1 | `build-sidecars.sh` + pinned tags + externalBin config | – | The `otool` check passes; the bundled app uses the bundled binaries (log line) with Homebrew's copies unlinked (`brew unlink llama.cpp whisper.cpp`) |
| T2 | Entitlements + Info.plist + signing config (both paths) | T1 | §2 verification commands pass (Developer ID) **or** the ad-hoc build launches and the mic works |
| T3 | First-run AI setup | T1 | Manual: fresh profile → download both → AI and voice work |
| T4 | Licenses + About window | – | Generated files are present; the About window lists everything in §4; review gate recorded |
| T5 | Accessibility pass | – | Checklist in `docs/qa/P6.md` §A done |
| T6 | Crash/diagnostics | – | Forced panic (debug menu) → the next launch shows the dialog; the diagnostics contain no article, vocab or transcript text |
| T7 | Final perf run | T1–T3 | `docs/perf.md` v1.0.0 section |
| T8 | Clean-machine test + README install guide | all | §9 checklist passes on a new macOS user account with Homebrew's binaries unlinked |

> **As built (2026-09-30)** — see `docs/dev/notes/P6-sidecars.md`, `docs/licenses.md` and `docs/qa/P6.md`.
> - **T1:** tags v0.5.0 (llama.cpp) and v1.9.4 (whisper.cpp); `LLAMA_CURL` no longer exists (`LLAMA_OPENSSL=OFF`, `LLAMA_USE_PREBUILT_UI=OFF` instead); minimum macOS is **13.3**. `externalBin` and the signing settings are in `src-tauri/tauri.bundle.conf.json` (passed with `--config`), so `cargo build`/`test` work without the built engines.
> - **T2:** personal use → ad-hoc signing (§2.2) with the hardened runtime and `Entitlements.plist` (`audio-input`). `src-tauri/Info.plist` is merged by Tauri automatically. The Developer ID path (§2.1) was not set up.
> - **T3:** onboarding step 5 with `check_engines` (`llama-server --version`, `whisper-server --help`); the widget footer shows "Downloading AI n %".
> - **T4:** About is a page, **Settings › About** (tray › About Tech English opens it), not a separate window. The lists are made by `scripts/gen-licenses.py` (no extra tools to install) instead of `cargo about` / `license-checker`.
> - **T5:** reduced motion, a text-size setting (`readingScale`: stories, quiz cards, talk transcript), contrast ≥ 4.5:1 for every text colour in both themes, shortcuts listed in Settings › About, robot-like voices grouped apart in the voice pickers.
> - **T6:** panic hook → `crash-<unix time>.txt` in the log folder (5 kept); crash notice on the next start in Settings › About; **Copy diagnostics** (`pbcopy`); a route error page. `TECH_ENGLISH_DEBUG_PANIC=1` crashes at start to test a release build.
> - **T8:** the DMG is made with `scripts/make-dmg.sh` (hdiutil; no Finder scripting). The clean-account test is a manual check.

## 9. Clean-machine checklist (`docs/qa/P6.md`)

- [ ] Install from the DMG (drag to Applications); first launch passes Gatekeeper (notarized) or the documented ad-hoc steps
- [ ] Onboarding → AI setup → downloads complete; the sha is verified
- [ ] Daily pick, Reader, B1 Summary, Word Book, quiz, voice session and review all work
- [ ] The Activity Monitor process list shows `llama-server` / `whisper-server` running from inside the app bundle, and stopping when idle
- [ ] Hibernate → only the app and WebKit processes remain; memory is within budget
- [ ] Launch at login works after a reboot
- [ ] Deleting the app and its `~/Library/Application Support/com.techenglish.app` folder removes everything (documented in the README under "Uninstall")
