#!/usr/bin/env python3
"""Collects the licenses of the third-party code that ships in the app (P6 dev spec §4).

Writes src-tauri/resources/THIRD_PARTY_RUST.txt and THIRD_PARTY_JS.txt (shown in About) and
prints a summary for the license review. Uses only cargo, npm and the files already on disk:

    python3 scripts/gen-licenses.py
"""
import hashlib
import json
import re
import subprocess
import sys
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
TAURI = ROOT / "src-tauri"
OUT = TAURI / "resources"
LICENSE_FILE = re.compile(r"^(licen[sc]e|copying|unlicense|notice)", re.I)
# Licenses that would need a closer look before sharing the app.
REVIEW = re.compile(r"GPL|AGPL|SSPL|CC-BY-NC|proprietary|unknown", re.I)


def run(cmd, cwd):
    return subprocess.run(cmd, cwd=cwd, check=True, capture_output=True, text=True).stdout


def license_texts(folder: Path):
    out = []
    for f in sorted(folder.iterdir()) if folder.is_dir() else []:
        if f.is_file() and LICENSE_FILE.match(f.name) and f.stat().st_size < 60_000:
            try:
                out.append(f.read_text(errors="replace").strip())
            except OSError:
                pass
    return out


def write(path: Path, title: str, packages):
    """packages: list of (name, version, license, [texts])"""
    packages = sorted(packages, key=lambda p: (p[0].lower(), p[1]))
    by_text = defaultdict(list)
    texts = {}
    for name, version, _lic, tx in packages:
        for t in tx:
            key = hashlib.sha1(re.sub(r"\s+", " ", t).encode()).hexdigest()
            texts[key] = t
            by_text[key].append(f"{name} {version}")
    lines = [title, "=" * len(title), "", f"{len(packages)} packages. Summary first, then the license texts.", ""]
    lines += [f"{n} {v} — {lic}" for n, v, lic, _ in packages]
    lines += ["", "", "License texts", "=============", ""]
    for key, users in sorted(by_text.items(), key=lambda kv: kv[1][0].lower()):
        lines += ["-" * 78, "Used by: " + ", ".join(users), "-" * 78, "", texts[key], "", ""]
    path.write_text("\n".join(lines) + "\n")
    return packages


def rust():
    tree = run(
        ["cargo", "tree", "-e", "normal", "--target", "aarch64-apple-darwin", "--prefix", "none", "--format", "{p}|{l}"],
        TAURI,
    )
    meta = json.loads(run(["cargo", "metadata", "--format-version", "1"], TAURI))
    dirs = {(p["name"], p["version"]): Path(p["manifest_path"]).parent for p in meta["packages"]}
    seen, packages = set(), []
    for line in tree.splitlines():
        m = re.match(r"^(\S+) v(\S+)(?: \(.*\))?\|(.*)$", line.strip())
        if not m or (m.group(1), m.group(2)) in seen or m.group(1) == "tech-english":
            continue
        seen.add((m.group(1), m.group(2)))
        name, version, lic = m.group(1), m.group(2), m.group(3) or "unknown"
        packages.append((name, version, lic, license_texts(dirs.get((name, version), Path("/nonexistent")))))
    return write(OUT / "THIRD_PARTY_RUST.txt", "Third-party Rust crates in Tech English", packages)


def js():
    paths = run(["npm", "ls", "--omit=dev", "--all", "--parseable"], ROOT).splitlines()
    seen, packages = set(), []
    for p in paths:
        folder = Path(p)
        pkg = folder / "package.json"
        if folder == ROOT or not pkg.is_file():
            continue
        d = json.loads(pkg.read_text())
        key = (d.get("name"), d.get("version"))
        if key in seen:
            continue
        seen.add(key)
        lic = d.get("license") or "unknown"
        if isinstance(lic, dict):
            lic = lic.get("type", "unknown")
        packages.append((d.get("name", folder.name), d.get("version", "?"), lic, license_texts(folder)))
    return write(OUT / "THIRD_PARTY_JS.txt", "Third-party JavaScript packages in Tech English", packages)


def main():
    flagged = []
    for label, packages in (("Rust", rust()), ("JavaScript", js())):
        kinds = defaultdict(int)
        for name, version, lic, texts in packages:
            kinds[lic] += 1
            if REVIEW.search(lic):
                flagged.append(f"{label}: {name} {version} — {lic}")
        missing = [f"{n} {v}" for n, v, _, t in packages if not t]
        print(f"{label}: {len(packages)} packages")
        for lic, n in sorted(kinds.items(), key=lambda kv: -kv[1]):
            print(f"  {n:4d}  {lic}")
        print(f"  without a license file in the package: {len(missing)}")
    print("\nNeeds a closer look:" if flagged else "\nNo copyleft or unknown licenses found.")
    for f in flagged:
        print("  " + f)
    return 0


if __name__ == "__main__":
    sys.exit(main())
