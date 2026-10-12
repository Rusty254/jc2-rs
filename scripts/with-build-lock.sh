#!/usr/bin/env bash
# Limits concurrent cargo builds across agents to 2 slots (scripts/.lock/slot1, slot2).
# usage: with-build-lock.sh <label> <command> [args...]
# Waits until a slot is free, holds it for the command, releases on exit.
# A slot older than 45 minutes is assumed abandoned and is broken.
# Slots are shared by the main checkout and every git worktree: resolve the shared .git dir.
# Merges into the main checkout use scripts/with-merge-lock.sh (separate single lock).
COMMON="$(git -C "$(dirname "$0")" rev-parse --path-format=absolute --git-common-dir 2>/dev/null)"
if [ -n "$COMMON" ]; then DEFAULT_LOCK="$(dirname "$COMMON")/scripts/.lock"
else DEFAULT_LOCK="$(cd "$(dirname "$0")" && pwd)/.lock"; fi
HERE="${JC2_LOCK_DIR:-$DEFAULT_LOCK}"
mkdir -p "$HERE"
label="$1"; shift
LOCK=""
while [ -z "$LOCK" ]; do
  for slot in slot1 slot2; do
    if mkdir "$HERE/$slot" 2>/dev/null; then LOCK="$HERE/$slot"; break; fi
    if [ -f "$HERE/$slot/holder" ]; then
      age=$(( $(date +%s) - $(stat -c %Y "$HERE/$slot/holder") ))
      if [ "$age" -gt 2700 ]; then
        echo "[build-lock] breaking stale $slot (${age}s old) held by: $(cat "$HERE/$slot/holder" 2>/dev/null)"
        rm -rf "$HERE/$slot"
      fi
    fi
  done
  if [ -z "$LOCK" ]; then
    echo "[build-lock] both slots busy ($(cat "$HERE/slot1/holder" 2>/dev/null) | $(cat "$HERE/slot2/holder" 2>/dev/null)) — waiting 20s ($(date +%T))"
    sleep 20
  fi
done
echo "$label $(date +%T) pid $$" > "$LOCK/holder"
trap 'rm -rf "$LOCK"' EXIT INT TERM
# Two builds share the 12 cores. Compiler cache settings are inert until sccache is installed
# and .cargo/config.toml names it as the rustc wrapper.
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-6}"
export SCCACHE_CACHE_SIZE="${SCCACHE_CACHE_SIZE:-40G}"
export SCCACHE_DIR="${SCCACHE_DIR:-$HOME/.sccache-jc2}"
echo "[build-lock] acquired $(basename "$LOCK") by $label at $(date +%T)"
"$@"
rc=$?
echo "[build-lock] released by $label at $(date +%T) (exit $rc)"
exit $rc
