#!/bin/sh
while [ "$#" -gt 0 ]; do
    case "$1" in
        --resources) resources=$2; shift ;;
        --control-socket) control=$2; shift ;;
    esac
    shift
done
[ -d "$resources" ] && [ -d "$(dirname "$control")" ] && [ -f server.toml ]
