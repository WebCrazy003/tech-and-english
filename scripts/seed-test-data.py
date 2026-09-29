#!/usr/bin/env python3
"""Create a ready-to-use test data folder (onboarding done, all default topics + feeds).

Usage: scripts/seed-test-data.py <data-dir> [--hibernate]
Then:  TECH_ENGLISH_DATA_DIR=<data-dir> "src-tauri/target/release/bundle/macos/Tech English.app/Contents/MacOS/tech-english"
"""
import json, pathlib, sqlite3, sys

root = pathlib.Path(__file__).resolve().parent.parent
data_dir = pathlib.Path(sys.argv[1])
data_dir.mkdir(parents=True, exist_ok=True)
db_path = data_dir / "app.db"
if db_path.exists():
    sys.exit(f"{db_path} already exists")

now = "2026-01-01T00:00:00Z"
db = sqlite3.connect(db_path)
db.executescript((root / "src-tauri/migrations/0001_init.sql").read_text())
db.execute("PRAGMA user_version = 1")
db.execute("INSERT INTO settings(key, value) VALUES ('app', ?)", (json.dumps({"onboardingDone": True}),))
if "--hibernate" in sys.argv:
    db.execute("INSERT INTO app_state(key, value) VALUES ('mode', 'hibernate')")
for t in json.loads((root / "src-tauri/resources/default_topics.json").read_text()):
    db.execute(
        "INSERT INTO topics(name, keywords, excluded_keywords, priority, notify, created_at) VALUES (?,?,?,?,?,?)",
        (t["name"], json.dumps(t["keywords"]), json.dumps(t["excludedKeywords"]), t["priority"], int(t["priority"] == 3), now),
    )
for f in json.loads((root / "src-tauri/resources/default_feeds.json").read_text()):
    db.execute(
        "INSERT INTO feeds(kind, name, url, source_weight, created_at) VALUES (?,?,?,?,?)",
        (f["kind"], f["name"], f["url"], f["sourceWeight"], now),
    )
db.commit()
print(f"seeded {db_path}")
