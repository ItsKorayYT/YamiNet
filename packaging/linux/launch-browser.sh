#!/bin/sh
set -eu
app_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
exec "$app_dir/yami-browser" --desktop "$@"
