#!/bin/sh
# Builds the ray tracing test scene (tools/rt_test_scene.txt) in a running game and captures it
# from fixed views at noon, sunset and night (docs/raytracing.md).
#
#   tools/rt_test_scene.sh CTL SOCKET OUT [LABEL]
#
# CTL is horizon-ctl, SOCKET the game's control socket, OUT a folder for the PNGs (LABEL-VIEW-TIME.png).
# The player goes to spectator mode: no hand, no gravity, the same views every run.
set -e
CTL=$1; SOCK=$2; OUT=$3; LABEL=${4:-scene}
HERE=$(dirname "$0")
mkdir -p "$OUT"
c() { "$CTL" --socket "$SOCK" "$@" > /dev/null; }
c command "gamemode spectator"
"$CTL" --socket "$SOCK" options-set chatVisibility 2 > /dev/null 2>&1 || true
c command "gamerule advance_time false"
c command "weather clear"
c command "tp @s 16 205 16"
sleep 6
grep -v '^#' "$HERE/rt_test_scene.txt" | grep -v '^$' | while IFS= read -r line; do
  c command "$line"
done
sleep 4
# view name, x y z yaw pitch
VIEWS="overview:-7,207,-7,-45,22 overhang:25,202,22,180,5 shapes:9,203,-3,0,35 tunnel:31.5,201.2,26.5,-90,5 sealed:-3.5,201.5,26.5,-90,10 glass:33,204,-1,0,35 pool:16.5,204,25.5,0,45 sky:16,203,16,0,-80 hall:-5,203,34.5,-90,25"
for t in noon:6000 sunset:12300 night:18000; do
  name=${t%%:*}; ticks=${t#*:}
  c time "$ticks"
  for v in $VIEWS; do
    view=${v%%:*}; set -- $(echo "${v#*:}" | tr ',' ' ')
    c command "tp @s $1 $2 $3 $4 $5"
    sleep 3
    c screenshot "$(cygpath -w "$OUT" 2>/dev/null || echo "$OUT")/$LABEL-$view-$name.png"
  done
done
