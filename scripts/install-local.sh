#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
key=${CLIGHT_SIGNING_KEY:-$HOME/.tauri/clight.key}
[ -f "$key" ] || { echo "Signing key not found: $key" >&2; exit 1; }

cd "$root/src-tauri"
TAURI_SIGNING_PRIVATE_KEY=$(cat "$key")
TAURI_SIGNING_PRIVATE_KEY_PASSWORD=""
NO_STRIP=true
export TAURI_SIGNING_PRIVATE_KEY TAURI_SIGNING_PRIVATE_KEY_PASSWORD NO_STRIP
cargo tauri build --bundles appimage

image=$(ls -t target/release/bundle/appimage/*.AppImage | head -n 1)
apps=$HOME/Applications
bin=$HOME/.local/bin
share=$HOME/.local/share
mkdir -p "$apps" "$bin" "$share/applications" "$share/icons/hicolor/256x256/apps"

cp "$image" "$apps/clight.AppImage.new"
chmod +x "$apps/clight.AppImage.new"
mv "$apps/clight.AppImage.new" "$apps/clight.AppImage"
rm -f "$bin/clight"
launcher="#!/bin/sh
nohup \"$apps/clight.AppImage\" \"\$@\" >/dev/null 2>&1 </dev/null &"
printf '%s\n' "$launcher" > "$bin/clight"
chmod +x "$bin/clight"
if [ ! -e "$bin/cl" ] || grep -q clight "$bin/cl"; then
    cp "$bin/clight" "$bin/cl"
fi
cp icons/128x128@2x.png "$share/icons/hicolor/256x256/apps/clight.png"

cat > "$share/applications/clight.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=clight
GenericName=C/C++ IDE
Comment=C and C++ IDE for low-end machines
Exec=$apps/clight.AppImage
Icon=clight
Terminal=false
Categories=Development;IDE;
Keywords=c;cpp;c++;cmake;ide;editor;code;
StartupWMClass=clight
EOF

update-desktop-database "$share/applications" 2>/dev/null || true
gtk-update-icon-cache -q "$share/icons/hicolor" 2>/dev/null || true
command -v kbuildsycoca6 >/dev/null && kbuildsycoca6 >/dev/null 2>&1 || true

echo "Installed $apps/clight.AppImage"
