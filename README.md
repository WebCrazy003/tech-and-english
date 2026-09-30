# Tech English

A local-first macOS menu-bar app. It picks one interesting tech story a day for you and turns it into English practice: an easy reader, a Word Book, quizzes and a voice tutor. Everything runs on your Mac. No paid APIs are used.

- Product spec: [docs/SPEC.md](docs/SPEC.md)
- Development plan per phase: [docs/dev/](docs/dev/README.md)
- Status: **v1.0.0** — all six phases are done (news, reader + AI chat, learning & sources, words, voice tutor, packaging)
- Licenses of everything inside the app: [docs/licenses.md](docs/licenses.md)

## Requirements

- macOS 13.3+ on Apple Silicon (built and tested on a Mac mini M1, 16 GB)
- To build: Rust (stable), Node 20+ and npm, Xcode Command Line Tools, and `cmake` (for the AI engines)
- To run the built app: nothing else. No Homebrew is needed; the AI engines are inside the app.

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

The AI runs on your Mac with two small engines that are **inside the app** (`llama-server` and `whisper-server`, built by `scripts/build-sidecars.sh`). You only download the models:

1. **First start.** The last setup step offers the language model (Qwen3.5 4B, 2.7 GB) and the speech model (Whisper base.en, 150 MB). The downloads continue in the background; the widget shows the progress. You can skip this and do it later.
2. **Later.** Settings › AI (language models) and Settings › Voice (speech models).

The language model starts when you first use the AI and unloads after 10 idle minutes or in Hibernate mode. The speech engine runs only during a talk.

In development (`npm run tauri dev`) the app looks for the engines next to the app, then in its data folder (`…/com.techenglish.app/bin/*/`), then in your PATH and Homebrew.

## Lessons and example sources

- **Today's lesson.** Turn on **Learn** for a topic (Settings › Topics). Each day the app then also picks a tutorial or explainer for that topic, next to the news story.
- **Add from an example.** Settings › News sources: paste an article you like. The app finds that site's feed, and can save the article so you can read it right away.

## Words and practice

- **Select a word** (1–8 words) in a story or an AI answer: the popup shows the macOS dictionary meaning, 🔊, **Explain simply** (AI) and **Add to Word Book**. Saved words are underlined lightly in stories.
- **Word Book** (⌘3): search, edit, export to CSV (saved in Downloads). **Practice** (⌘4): a 10-word quiz; Space shows the answer, 1 / 2 / 3 grade it. Words come back on a spaced-repetition schedule (FSRS).
- Better voices: System Settings › Accessibility › Spoken Content › System Voice › Manage Voices… then choose one in Settings › Voice.

## Talk (voice tutor)

- **Talk** (⌘5): choose a story, then **Start conversation**. Hold **Space** (or click the mic) while you speak. The tutor answers out loud, corrects important mistakes and asks you to repeat the correct sentence.
- Say "speak slower please", "say that again" or "how do I say *scalability*?". Click a word in the tutor's text to look it up or to practise saying it.
- **End** shows a review: you choose which words and corrections go into the Word Book.
- The first time, macOS asks for the microphone. Audio is processed on your Mac and is never saved.
- While the app reads aloud (tutor, or **Listen** in the Reader), the sentence and the word being spoken are highlighted.

## Check and test

```bash
scripts/check.sh                                   # fmt, clippy, all tests, typecheck, lint
cargo test --manifest-path src-tauri/Cargo.toml --test live_feeds -- --ignored --nocapture   # live feeds (internet)
cargo test --manifest-path src-tauri/Cargo.toml --test live_feeds discover_examples -- --ignored --nocapture   # "find feed" on real URLs
DB_COPY=/path/to/copy-of-app.db cargo test --manifest-path src-tauri/Cargo.toml --test live_feeds real_db_copy -- --ignored --nocapture   # upgrade a COPY of real data
TE_MODEL=qwen3.5-4b cargo test --manifest-path src-tauri/Cargo.toml --release --test live_ai -- --ignored --nocapture   # real model
cargo test --manifest-path src-tauri/Cargo.toml --test golden_tutor -- --ignored --nocapture golden_tutor   # tutor corrections, 30 cases, real model
cargo test --manifest-path src-tauri/Cargo.toml --test live_voice -- --ignored --nocapture   # a spoken session on the real engines (no microphone needed)
scripts/measure-idle.sh "Standard" 10              # idle memory/CPU of the release build
python3 scripts/gen-licenses.py                    # refresh the third-party license lists
```

## Build and install

```bash
scripts/build-sidecars.sh        # once: builds the two AI engines into src-tauri/binaries/ (about 5 minutes)
npm run tauri build -- --bundles app --config src-tauri/tauri.bundle.conf.json
```

The output is `src-tauri/target/release/bundle/macos/Tech English.app`. Drag it to **Applications**, or use the Desktop shortcuts: `scripts/Tech English - Start.command` and `scripts/Tech English - Stop.command`. Start builds the engines and the app first if needed.

**First launch.** The app is for personal use, so it is signed "ad hoc" (no Apple Developer ID):

- An app you built on this Mac opens normally.
- If you copied the app from another Mac, macOS blocks it the first time: right-click the app and choose **Open**, or allow it in System Settings › Privacy & Security › **Open Anyway**.
- After each rebuild, macOS may ask for the microphone again, because the signature changes.

## Where data lives

- Database and AI models: `~/Library/Application Support/com.techenglish.app/` (`app.db`, `models/`)
- Logs and crash reports: `~/Library/Logs/com.techenglish.app/` (IDs, counts and timings only; no stories, words or conversations)
- **Settings › About › Copy diagnostics** copies version, system and engine status for a bug report. Nothing is ever sent over the network.

## Uninstall

1. Quit the app (menu bar icon › Quit Tech English).
2. Move **Tech English.app** to the Trash.
3. Delete `~/Library/Application Support/com.techenglish.app` and `~/Library/Logs/com.techenglish.app`.
4. If you turned on "Launch at login": System Settings › General › Login Items › remove Tech English.
