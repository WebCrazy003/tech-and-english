# Tech English

A local-first macOS menu-bar app. It picks one interesting tech story a day for you and (in later phases) turns it into English practice: an easy reader, a Word Book, quizzes and a voice tutor. Everything runs on your Mac. No paid APIs are used.

- Product spec: [docs/SPEC.md](docs/SPEC.md)
- Development plan per phase: [docs/dev/](docs/dev/README.md)
- Current phase: **P5 — Voice tutor** (P1–P4 done: v0.1.0 news, v0.2.0 reader + AI chat, v0.3.0 learning & sources, v0.4.0 words)

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

## AI setup (local, free)

The Reader's summaries and chat use a local model through llama.cpp's `llama-server`:

1. **Engine.** The app looks for `llama-server` in its data folder (`…/com.techenglish.app/bin/*/llama-server`), in Homebrew, and in your PATH. Homebrew has no build for macOS 14 on this setup, so the official build from https://github.com/ggml-org/llama.cpp/releases (`llama-bXXXX-bin-macos-arm64.tar.gz`) is unpacked into the data folder.
2. **Model.** Settings › AI → Download (Qwen3.5 4B, 2.7 GB, recommended). Two alternatives are listed there too.

The model starts when you first use the AI and unloads after 10 idle minutes or in Hibernate mode.

## Lessons and example sources

- **Today's lesson.** Turn on **Learn** for a topic (Settings › Topics). Each day the app then also picks a tutorial or explainer for that topic, next to the news story.
- **Add from an example.** Settings › News sources: paste an article you like. The app finds that site's feed, and can save the article so you can read it right away.

## Words and practice

- **Select a word** (1–8 words) in a story or an AI answer: the popup shows the macOS dictionary meaning, 🔊, **Explain simply** (AI) and **Add to Word Book**. Saved words are underlined lightly in stories.
- **Word Book** (⌘3): search, edit, export to CSV (saved in Downloads). **Practice** (⌘4): a 10-word quiz; Space shows the answer, 1 / 2 / 3 grade it. Words come back on a spaced-repetition schedule (FSRS).
- Better voices: System Settings › Accessibility › Spoken Content › System Voice › Manage Voices… then choose one in Settings › Voice.

## Check and test

```bash
scripts/check.sh                                   # fmt, clippy, all tests, typecheck, lint
cargo test --manifest-path src-tauri/Cargo.toml --test live_feeds -- --ignored --nocapture   # live feeds (internet)
cargo test --manifest-path src-tauri/Cargo.toml --test live_feeds discover_examples -- --ignored --nocapture   # "find feed" on real URLs
DB_COPY=/path/to/copy-of-app.db cargo test --manifest-path src-tauri/Cargo.toml --test live_feeds real_db_copy -- --ignored --nocapture   # upgrade a COPY of real data
TE_MODEL=qwen3.5-4b cargo test --manifest-path src-tauri/Cargo.toml --release --test live_ai -- --ignored --nocapture   # real model
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
