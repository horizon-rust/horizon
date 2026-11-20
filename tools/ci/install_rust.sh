#!/usr/bin/env bash
# Usage: tools/ci/install_rust.sh [PACKAGE...]
# A verify part's setup: the stable toolchain with rustfmt and clippy and, on Blacksmith's runners (HOSTED=true), the
# Debian packages named. The two run side by side: one after the other they took ~14 s of each Rust part (2026-11-20:
# apt ~7.5 s, rustup ~6.5 s). `apt-get update` stays: the image's package lists name the libudev-dev that matches its
# own libudev1, and the mirror had already dropped that version (#18843).
# apt is bounded (APT_TIMEOUT seconds, 300 by default; the tests shorten it), so a stalled mirror can't use up the job's
# time, and always reaped: if rustup fails first, the script still waits for apt and shows what it printed.
set -euo pipefail
apt=
log=
finish_apt() {
  [ -n "$apt" ] || return 0
  local status=0
  wait "$apt" || status=$?
  apt=
  echo "::group::apt-get (exit $status)"
  cat "$log"
  echo "::endgroup::"
  return "$status"
}
if [ "${HOSTED:-false}" = true ] && [ $# -gt 0 ]; then
  log=$(mktemp)
  timeout -k 30 "${APT_TIMEOUT:-300}" bash -c \
    'sudo apt-get update && sudo apt-get install -y --no-install-recommends "$@"' apt "$@" >"$log" 2>&1 &
  apt=$!
  # rustup's failure ends the script here (set -e); the trap keeps its status.
  trap 'finish_apt || true' EXIT
fi
rustup toolchain install stable --profile minimal --component rustfmt --component clippy
trap - EXIT
finish_apt
