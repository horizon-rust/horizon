#!/bin/sh
# Builds the rain test scene (tools/rain_test_scene.txt) in a running game and captures it: dry,
# under the rain by day and by night, the wetness rising and the world drying (docs/raytracing.md,
# "Rain and wet ground"). Start the game with HORIZON_WET_SPEED (say 10) for the time lapses.
#
#   tools/rain_test_scene.sh CTL SOCKET OUT [LABEL] [STEPS]
#
# CTL is horizon-ctl, SOCKET the game's control socket, OUT a folder for the PNGs
# (LABEL-VIEW-WHEN.png); STEPS "all" (default), "views" (dry and rain, day and night) or "lapse".
set -e
CTL=$1; SOCK=$2; OUT=$3; LABEL=${4:-rain}; STEPS=${5:-all}
HERE=$(dirname "$0")
mkdir -p "$OUT"
c() { "$CTL" --socket "$SOCK" "$@" > /dev/null; }
shot() { c screenshot "$(cygpath -w "$OUT" 2>/dev/null || echo "$OUT")/$LABEL-$1.png"; }
c command "gamemode spectator"
"$CTL" --socket "$SOCK" options-set chatVisibility 2 > /dev/null 2>&1 || true
c command "gamerule advance_time false"
c command "gamerule advance_weather false"
c command "weather clear"
c time 6000
c command "tp @s 16 205 16"
sleep 6
grep -v '^#' "$HERE/rain_test_scene.txt" | grep -v '^$' | while IFS= read -r line; do
  c command "$line"
done
sleep 4
# view name, x y z yaw pitch
VIEWS="street:-7,202,12.5,-90,12 roof:6,203.5,6,-55,28 path:3.5,203,12,180,18 forest:3.5,202,1,180,8 pond:28,203.5,10,180,38"
views_at() {
  for v in $VIEWS; do
    view=${v%%:*}; set -- $(echo "${v#*:}" | tr ',' ' ')
    c command "tp @s $1 $2 $3 $4 $5"
    sleep 3
    shot "$view-$WHEN"
  done
}
if [ "$STEPS" = all ] || [ "$STEPS" = views ]; then
  # Whatever rain fell before has dried (HORIZON_WET_SPEED 60 dries in seconds).
  sleep 25
  WHEN=dry-noon; views_at
  c time 18000; WHEN=dry-night; views_at
  c time 6000
  c command "weather rain 1000000"
  # Soaked and the puddles grown (HORIZON_WET_SPEED speeds it up).
  sleep 40
  WHEN=rain-noon; views_at
  c time 18000; WHEN=rain-night; views_at
  c time 6000
fi
if [ "$STEPS" = all ] || [ "$STEPS" = lapse ]; then
  c command "weather clear"
  c command "tp @s 6 203.5 6 -55 28"
  sleep 60
  c command "weather rain 1000000"
  # A shot every 6 s while it rains, then every 15 s while it dries (real seconds: times
  # HORIZON_WET_SPEED for the world's).
  for i in 0 1 2 3 4 5 6 7 8 9; do
    shot "lapse-wet-$(printf %03d $((i * 6)))s"
    sleep 6
  done
  c command "weather clear"
  for i in 0 1 2 3 4 5 6 7 8 9; do
    shot "lapse-dry-$(printf %03d $((i * 15)))s"
    sleep 15
  done
fi
