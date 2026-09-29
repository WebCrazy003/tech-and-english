#!/bin/zsh -l
# Stop Tech English (any running copy, release or dev).
set -eu

if ! pgrep -x tech-english >/dev/null; then
  echo "Tech English is not running."
  exit 0
fi

pkill -x tech-english
for _ in {1..10}; do
  pgrep -x tech-english >/dev/null || { echo "Tech English stopped."; exit 0; }
  sleep 0.5
done

pkill -9 -x tech-english
echo "Tech English stopped (forced)."
