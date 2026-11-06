#!/usr/bin/env bash
# The mobs' region simulation, which an ordinary harness run never reaches: automatic mode needs 128 mobs in two
# regions, and the harnesses' scenes are smaller. So the harnesses that tick mobs in the real server run here with
# every commit replayed and compared (HORIZON_SIMULATION_MODE=verify, docs/server-regions.md). mob_ai isn't among them:
# its probe has a mob loop of its own. Then the tests that need the vanilla data, which `checks` runs without.
# Run it after the ordinary harnesses: a pass here records the harness as passed too.
set -euo pipefail
version=$(sed -n 's/.*"version": *"\([^"]*\)".*/\1/p' research-lock.json | head -1)
HORIZON_SIMULATION_MODE=verify PARITY_HOSTS='' python3 tools/parity.py \
  dedicated_server server_entities server_player_damage natural_spawn --timeout 2400
export HORIZON_TEST_RESOURCES="$PWD/.local/horizon-client/$version" HORIZON_REQUIRE_TEST_RESOURCES=1
export CARGO_TARGET_DIR="$PWD/.local/parity-target"
cargo test --locked -p horizon-client-lib --lib mobs::region_tick
cargo test --locked -p horizon-client --lib -- --include-ignored \
  pool_and_serial entity_hurt_player_reads_riders an_effect_hit_on_a_guest clean_commit
