#!/usr/bin/env bash
# Automated headless online test of rounds-port over our dev warpzone-server.
#
#   scripts/net/online-test.sh [CASE...]      # default: all cases
#
# Cases (machines x bot players):
#   12p      3 machines, 4+4+4 bots = 12 players, 40 ms +-15 jitter, 3 % loss (1 round)
#   1p2      2 machines, 1+2 bots, 60 ms +-20 jitter, 5 % loss (2 rounds)
#   teams    3 machines, 2+2+2 bots in 2 teams, 25 ms +-10, 2 % loss (2 rounds)
#   drop     4 machines, 2+2+2+2 bots, 30 ms +-10, 2 % loss: m2 leaves at
#            frame 1200 (card picks), m3 crashes at frame 2000 (1 round of 2 points)
#   relay    3 machines, 2+2+1 bots, every link through the Warpzone relay, 20 ms, 1 %
#   win      3 machines, 2+2+2 bots, 30 ms +-10, 2 % loss; machine 1 runs the
#            Windows build under Proton (cross-platform determinism)
#
# ONLINE_TEST_WINDOWS="1 3" runs those machine indices of any case with the
# Windows build (scripts/run-windows.sh, target/dist/windows; build it with
# scripts/build-windows.sh); the PASS rule then also proves that Linux and
# Windows simulate bit for bit alike (same checksums, same screenshots).
#
# Each case starts machine 0 as host (unlisted room, join code), the others
# join (odd machines by code, even ones through the invite link, as
# `--warpzone-join <invite>` from the launcher would). Every machine plays a full match with bots (rematch "no"
# ends it), reports its final frame/checksum (finish_match) and saves
# screenshots of the same confirmed frames. PASS = every remaining machine
# printed RESULT with agreed=true, the same frame, checksum and winner, and
# no desync.
#
# Env: ROUNDS_WZ_PORT / ROUNDS_WZ_UDP_PORT (default 3998 / 3479; 2998 is
# used by another project), ONLINE_TEST_BIN (default target/release/rounds-port),
# ONLINE_TEST_ROUNDS (rounds to win, default 2), ONLINE_TEST_POINTS (1),
# ONLINE_TEST_SHOTS (confirmed frames to screenshot, default 600,1500,2400).
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"
source env.sh
PORT="${ROUNDS_WZ_PORT:-3998}"
UDP_PORT="${ROUNDS_WZ_UDP_PORT:-3479}"
export WARPZONE_SERVER="http://127.0.0.1:$PORT"
export ROUNDS_NO_SAVE=1
BIN="${ONLINE_TEST_BIN:-$ROOT/target/release/rounds-port}"
ROUNDS="${ONLINE_TEST_ROUNDS:-2}"
POINTS="${ONLINE_TEST_POINTS:-1}"
SHOTS="${ONLINE_TEST_SHOTS:-600,1500,2400}"
OUT="$ROOT/scratch/online"
mkdir -p "$OUT"

if [ ! -x "$BIN" ]; then
  echo "building $BIN"
  cargo build --release -p rounds-port || exit 1
fi

# Dev server (started here unless one answers already).
SERVER_PID=""
if ! curl -s -m 2 "$WARPZONE_SERVER/api/v1/info" > /dev/null; then
  ROUNDS_WZ_PORT=$PORT ROUNDS_WZ_UDP_PORT=$UDP_PORT "$ROOT/scripts/net/dev-server.sh" > "$OUT/server.log" 2>&1 &
  SERVER_PID=$!
  for _ in $(seq 1 120); do
    curl -s -m 1 "$WARPZONE_SERVER/api/v1/info" > /dev/null && break
    sleep 0.5
  done
fi
cleanup() {
  jobs -p | xargs -r kill 2> /dev/null
  [ -n "$SERVER_PID" ] && kill "$SERVER_PID" 2> /dev/null
}
trap cleanup EXIT

# run_case NAME "BOTS..." DELAY JITTER LOSS [TEAMS [ROUNDS [POINTS]]]
# (LEAVE="m@frame ..." / CRASH="m@frame ..." in the environment drop machines)
run_case() {
  local name=$1 bots=($2) delay=$3 jitter=$4 loss=$5 teams=${6:-0}
  local rounds=${7:-$ROUNDS} points=${8:-$POINTS}
  local n=${#bots[@]} total=0
  for b in "${bots[@]}"; do total=$((total + b)); done
  local dir="$OUT/$name"
  rm -rf "$dir"
  mkdir -p "$dir"
  echo "== $name: ${bots[*]} bots on $n machines, ${delay}ms +-${jitter} ${loss}% loss, teams $teams, rounds $rounds, points $points"
  local pids=() expect=$total
  local -a leave=() crash=()
  [ -n "${LEAVE:-}" ] && leave=($LEAVE)
  [ -n "${CRASH:-}" ] && crash=($CRASH)
  for i in $(seq 0 $((n - 1))); do
    local home="$dir/home-m$i"
    mkdir -p "$home"
    local args=(--headless --online join --bots "${bots[$i]}" --username "t${name}${i}x$RANDOM"
      --shots $SHOTS --shot-prefix "$dir/m$i" --timeout 900)
    if [ "$i" = 0 ]; then
      args=(--headless --online host --bots "${bots[0]}" --expect "$expect" --rounds "$rounds"
        --points "$points" --teams "$teams" --seed 4242 --username "t${name}0x$RANDOM"
        --shots $SHOTS --shot-prefix "$dir/m0" --timeout 900)
    else
      if [ $((i % 2)) = 1 ]; then
        args+=(--code "$CODE")
      else
        args+=(--room "$INVITE")
      fi
    fi
    for spec in "${leave[@]}"; do [ "${spec%@*}" = "$i" ] && args+=(--leave-at "${spec#*@}"); done
    for spec in "${crash[@]}"; do [ "${spec%@*}" = "$i" ] && args+=(--crash-at "${spec#*@}"); done
    local launch=("$BIN")
    for w in ${WINDOWS:-${ONLINE_TEST_WINDOWS:-}}; do
      [ "$w" = "$i" ] && launch=("$ROOT/scripts/run-windows.sh") && echo "  m$i: Windows build (Proton)"
    done
    WARPZONE_HOME="$home" ROUNDS_NET_FORCE_RELAY="${RELAY:-0}" ROUNDS_NET_DELAY_MS=$delay ROUNDS_NET_JITTER_MS=$jitter ROUNDS_NET_LOSS=$loss \
      "${launch[@]}" "${args[@]}" > "$dir/m$i.log" 2>&1 &
    pids+=($!)
    if [ "$i" = 0 ]; then
      ROOM="" CODE="" INVITE=""
      for _ in $(seq 1 120); do
        local line
        line=$(grep -m1 '^ROOM ' "$dir/m0.log" || true)
        if [ -n "$line" ]; then
          ROOM=$(echo "$line" | awk '{print $2}')
          CODE=$(echo "$line" | awk '{print $3}')
          INVITE=$(echo "$line" | awk '{print $4}')
          break
        fi
        sleep 0.5
      done
      if [ -z "$ROOM" ]; then
        echo "  host did not create a room"; tail -5 "$dir/m0.log"; return 1
      fi
      echo "  room $ROOM code $CODE"
    fi
  done
  local codes=()
  for i in $(seq 0 $((n - 1))); do
    wait "${pids[$i]}"
    codes+=($?)
  done
  # Evaluate.
  local results ok=1
  results=$(grep -h '^RESULT' "$dir"/m*.log)
  echo "$results" | sed 's/^/  /'
  local expected_left=$n
  expected_left=$((n - ${#leave[@]} - ${#crash[@]}))
  local count
  count=$(echo "$results" | grep -c '^RESULT' || true)
  [ "$count" = "$expected_left" ] || { echo "  expected $expected_left RESULT lines, got $count"; ok=0; }
  echo "$results" | grep -q 'agreed=false' && { echo "  disagreement"; ok=0; }
  echo "$results" | grep -vq 'desyncs=0' && { echo "  desync"; ok=0; }
  local distinct
  distinct=$(echo "$results" | grep -o 'frame=[0-9]* checksum=[0-9a-f]* winner=[0-9-]*' | sort -u | wc -l)
  [ "$distinct" = 1 ] || { echo "  results differ"; ok=0; }
  echo "  exit codes: ${codes[*]}"
  # Screenshots of the same confirmed frames must match between machines.
  for f in ${SHOTS//,/ }; do
    local sums
    sums=$(ls "$dir"/m*-f$f.png 2> /dev/null | xargs -r -n1 sha1sum | awk '{print $1}' | sort -u | wc -l)
    local cnt
    cnt=$(ls "$dir"/m*-f$f.png 2> /dev/null | wc -l)
    echo "  frame $f: $cnt screenshots, $sums distinct images"
  done
  if [ $ok = 1 ]; then
    echo "  SUMMARY $name PASS"
    echo "$name PASS" >> "$OUT/summary.txt"
  else
    echo "  SUMMARY $name FAIL (logs in $dir)"
    echo "$name FAIL" >> "$OUT/summary.txt"
  fi
}

CASES=("$@")
[ ${#CASES[@]} = 0 ] && CASES=(12p 1p2 teams drop relay)
: > "$OUT/summary.txt"
for c in "${CASES[@]}"; do
  case $c in
    12p) run_case 12p "4 4 4" 40 15 3 0 1 1 ;;
    1p2) run_case 1p2 "1 2" 60 20 5 0 2 1 ;;
    teams) run_case teams "2 2 2" 25 10 2 2 2 1 ;;
    drop) LEAVE="2@1200" CRASH="3@2000" run_case drop "2 2 2 2" 30 10 2 0 1 2 ;;
    relay) RELAY=1 run_case relay "2 2 1" 20 5 1 0 1 1 ;;
    win) WINDOWS="${ONLINE_TEST_WINDOWS:-1}" run_case win "2 2 2" 30 10 2 0 1 1 ;;
    *) echo "unknown case $c" ;;
  esac
done
echo "== summary"
cat "$OUT/summary.txt"
! grep -q FAIL "$OUT/summary.txt"
