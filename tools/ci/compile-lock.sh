#!/usr/bin/env bash
# compile-lock.sh <role> -- <command>...: the command in a turn on the Mac mini (tools/compile_lock.py), for
# game-build.yml's Mac compiles. Its build job compiles before runner-env.sh puts Homebrew's Python on PATH, so this
# takes whichever python3 there is (/usr/bin/python3 then). A Python that can't run the module (none, Apple's stub
# without the command line tools) never keeps the command from running: it then runs without the lock.
role=$1
shift
[ "${1:-}" = -- ] && shift
script="${0%/*}/../compile_lock.py"
for python in python3.13 python3; do
  if command -v "$python" > /dev/null 2>&1 && "$python" "$script" --probe > /dev/null 2>&1; then
    exec "$python" "$script" "$role" -- "$@"
  fi
done
echo "  compile lock: no Python runs tools/compile_lock.py here; compiling without it"
exec "$@"
