#!/usr/bin/env bash
# Serialises cargo builds across agents sharing one target dir.
# usage: with-build-lock.sh <label> <command> [args...]
# Waits until the lock is free, holds it for the command, releases on exit.
# A lock older than 45 minutes is assumed abandoned and is broken.
# One lock for the main checkout and every git worktree: resolve the shared .git dir, so a
# worktree copy of this script does not lock only itself.
COMMON="$(git -C "$(dirname "$0")" rev-parse --path-format=absolute --git-common-dir 2>/dev/null)"
if [ -n "$COMMON" ]; then DEFAULT_LOCK="$(dirname "$COMMON")/scripts/.lock"
else DEFAULT_LOCK="$(cd "$(dirname "$0")" && pwd)/.lock"; fi
HERE="${JC2_LOCK_DIR:-$DEFAULT_LOCK}"
LOCK="$HERE/lock"; mkdir -p "$HERE"
label="$1"; shift
while ! mkdir "$LOCK" 2>/dev/null; do
  holder="$(cat "$LOCK/holder" 2>/dev/null)"
  if [ -f "$LOCK/holder" ]; then
    age=$(( $(date +%s) - $(stat -c %Y "$LOCK/holder") ))
    if [ "$age" -gt 2700 ]; then
      echo "[build-lock] breaking stale lock (${age}s old) held by: $holder"
      rm -rf "$LOCK"; continue
    fi
  fi
  echo "[build-lock] held by: ${holder:-?} — waiting 20s ($(date +%T))"
  sleep 20
done
echo "$label $(date +%T) pid $$" > "$LOCK/holder"
trap 'rm -rf "$LOCK"' EXIT INT TERM
echo "[build-lock] acquired by $label at $(date +%T)"
"$@"
rc=$?
echo "[build-lock] released by $label at $(date +%T) (exit $rc)"
exit $rc
