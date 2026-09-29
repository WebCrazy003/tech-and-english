#!/usr/bin/env bash
# Measure idle memory/CPU of the release app (docs/dev/P1 §12).
# Usage: scripts/measure-idle.sh <label> [minutes=10] [--hibernate]
# Starts the release app with a fresh seeded data folder, waits 2 min (first fetch),
# then samples the app + the WebKit processes it started every 10 s.
set -euo pipefail
cd "$(dirname "$0")/.."
label=$1; minutes=${2:-10}; flag=${3:-}
app="src-tauri/target/release/bundle/macos/Tech English.app/Contents/MacOS/tech-english"
dir=$(mktemp -d -t techenglish-perf)
python3 scripts/seed-test-data.py "$dir" $flag >/dev/null

webkit_pids() { { pgrep -f "com.apple.WebKit" || true; } | sort; }
before=$(webkit_pids)
TECH_ENGLISH_DATA_DIR="$dir" "$app" >/dev/null 2>&1 &
app_pid=$!
sleep 15
wk=$(comm -13 <(echo "$before") <(webkit_pids) | tr '\n' ' ')
pids="$app_pid $wk"
echo "[$label] app pid $app_pid, webkit pids: $wk" >&2
sleep 105

cpu_secs() { # total CPU seconds of all pids
  ps -o time= -p $(echo $pids | tr ' ' ',') 2>/dev/null | awk -F'[:.]' '{ if (NF==3) s+=$1*60+$2+$3/100; else s+=$1*3600+$2*60+$3+$4/100 } END { printf "%.2f", s }'  # M:SS.cc or H:MM:SS.cc
}
rss_mb() { ps -o rss= -p $(echo $pids | tr ' ' ',') 2>/dev/null | awk '{ s+=$1 } END { printf "%.1f", s/1024 }'; }

c0=$(cpu_secs); t0=$(date +%s); max=0; sum=0; n=0
for _ in $(seq 1 $((minutes * 6))); do
  r=$(rss_mb); sum=$(echo "$sum + $r" | bc); n=$((n + 1))
  if (( $(echo "$r > $max" | bc) )); then max=$r; fi
  sleep 10
done
c1=$(cpu_secs); t1=$(date +%s)
kill "$app_pid" 2>/dev/null || true
avg=$(echo "scale=1; $sum / $n" | bc)
cpu=$(echo "scale=3; ($c1 - $c0) * 100 / ($t1 - $t0)" | bc)
articles=$(sqlite3 "$dir/app.db" "select count(*) from articles" 2>/dev/null || echo "?")
echo "| $label | ${minutes} min | ${avg} MB | ${max} MB | ${cpu} % | $articles articles |"
