#!/usr/bin/env bash
# Usage: tools/ci/install_rust.sh [PACKAGE...]
# A verify part's setup: the stable toolchain with rustfmt and clippy and, on Blacksmith's runners (HOSTED=true), the
# Debian packages named. The two run side by side: one after the other they took ~14 s of each Rust part (2026-11-20:
# apt ~7.5 s, rustup ~6.5 s). `apt-get update` stays: the image's package lists name the libudev-dev that matches its
# own libudev1, and the mirror had already dropped that version (#18843).
set -euo pipefail
apt=
if [ "${HOSTED:-false}" = true ] && [ $# -gt 0 ]; then
  log=$(mktemp)
  { sudo apt-get update && sudo apt-get install -y --no-install-recommends "$@"; } >"$log" 2>&1 &
  apt=$!
fi
rustup toolchain install stable --profile minimal --component rustfmt --component clippy
if [ -n "$apt" ]; then
  status=0
  wait "$apt" || status=$?
  echo "::group::apt-get (exit $status)"
  cat "$log"
  echo "::endgroup::"
  exit "$status"
fi
