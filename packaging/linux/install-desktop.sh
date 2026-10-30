#!/bin/sh
# Register this extracted bundle with the current desktop; never copy/delete user data.
set -eu
app_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
applications="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
mkdir -p "$applications"
# Desktop Entry quoted Exec syntax; escape reserved characters, including percent.
escaped=$(printf '%s' "$app_dir/launch-browser.sh" | sed 's/\\/\\\\/g; s/"/\\"/g; s/`/\\`/g; s/\$/\\$/g; s/%/%%/g')
cat > "$applications/yaminet.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=Yami Browser (experimental)
Exec="$escaped"
Terminal=false
Categories=Network;WebBrowser;
Comment=Browse experimental identity-authenticated YLang sites
DESKTOP
printf 'Launcher installed. Keep this bundle at its current location.\n'
