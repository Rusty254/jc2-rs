#!/usr/bin/env bash
# Serialises merges into the main checkout (single slot; builds use with-build-lock.sh).
# usage: with-merge-lock.sh <label> <command> [args...]
# Waits until the lock is free, holds it for the command, releases on exit.
# A lock older than 45 minutes is assumed abandoned and is broken.
# One lock for the main checkout and every git worktree: resolve the shared .git dir.
COMMON="$(git -C "$(dirname "$0")" rev-parse --path-format=absolute --git-common-dir 2>/dev/null)"
if [ -n "$COMMON" ]; then DEFAULT_LOCK="$(dirname "$COMMON")/scripts/.lock"
else DEFAULT_LOCK="$(cd "$(dirname "$0")" && pwd)/.lock"; fi
HERE="${JC2_LOCK_DIR:-$DEFAULT_LOCK}"
LOCK="$HERE/merge"; mkdir -p "$HERE"
label="$1"; shift
while ! mkdir "$LOCK" 2>/dev/null; do
  holder="$(cat "$LOCK/holder" 2>/dev/null)"
  if [ -f "$LOCK/holder" ]; then
    age=$(( $(date +%s) - $(stat -c %Y "$LOCK/holder") ))
    if [ "$age" -gt 2700 ]; then
      echo "[merge-lock] breaking stale lock (${age}s old) held by: $holder"
      rm -rf "$LOCK"; continue
    fi
  fi
  echo "[merge-lock] held by: ${holder:-?} — waiting 10s ($(date +%T))"
  sleep 10
done
echo "$label $(date +%T) pid $$" > "$LOCK/holder"
trap 'rm -rf "$LOCK"' EXIT INT TERM
echo "[merge-lock] acquired by $label at $(date +%T)"
"$@"
rc=$?
echo "[merge-lock] released by $label at $(date +%T) (exit $rc)"
exit $rc
