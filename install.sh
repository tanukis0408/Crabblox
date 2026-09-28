#!/usr/bin/env bash
# Mac O' Blox installer: Darling, the tools the launcher needs, and the
# launcher itself with its app menu entry. Run it again to update.
#
#   curl -fsSL https://raw.githubusercontent.com/tanukis0408/Crabblox/main/install.sh | bash
#
# Everything runs inside main(), called on the last line, so a download cut
# off halfway does nothing.

set -euo pipefail

REPO=https://github.com/tanukis0408/Crabblox.git
DIR=${XDG_DATA_HOME:-$HOME/.local/share}/Crabblox

say() { printf '\033[1;35m==>\033[0m %s\n' "$*"; }
die() { printf '\033[1;31mError:\033[0m %s\n' "$*" >&2; exit 1; }

install_arch() {
  # Only packages that are not installed at all: asking pacman for an
  # installed but outdated one (pipewire-audio 1.6.8 with 1.6.9 in the repo)
  # makes it a partial upgrade that breaks on pinned dependencies.
  local wanted=(git base-devel clang lld unzip python python-gobject gtk4 libadwaita)
  command -v pw-cat >/dev/null || wanted+=(pipewire-audio)
  local missing
  missing=$(pacman -T "${wanted[@]}" || true)
  if [[ -n $missing ]]; then
    say "Installing tools (pacman): $(echo $missing)"
    # shellcheck disable=SC2086
    sudo pacman -S --needed --noconfirm $missing ||
      die "pacman could not install them. Update the system with 'sudo pacman -Syu' and run this again."
  fi
  command -v darling >/dev/null && return
  say "Installing Darling from the AUR (darling-bin)"
  if command -v paru >/dev/null; then
    paru -S --needed --noconfirm --skipreview darling-bin
  elif command -v yay >/dev/null; then
    yay -S --needed --noconfirm --answerdiff None --answerclean None darling-bin
  else
    local build
    build=$(mktemp -d)
    git clone --depth 1 https://aur.archlinux.org/darling-bin.git "$build/darling-bin"
    (cd "$build/darling-bin" && makepkg -si --noconfirm)
    rm -rf "$build"
  fi
}

install_debian() {
  say "Installing tools (apt)"
  sudo apt-get update
  sudo apt-get install -y git curl unzip clang lld pipewire-bin python3 python3-gi \
    gir1.2-gtk-4.0 gir1.2-adw-1
  command -v darling >/dev/null && return
  # The release page redirects to the newest tag, v0.1.YYYYMMDD; its Debian
  # packages (built for Ubuntu 24.04) come as debs_YYYYMMDD.zip.
  local tag build
  tag=$(curl -fsSLI -o /dev/null -w '%{url_effective}' https://github.com/darlinghq/darling/releases/latest)
  tag=${tag##*/}
  say "Installing Darling $tag"
  build=$(mktemp -d)
  curl -fL --progress-bar -o "$build/debs.zip" \
    "https://github.com/darlinghq/darling/releases/download/$tag/debs_${tag##*.}.zip"
  unzip -q "$build/debs.zip" -d "$build"
  sudo apt-get install -y "$build"/debs_*/*.deb
  rm -rf "$build"
}

install_fedora() {
  say "Installing tools (dnf)"
  sudo dnf install -y git clang lld unzip pipewire-utils python3-gobject gtk4 libadwaita
}

main() {
  # Package managers may ask questions; with curl | bash stdin is this script.
  if [[ ! -t 0 ]] && (: </dev/tty) 2>/dev/null; then exec </dev/tty; fi
  [[ $(uname -m) == x86_64 ]] || die "Darling runs only on x86_64."
  [[ $EUID -ne 0 ]] || die "Run this as your user, not root. sudo is used when needed."
  [[ -r /etc/os-release ]] && . /etc/os-release
  local family=" ${ID:-} ${ID_LIKE:-} "
  case "$family" in
    *" arch "*) install_arch ;;
    *" debian "* | *" ubuntu "*) install_debian ;;
    *" fedora "*) install_fedora ;;
    *)
      # Derivatives that do not say what they are based on (LeagueArchy has
      # no ID_LIKE): go by the package manager.
      if command -v pacman >/dev/null; then install_arch
      elif command -v apt-get >/dev/null; then install_debian
      elif command -v dnf >/dev/null; then install_fedora
      else say "Unknown distribution, install Darling, clang, lld, unzip, PipeWire, PyGObject, GTK 4 and libadwaita yourself"
      fi ;;
  esac
  command -v unzip >/dev/null || die "unzip is not installed. Install unzip with your package manager and run this again."
  command -v darling >/dev/null ||
    die "Darling is not installed. Build it with https://docs.darlinghq.org/build-instructions.html and run this again."

  if [[ -d $DIR/.git ]]; then
    say "Updating Crabblox"
    git -C "$DIR" pull --ff-only
  else
    say "Downloading Crabblox"
    git clone --depth 1 "$REPO" "$DIR"
  fi

  if [[ -f "$DIR/prebuilt/libMacOBloxShims.dylib" && ! -f "$DIR/build/libMacOBloxShims.dylib" ]]; then
    mkdir -p "$DIR/build/frameworks"
    cp -a "$DIR/prebuilt/libMacOBloxShims.dylib" "$DIR/build/" || true
    cp -a "$DIR/prebuilt/frameworks"/* "$DIR/build/frameworks/" || true
  fi

  if command -v clang >/dev/null && command -v ld.lld >/dev/null; then
    say "Verifying/building the Roblox shim from source"
    "$DIR/build_debug_shim.sh" || true
  else
    say "Using prebuilt Roblox shims and frameworks"
  fi
  if command -v cargo >/dev/null && [[ -d "$DIR/launcher-rs" ]]; then
    say "Building Crabblox Rust launcher (cargo)"
    (cd "$DIR" && cargo build --release --manifest-path launcher-rs/Cargo.toml && cp launcher-rs/target/release/crabblox "$DIR/crabblox") || true
  fi
  "$DIR/launcher/install.sh"
  local bin1="$DIR/RobloxPlayer.app/Contents/MacOS/RobloxPlayer"
  local bin2="${XDG_DATA_HOME:-$HOME/.local/share}/Crabblox/RobloxPlayer.app/Contents/MacOS/RobloxPlayer"
  if [[ ! -f "$bin1" && ! -f "$bin2" ]]; then
    local crabblox_bin="$HOME/.local/bin/crabblox"
    [[ -x "$crabblox_bin" ]] || crabblox_bin="$DIR/crabblox"
    if [[ -x "$crabblox_bin" ]]; then
      say "Downloading official macOS Roblox client..."
      if ! "$crabblox_bin" update; then
        say "CLI update failed. You can launch Crabblox and click 'Установить Roblox' in the UI."
      fi
    fi
  fi
  say "Done. Open Crabblox from the app menu or run 'crabblox'!"
}

main "$@"
