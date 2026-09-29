#!/usr/bin/env bash
# Quality gate — run before every commit (docs/dev/README.md §2.9).
set -euo pipefail
cd "$(dirname "$0")/.."

echo "== rust fmt";    cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
echo "== rust clippy"; cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
echo "== rust tests";  cargo test --manifest-path src-tauri/Cargo.toml --quiet
echo "== typecheck";   npm run --silent typecheck
echo "== lint";        npm run --silent lint
echo "== web tests";   npx vitest --run --silent

echo "== no direct Utc::now()"
if grep -rn "Utc::now()" src-tauri/src --include=*.rs | grep -v "src-tauri/src/clock.rs"; then
  echo "Use the Clock trait instead of Utc::now()" >&2
  exit 1
fi
echo "All checks passed."
