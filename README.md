# Tech English

A local-first macOS menu-bar app. It picks one interesting tech story a day for you and (in later phases) turns it into English practice: an easy reader, a Word Book, quizzes and a voice tutor. Everything runs on your Mac. No paid APIs are used.

- Product spec: [docs/SPEC.md](docs/SPEC.md)
- Development plan per phase: [docs/dev/](docs/dev/README.md)
- Current phase: **P1 — App shell & News** (no AI yet)

## Requirements

- macOS 13+ on Apple Silicon (built and tested on a Mac mini M1, 16 GB)
- Rust (stable), Node 20+ and npm, Xcode Command Line Tools

## Run

```bash
npm install
npm run tauri dev
```

The widget appears at the top-right of the screen, and a speech-bubble icon appears in the menu bar. The first launch opens the setup window.

To test without touching your real data, use a throwaway data folder:

```bash
python3 scripts/seed-test-data.py /tmp/te-test
TECH_ENGLISH_DATA_DIR=/tmp/te-test npm run tauri dev
```

**Preview the UI in a normal browser** (fake data, no Rust): run `npm run dev` and open http://localhost:1420/ or http://localhost:1420/widget.html. Add `?fresh` to start at the setup screen.

## Check and test

```bash
scripts/check.sh                                   # fmt, clippy, all tests, typecheck, lint
cargo test --manifest-path src-tauri/Cargo.toml --test live_feeds -- --ignored --nocapture   # live feeds (internet)
scripts/measure-idle.sh "Standard" 10              # idle memory/CPU of the release build
```

## Build

```bash
npm run tauri build -- --bundles app
```

The output is `src-tauri/target/release/bundle/macos/Tech English.app`.

**Desktop shortcuts:** `scripts/Tech English - Start.command` and `scripts/Tech English - Stop.command` (linked on the Desktop). Start rebuilds the app first if the code changed since the last build. The app is for personal use, so it is not signed. The first time you open it, right-click the app and choose **Open**.

## Where data lives

- Database: `~/Library/Application Support/com.techenglish.app/app.db`
- Logs: `~/Library/Logs/com.techenglish.app/` (IDs and counts only, no article text)

To uninstall, delete the app and those two folders.
