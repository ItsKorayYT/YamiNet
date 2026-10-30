#!/bin/sh
set -eu
rm -f -- "${XDG_DATA_HOME:-$HOME/.local/share}/applications/yaminet.desktop"
printf 'Launcher removed. Delete the extracted application bundle separately. User data is retained.\n'
