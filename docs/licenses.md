# Licenses

What ships inside **Tech English.app**, and under which license. The same notices are shown in the app under **Settings › About › Licenses**.

## Parts of the app

| Part | License | Where the text is |
|---|---|---|
| llama.cpp (`llama-server`, tag v0.5.0) | MIT | `resources/NOTICES.txt` |
| whisper.cpp (`whisper-server`, tag v1.9.4) | MIT | `resources/NOTICES.txt` |
| New General Service List 1.2 (word list for story difficulty) | CC BY-SA 4.0 | attribution in `resources/NOTICES.txt` and at the top of `resources/ngsl.txt` |
| Rust crates (354 in the macOS build) | see below | `resources/THIRD_PARTY_RUST.txt` |
| npm packages (16 in the production bundle) | see below | `resources/THIRD_PARTY_JS.txt` |

**Not part of the app:** the AI models (downloaded by the user; each license is shown before the download and listed in About afterwards), the macOS dictionary and the macOS voices.

## How the lists are made

```bash
python3 scripts/gen-licenses.py
```

It uses `cargo tree` (normal dependencies for `aarch64-apple-darwin`), `cargo metadata` and `npm ls --omit=dev`, reads the license files from the packages on disk, and writes the two `THIRD_PARTY_*.txt` files. Run it again when dependencies change.

## Review (2026-09-30)

| License | Rust crates | npm packages |
|---|---|---|
| MIT / Apache-2.0 family (incl. `MIT OR Apache-2.0`, `Zlib OR Apache-2.0 OR MIT`, `Unlicense OR MIT`, `0BSD OR …`, `CC0-1.0 OR MIT-0 OR Apache-2.0`) | 318 | 15 |
| Unicode-3.0 (ICU data; alone or combined) | 19 | – |
| BSD-2-Clause / BSD-3-Clause (alone, combined or as a choice) | 10 | – |
| MPL-2.0 (alone or as a choice) | 5 | 1 (`MPL-2.0 OR Apache-2.0`: DOMPurify) |
| Zlib | 1 | – |
| `LGPL-3.0-or-later OR MPL-2.0` | 1 (`priority-queue`) | – |
| **Total** | **354** | **16** |

- **No GPL, AGPL or "non-commercial" licenses.** The script flags them; the only flag is `priority-queue`, which is dual-licensed: we use it under **MPL-2.0**.
- **MPL-2.0** is a file-level copyleft: it only asks that changes to *those files* stay open. We do not change them, so there is nothing to publish.
- **CC BY-SA 4.0 (NGSL):** attribution is given. The word list is shipped unchanged in meaning (lemmatized forms, one per line); if the list itself is changed and shared, it must stay CC BY-SA.
- 23 Rust crates have no license file inside the package; their SPDX license is in the summary list.

**Result:** nothing conflicts with personal use or with sharing the app for free. The app's own license is still not chosen (SPEC Q7: personal use only, decided 2026-09-29), so this review is informational.
