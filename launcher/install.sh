#!/usr/bin/env bash
# Installs the Crabblox launcher for the current user: menu entry, icons
# and a `crabblox` command. Run again after moving the project folder.
set -euo pipefail
launcher_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
project_dir=$(dirname -- "$launcher_dir")
data_home=${XDG_DATA_HOME:-$HOME/.local/share}

for size in 16 22 24 32 48 64 128 256 512; do
  install -Dm644 "$project_dir/branding/icons/crabblox-$size.png" \
    "$data_home/icons/hicolor/${size}x${size}/apps/crabblox.png"
  install -Dm644 "$project_dir/branding/icons/crabblox-$size.png" \
    "$data_home/icons/hicolor/${size}x${size}/apps/macoblox.png"
done

install -d "$HOME/.local/bin"
if [[ -f "$project_dir/crabblox" ]]; then
  install -m 755 "$project_dir/crabblox" "$HOME/.local/bin/crabblox"
else
  ln -sf "$launcher_dir/macoblox-launcher" "$HOME/.local/bin/crabblox"
fi
ln -sf "$HOME/.local/bin/crabblox" "$HOME/.local/bin/macoblox"

install -d "$data_home/applications"
cat > "$data_home/applications/crabblox.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=Crabblox
Comment=Run Roblox on Linux through Darling (by Monster Dev)
Comment[ru]=Запуск клиента Roblox для macOS на Linux через Darling (Monster Dev)
GenericName=Roblox launcher
GenericName[ru]=Лаунчер Roblox
Exec=$HOME/.local/bin/crabblox %u
Icon=crabblox
Terminal=false
Categories=Game;
Keywords=roblox;darling;crabblox;
MimeType=x-scheme-handler/roblox-player;
StartupNotify=true
DESKTOP

cat > "$data_home/applications/xyz.narez.MacOBlox.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=Crabblox
Comment=Run Roblox on Linux through Darling (by Monster Dev)
Comment[ru]=Запуск клиента Roblox для macOS на Linux через Darling (Monster Dev)
GenericName=Roblox launcher
GenericName[ru]=Лаунчер Roblox
Exec=$HOME/.local/bin/crabblox %u
Icon=crabblox
Terminal=false
Categories=Game;
Keywords=roblox;darling;crabblox;
MimeType=x-scheme-handler/roblox-player;
StartupNotify=true
DESKTOP

# Roblox Studio (Windows version through Wine), also the handler of the
# roblox-studio: links and of roblox-studio-auth: that signs Studio in.
cat > "$data_home/applications/xyz.narez.MacOBlox.Studio.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=Roblox Studio (Crabblox)
Comment=Roblox Studio through Wine
Comment[ru]=Roblox Studio через Wine
Exec=$launcher_dir/macoblox-launcher --studio %u
Icon=crabblox
Terminal=false
Categories=Development;
MimeType=x-scheme-handler/roblox-studio;x-scheme-handler/roblox-studio-auth;application/x-roblox-place;
StartupWMClass=robloxstudiobeta.exe
DESKTOP
if command -v xdg-mime >/dev/null; then
  xdg-mime default crabblox.desktop x-scheme-handler/roblox-player || true
  for type in x-scheme-handler/roblox-studio x-scheme-handler/roblox-studio-auth; do
    xdg-mime default xyz.narez.MacOBlox.Studio.desktop "$type" || true
  done
fi

# The game window itself (X11 class RobloxPlayer) gets the same icon in docks.
cat > "$data_home/applications/macoblox-roblox-window.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=Roblox (Crabblox)
Exec=$HOME/.local/bin/crabblox
Icon=crabblox
NoDisplay=true
StartupWMClass=RobloxPlayer
DESKTOP

update-desktop-database "$data_home/applications" 2>/dev/null || true
gtk-update-icon-cache -q -t "$data_home/icons/hicolor" 2>/dev/null || true
echo "Crabblox (by Monster Dev) installed: find it in the app menu or run: crabblox"
