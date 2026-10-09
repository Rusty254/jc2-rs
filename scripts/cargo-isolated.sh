#!/usr/bin/env bash
# usage: cargo-isolated.sh <codegen-units> <cargo args...>
# Gives this worktree's workspace crates a unique metadata hash in the SHARED target dir
# (cargo keys workspace members by workspace-relative path, so worktrees otherwise collide
# and a build can come back "Fresh" with another agent's code). Dependencies stay shared.
cu="$1"; shift
sub="$1"; shift
exec cargo "$sub" \
  --config "profile.release.package.jc2-formats.codegen-units=$cu" \
  --config "profile.release.package.jc2-game.codegen-units=$cu" \
  --config "profile.release.package.jc2-tool.codegen-units=$cu" \
  "$@"
