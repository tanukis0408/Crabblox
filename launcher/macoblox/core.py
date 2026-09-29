"""Backend of the Mac O’ Blox launcher: paths, settings, fast flags, Roblox
updates and running the macOS client through Darling. No GTK here."""

import datetime
import json
import os
import plistlib
import re
import shutil
import signal
import struct
import subprocess
import tempfile
import time
import urllib.request
import zipfile
from pathlib import Path

from . import __version__
from .i18n import _

PROJECT = Path(__file__).resolve().parents[2]
# Everything the launcher writes: the project folder for a git checkout, the
# user's data folder when the sources are installed read-only (a package).
DATA_DIR = (PROJECT if os.access(PROJECT, os.W_OK) else
            Path(os.environ.get("XDG_DATA_HOME", Path.home() / ".local" / "share")) / "macoblox")
APP_BUNDLE = DATA_DIR / "RobloxPlayer.app"
BUILD_DIR = DATA_DIR / "build"
# A package may ship the shim built already (the Flatpak has no compiler).
PREBUILT_SHIM = os.environ.get("MACOBLOX_PREBUILT_SHIM")
SHIM_DIR = Path(PREBUILT_SHIM) if PREBUILT_SHIM else BUILD_DIR
SHIM = SHIM_DIR / "libMacOBloxShims.dylib"
# Frameworks RobloxPlayer links that Darling lacks; stubs from frameworks/.
FRAMEWORKS = ["CoreML", "CoreHaptics", "DeviceCheck"]
FRAMEWORKS_BUILD = SHIM_DIR / "frameworks"
# Packages install Darling's macOS root to /usr/libexec/darling, a build from
# source to /usr/local/libexec/darling, the Flatpak to /app/libexec/darling.
DARLING_SYSROOT = next((path for path in (Path("/usr/libexec/darling"), Path("/usr/local/libexec/darling"),
                                          Path("/app/libexec/darling"))
                        if path.is_dir()), Path("/usr/libexec/darling"))
DARLING_PREFIX = Path(os.environ.get("DPREFIX") or Path.home() / ".darling")
# Rootless Darling (for sandboxes such as Flatpak, see flatpak/darling-noroot.c):
# this library is preloaded into `darling` only, never into the launcher.
NOROOT_LIB = os.environ.get("MACOBLOX_NOROOT_LIB")
# Darling's bridges to host libraries that Roblox never uses, relative to the
# macOS root: when the host lacks the library, they are patched to load nothing.
NATIVE_LIBS = [f"usr/lib/native/{name}.dylib"
               for name in ("libavcodec", "libavformat", "libavutil", "libswresample", "libjpeg", "libfuse")]
NATIVE_LIBS.append("System/Library/Frameworks/OpenGL.framework/Versions/A/Libraries/libGLU.dylib")
NATIVE_BUILD = BUILD_DIR / "native"
BUILD_SCRIPT = PROJECT / "build_debug_shim.sh"
LOGS = DATA_DIR / "logs"
BACKUPS = DATA_DIR / "backups"
DOWNLOADS = DATA_DIR / "downloads"
ICONS = PROJECT / "branding" / "icons"
FAST_FLAGS = APP_BUNDLE / "Contents" / "MacOS" / "ClientSettings" / "ClientAppSettings.json"

CONFIG_DIR = Path(os.environ.get("XDG_CONFIG_HOME", Path.home() / ".config")) / "macoblox"
CACHE_DIR = Path(os.environ.get("XDG_CACHE_HOME", Path.home() / ".cache")) / "macoblox"
SETTINGS_FILE = CONFIG_DIR / "settings.json"

DARLING_HOME = DARLING_PREFIX / "Users" / os.environ.get("USER", "user")
SESSION_FILES = [
    DARLING_HOME / "Library" / "MacOBlox" / "Cookies.plist",
    DARLING_HOME / "Library" / "MacOBlox" / "Keychain",
]
USER_CACHE_FILE = CACHE_DIR / "user.json"

VERSION_URL = "https://clientsettingscdn.roblox.com/v2/client-version/MacPlayer"
DOWNLOAD_URL = "https://setup.rbxcdn.com/mac/{upload}-RobloxPlayer.zip"

DEFAULT_SETTINGS = {
    "language": "en",
    "mouse_sensitivity": 1.0,
    "hide_menu_bar": False,
    "dns": "system",
    "dns_custom": "",
    "show_launcher_after_exit": True,
    "diagnostic_signals": False,
    "trace_udp": False,
    "trace_lock": False,
    "trace_events": False,
    "trace_gl": False,
    "fps_log": False,
    "trace_keys": False,
    "keep_logs": 30,
}

# Settings -> environment variables understood by the shim.
TRACE_ENV = {
    "diagnostic_signals": "MACOBLOX_DIAGNOSTIC_SIGNALS",
    "trace_udp": "MACOBLOX_TRACE_UDP",
    "trace_lock": "MACOBLOX_TRACE_LOCK",
    "trace_events": "MACOBLOX_TRACE_EVENTS",
    "trace_gl": "MACOBLOX_TRACE_GL",
    "fps_log": "MACOBLOX_FPS_LOG",
    "trace_keys": "MACOBLOX_TRACE_KEYS",
}


def load_settings():
    settings = dict(DEFAULT_SETTINGS)
    try:
        settings.update(json.loads(SETTINGS_FILE.read_text()))
    except (OSError, ValueError):
        pass
    return settings


def save_settings(settings):
    CONFIG_DIR.mkdir(parents=True, exist_ok=True)
    SETTINGS_FILE.write_text(json.dumps(settings, indent=2, ensure_ascii=False))


# ---------------------------------------------------------------- fast flags

def load_fast_flags():
    try:
        data = json.loads(FAST_FLAGS.read_text())
        return data if isinstance(data, dict) else {}
    except (OSError, ValueError):
        return {}


def save_fast_flags(flags):
    FAST_FLAGS.parent.mkdir(parents=True, exist_ok=True)
    FAST_FLAGS.write_text(json.dumps(flags, indent=2, ensure_ascii=False))


def parse_flag_value(text):
    """Turn what the user typed into the JSON value Roblox expects."""
    stripped = text.strip()
    lowered = stripped.lower()
    if lowered in ("true", "false"):
        return lowered == "true"
    try:
        return int(stripped)
    except ValueError:
        return stripped


def format_flag_value(value):
    if isinstance(value, bool):
        return "true" if value else "false"
    return str(value)


# ------------------------------------------------------------------ versions

def installed_version():
    try:
        with open(APP_BUNDLE / "Contents" / "Info.plist", "rb") as file:
            return plistlib.load(file).get("CFBundleShortVersionString")
    except (OSError, plistlib.InvalidFileException):
        return None


def latest_version():
    """Returns (version, clientVersionUpload) from Roblox's version service."""
    request = urllib.request.Request(VERSION_URL, headers={"User-Agent": "MacOBlox"})
    with urllib.request.urlopen(request, timeout=15) as response:
        data = json.load(response)
    return data["version"], data["clientVersionUpload"]


def update_roblox(upload, progress=None):
    """Download the official macOS client and swap it in, keeping fast flags.
    The previous bundle is moved to backups/. progress(fraction, text)."""
    DOWNLOADS.mkdir(parents=True, exist_ok=True)
    archive = DOWNLOADS / f"{upload}-RobloxPlayer.zip"
    part = archive.with_suffix(".zip.part")

    # Only download if archive is missing or invalid
    need_download = True
    if archive.exists():
        try:
            with zipfile.ZipFile(archive) as z:
                if z.testzip() is None:
                    need_download = False
        except Exception:
            archive.unlink(missing_ok=True)

    if need_download:
        url = DOWNLOAD_URL.format(upload=upload)
        for attempt in range(4):
            try:
                headers = {"User-Agent": "MacOBlox"}
                offset = part.stat().st_size if part.exists() else 0
                if offset > 0:
                    headers["Range"] = f"bytes={offset}-"
                request = urllib.request.Request(url, headers=headers)
                with urllib.request.urlopen(request, timeout=30) as response:
                    is_partial = getattr(response, "status", 200) == 206
                    mode = "ab" if is_partial and offset > 0 else "wb"
                    if not is_partial:
                        offset = 0
                    total = int(response.headers.get("Content-Length") or 0) + offset
                    done = offset
                    with open(part, mode) as out:
                        while chunk := response.read(1 << 16):
                            out.write(chunk)
                            done += len(chunk)
                            if progress and total:
                                progress(done / total * 0.9, _("Downloading {done} of {total} MB",
                                                                  done=done >> 20, total=total >> 20))
                if total and done < total:
                    raise RuntimeError(f"Download incomplete: {done}/{total} bytes")
                if not zipfile.is_zipfile(part):
                    part.unlink(missing_ok=True)
                    raise RuntimeError(_("The download is not a zip archive"))
                part.rename(archive)
                break
            except Exception:
                if attempt == 3:
                    part.unlink(missing_ok=True)
                    raise
                time.sleep(2)

    if progress:
        progress(0.92, _("Unpacking"))
    unpack = Path(tempfile.mkdtemp(prefix="unpack-", dir=DOWNLOADS))
    try:
        # unzip keeps executable bits
        res = subprocess.run(["unzip", "-q", "-o", str(archive), "-d", str(unpack)], capture_output=True)
        new_bundle = unpack / "RobloxPlayer.app"
        binary = new_bundle / "Contents" / "MacOS" / "RobloxPlayer"
        if res.returncode not in (0, 1) or not binary.exists():
            # Fallback to python zipfile extraction preserving permissions
            with zipfile.ZipFile(archive) as z:
                for info in z.infolist():
                    z.extract(info, unpack)
                    mode = info.external_attr >> 16
                    if mode:
                        try:
                            (unpack / info.filename).chmod(mode)
                        except OSError:
                            pass

        if not new_bundle.is_dir():
            raise RuntimeError(_("The archive has no RobloxPlayer.app"))
        flags = load_fast_flags()
        old_version = installed_version() or "unknown"
        BACKUPS.mkdir(parents=True, exist_ok=True)
        backup = BACKUPS / f"RobloxPlayer-{old_version}.app"
        if backup.exists():
            shutil.rmtree(backup)
        if APP_BUNDLE.exists():
            APP_BUNDLE.rename(backup)
        new_bundle.rename(APP_BUNDLE)

        # Rotate old backups: keep at most 2 latest backups
        try:
            existing_backups = sorted(BACKUPS.glob("RobloxPlayer-*.app"), key=lambda p: p.stat().st_mtime, reverse=True)
            for old_b in existing_backups[2:]:
                shutil.rmtree(old_b, ignore_errors=True)
        except Exception:
            pass

        if flags:
            save_fast_flags(flags)
        if progress:
            progress(1.0, _("Done"))
        return backup
    finally:
        shutil.rmtree(unpack, ignore_errors=True)


# ----------------------------------------------------------------- processes

def _user_processes():
    uid = os.getuid()
    for entry in Path("/proc").iterdir():
        if not entry.name.isdigit():
            continue
        try:
            if entry.stat().st_uid != uid:
                continue
            args = (entry / "cmdline").read_bytes().replace(b"\0", b" ").decode(errors="replace")
        except OSError:
            continue
        yield int(entry.name), args


def roblox_pids():
    """Host PIDs of running RobloxPlayer / RobloxCrashHandler processes."""
    pids = []
    for pid, args in _user_processes():
        if args.startswith("darling shell"):
            continue
        if "RobloxPlayer" in args.split(" ", 1)[0] or "RobloxCrashHandler" in args:
            pids.append(pid)
    return pids


def _darlingservers():
    """darlingserver processes of our prefix."""
    return [pid for pid, args in _user_processes()
            if args.startswith(f"darlingserver {DARLING_PREFIX} ")]


def darlingserver_running():
    return bool(_darlingservers())


def stop_roblox():
    pids = roblox_pids()
    for pid in pids:
        try:
            os.kill(pid, signal.SIGTERM)
        except OSError:
            pass
    deadline = time.time() + 3
    while time.time() < deadline and roblox_pids():
        time.sleep(0.2)
    for pid in roblox_pids():
        try:
            os.kill(pid, signal.SIGKILL)
        except OSError:
            pass


def _darling_path(path):
    """Path of a file under the Darling prefix as seen inside the container."""
    return "/" + str(path.relative_to(DARLING_PREFIX))


def signed_in():
    """Whether a Roblox login is saved (only the cookie's name is read)."""
    try:
        with open(SESSION_FILES[0], "rb") as file:
            cookies = plistlib.load(file)
    except (OSError, plistlib.InvalidFileException, ValueError):
        return False
    return any(isinstance(c, dict) and c.get("Name") == ".ROBLOSECURITY" and c.get("Value")
               for c in cookies if isinstance(cookies, list))


def signed_in_user():
    """Returns the username of the signed-in user from cache, or None."""
    if not signed_in():
        return None
    try:
        data = json.loads(USER_CACHE_FILE.read_text())
        return data.get("name") or data.get("displayName")
    except (OSError, ValueError):
        return None


def save_session_cookie(cookie_value):
    """Save a Roblox .ROBLOSECURITY cookie to Darling's Cookies.plist after validating it."""
    cookie_value = cookie_value.strip().strip('"').strip("'")
    if ".ROBLOSECURITY=" in cookie_value:
        cookie_value = cookie_value.split(".ROBLOSECURITY=", 1)[1].split(";", 1)[0].strip()

    if not cookie_value:
        raise ValueError(_("Cookie is empty"))

    # Verify cookie with Roblox API
    req = urllib.request.Request(
        "https://users.roblox.com/v1/users/authenticated",
        headers={"Cookie": f".ROBLOSECURITY={cookie_value}", "User-Agent": "MacOBlox/Linux"}
    )
    try:
        with urllib.request.urlopen(req, timeout=10) as resp:
            user_data = json.loads(resp.read().decode())
    except urllib.error.HTTPError as err:
        if err.code == 401:
            raise ValueError(_("Invalid cookie: Roblox rejected the authentication token."))
        raise RuntimeError(_("Roblox API returned error {code}", code=err.code))
    except Exception as err:
        raise RuntimeError(_("Could not connect to Roblox: {err}", err=str(err)))

    username = user_data.get("name") or user_data.get("displayName") or "Roblox User"
    user_id = user_data.get("id")

    CACHE_DIR.mkdir(parents=True, exist_ok=True)
    try:
        USER_CACHE_FILE.write_text(json.dumps({"name": username, "id": user_id}, indent=2))
    except OSError:
        pass

    if darlingserver_running():
        restart_darling()

    cookie_file = SESSION_FILES[0]
    cookie_file.parent.mkdir(parents=True, exist_ok=True)

    cookies = []
    if cookie_file.exists():
        try:
            with open(cookie_file, "rb") as f:
                cookies = plistlib.load(f)
        except Exception:
            cookies = []
        if not isinstance(cookies, list):
            cookies = []

    cookies = [c for c in cookies if isinstance(c, dict) and c.get("Name") != ".ROBLOSECURITY"]
    expires = datetime.datetime.now(datetime.timezone.utc) + datetime.timedelta(days=3650)
    cookies.append({
        "Domain": ".roblox.com",
        "Path": "/",
        "Name": ".ROBLOSECURITY",
        "Value": cookie_value,
        "Expires": expires,
        "Secure": True,
    })

    with open(cookie_file, "wb") as f:
        plistlib.dump(cookies, f)
    try:
        cookie_file.chmod(0o600)
    except OSError:
        pass

    return username


def import_browser_cookies():
    """Look for an active Roblox session (.ROBLOSECURITY) in installed browsers
    (Chrome, Chromium, Brave, Vivaldi, Edge, Firefox, Zen, Floorp, LibreWolf, etc.).
    If found and valid, saves it to Cookies.plist and returns (username, browser_name).
    Otherwise returns (None, None).
    """
    import glob
    import hashlib
    import sqlite3

    # 1. Chromium-based browsers (native, Flatpak, Snap)
    chromium_browsers = [
        ("Google Chrome", "~/.config/google-chrome"),
        ("Chromium", "~/.config/chromium"),
        ("Brave", "~/.config/BraveSoftware/Brave-Browser"),
        ("Vivaldi", "~/.config/vivaldi"),
        ("Microsoft Edge", "~/.config/microsoft-edge"),
        ("Opera", "~/.config/opera"),
        ("Opera GX", "~/.config/opera-gx"),
        ("Yandex Browser", "~/.config/yandex-browser"),
        ("Thorium", "~/.config/thorium"),
        ("Chromium (Snap)", "~/snap/chromium/common/chromium"),
        ("Chromium (Snap Alt)", "~/snap/chromium/current/.config/chromium"),
        ("Chrome (Flatpak)", "~/.var/app/com.google.Chrome/config/google-chrome"),
        ("Chromium (Flatpak)", "~/.var/app/org.chromium.Chromium/config/chromium"),
        ("Brave (Flatpak)", "~/.var/app/com.brave.Browser/config/BraveSoftware/Brave-Browser"),
        ("Edge (Flatpak)", "~/.var/app/com.microsoft.Edge/config/microsoft-edge"),
        ("Vivaldi (Flatpak)", "~/.var/app/com.vivaldi.Vivaldi/config/vivaldi"),
        ("Opera (Flatpak)", "~/.var/app/com.opera.Opera/config/opera"),
    ]

    passwords = [b"peanuts", b""]
    for app in ["chrome", "google-chrome", "chromium", "brave", "microsoft-edge", "edge", "opera", "vivaldi", "yandex-browser", "thorium"]:
        try:
            p = subprocess.run(["secret-tool", "lookup", "application", app], capture_output=True, text=False, timeout=0.6)
            if p.returncode == 0 and p.stdout:
                pwd = p.stdout.strip()
                if pwd and pwd not in passwords:
                    passwords.insert(0, pwd)
        except Exception:
            pass

    for bname, base_dir in chromium_browsers:
        patterns = [
            f"{base_dir}/Default/Cookies",
            f"{base_dir}/Default/Network/Cookies",
            f"{base_dir}/Profile */Cookies",
            f"{base_dir}/Profile */Network/Cookies",
            f"{base_dir}/Cookies",
            f"{base_dir}/Network/Cookies",
        ]
        for pat in patterns:
            for db_path in glob.glob(os.path.expanduser(pat)):
                if not os.path.exists(db_path):
                    continue
                temp_dir = tempfile.mkdtemp(prefix="crabblox-chrom-")
                tmp = os.path.join(temp_dir, "Cookies")
                try:
                    shutil.copy2(db_path, tmp)
                    for suffix in ["-wal", "-shm", "-journal"]:
                        aux = db_path + suffix
                        if os.path.exists(aux):
                            try:
                                shutil.copy2(aux, tmp + suffix)
                            except Exception:
                                pass

                    conn = sqlite3.connect(tmp, timeout=1.5)
                    cur = conn.cursor()
                    try:
                        cur.execute('SELECT value, encrypted_value FROM cookies WHERE (host_key LIKE "%roblox.com%" OR host_key LIKE "%roblox%") AND name = ".ROBLOSECURITY" ORDER BY last_access_utc DESC, creation_utc DESC')
                    except Exception:
                        cur.execute('SELECT value, encrypted_value FROM cookies WHERE (host_key LIKE "%roblox.com%" OR host_key LIKE "%roblox%") AND name = ".ROBLOSECURITY"')
                    rows = cur.fetchall()
                    conn.close()

                    for val, enc in rows:
                        # 1. Plaintext cookie check
                        if val and isinstance(val, str) and val.strip():
                            v = val.strip()
                            if "_|WARNING:-DO" in v or len(v) > 50:
                                try:
                                    username = save_session_cookie(v)
                                    return username, bname
                                except Exception:
                                    pass

                        # 2. Encrypted cookie check
                        if enc and isinstance(enc, (bytes, bytearray)) and enc.startswith(b"v10") and len(enc) > 3:
                            for pwd in passwords:
                                try:
                                    key = hashlib.pbkdf2_hmac("sha1", pwd, b"saltysalt", 1, 16)
                                    iv = b" " * 16
                                    proc = subprocess.Popen(
                                        ["openssl", "enc", "-d", "-aes-128-cbc", "-K", key.hex(), "-iv", iv.hex()],
                                        stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE
                                    )
                                    out, _ = proc.communicate(enc[3:])
                                    text = out.decode("utf-8", errors="ignore")
                                    idx = text.find("_|WARNING:-DO")
                                    if idx != -1:
                                        cookie = text[idx:].strip()
                                        cookie = cookie.rstrip("\x00\r\n\t ")
                                        try:
                                            username = save_session_cookie(cookie)
                                            return username, bname
                                        except Exception:
                                            pass
                                except Exception:
                                    pass
                except Exception:
                    pass
                finally:
                    shutil.rmtree(temp_dir, ignore_errors=True)

    # 2. Gecko-based browsers (unencrypted SQLite)
    gecko_patterns = [
        ("Firefox", "~/.mozilla/firefox/*/cookies.sqlite"),
        ("Firefox (Snap)", "~/snap/firefox/common/.mozilla/firefox/*/cookies.sqlite"),
        ("Firefox (Flatpak)", "~/.var/app/org.mozilla.firefox/.mozilla/firefox/*/cookies.sqlite"),
        ("Zen Browser", "~/.zen/*/cookies.sqlite"),
        ("Zen Browser (Flatpak)", "~/.var/app/app.zen_browser.zen/.zen/*/cookies.sqlite"),
        ("Floorp", "~/.floorp/*/cookies.sqlite"),
        ("Floorp (Flatpak)", "~/.var/app/one.ablaze.floorp/.floorp/*/cookies.sqlite"),
        ("LibreWolf", "~/.librewolf/*/cookies.sqlite"),
        ("LibreWolf (Flatpak)", "~/.var/app/io.gitlab.librewolf-community/.librewolf/*/cookies.sqlite"),
        ("Waterfox", "~/.waterfox/*/cookies.sqlite"),
    ]

    for bname, pat in gecko_patterns:
        for db_path in glob.glob(os.path.expanduser(pat)):
            if not os.path.exists(db_path):
                continue
            temp_dir = tempfile.mkdtemp(prefix="crabblox-gecko-")
            tmp = os.path.join(temp_dir, "cookies.sqlite")
            try:
                shutil.copy2(db_path, tmp)
                for suffix in ["-wal", "-shm", "-journal"]:
                    aux = db_path + suffix
                    if os.path.exists(aux):
                        try:
                            shutil.copy2(aux, tmp + suffix)
                        except Exception:
                            pass

                conn = sqlite3.connect(tmp, timeout=1.5)
                cur = conn.cursor()
                try:
                    cur.execute('SELECT value FROM moz_cookies WHERE (host LIKE "%roblox.com" OR host LIKE "%roblox%") AND name = ".ROBLOSECURITY" ORDER BY lastAccessed DESC, creationTime DESC')
                except Exception:
                    cur.execute('SELECT value FROM moz_cookies WHERE (host LIKE "%roblox.com" OR host LIKE "%roblox%") AND name = ".ROBLOSECURITY"')
                rows = cur.fetchall()
                conn.close()

                for row in rows:
                    if row and row[0]:
                        cookie = str(row[0]).strip().rstrip("\x00\r\n\t ")
                        if cookie:
                            try:
                                username = save_session_cookie(cookie)
                                return username, bname
                            except Exception:
                                pass
            except Exception:
                pass
            finally:
                shutil.rmtree(temp_dir, ignore_errors=True)

    return None, None


def exit_reason(log_path):
    """A known cause for a game that quit, from its log, or None.
    "captcha": Roblox tried to show its web view (captcha on sign-up or
    password sign-in), which Darling does not have."""
    try:
        with open(log_path, "rb") as file:
            file.seek(0, os.SEEK_END)
            file.seek(max(0, file.tell() - 16384))
            tail = file.read().decode(errors="replace")
    except (OSError, TypeError):
        return None
    if "class WKWebView" in tail or "Selector setDetachesHiddenViews:" in tail:
        return "captcha"
    return None


def logout():
    # Files inside ~/.darling must not be removed from the host while
    # darlingserver runs: its overlay then stops showing new files to the host.
    try:
        USER_CACHE_FILE.unlink(missing_ok=True)
    except OSError:
        pass
    if darlingserver_running():
        subprocess.run(["darling", "shell", "/bin/rm", "-rf",
                        *[_darling_path(path) for path in SESSION_FILES]],
                       env=dict(os.environ, EGL_PLATFORM="x11"), stdin=subprocess.DEVNULL,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=60)
        return
    for path in SESSION_FILES:
        if path.is_dir():
            shutil.rmtree(path, ignore_errors=True)
        elif path.exists():
            path.unlink()


def cleanup_logs(keep):
    """Remove old logs written by the launcher itself (launch-YYYYmmdd-HHMMSS.log);
    logs from run_debug.sh and other tools are left alone."""
    import re
    pattern = re.compile(r"launch-\d{8}-\d{6}\.log")
    logs = sorted((p for p in LOGS.glob("launch-*.log") if pattern.fullmatch(p.name)),
                  key=lambda p: p.stat().st_mtime, reverse=True)
    for old in logs[keep:]:
        try:
            old.unlink()
        except OSError:
            pass


def build_shim():
    if PREBUILT_SHIM:
        return True, _("The shim comes built with this package")
    result = subprocess.run([str(BUILD_SCRIPT)], capture_output=True, text=True,
                            env=dict(os.environ, MACOBLOX_BUILD_DIR=str(BUILD_DIR),
                                     DARLING_SYSROOT=str(DARLING_SYSROOT)))
    return result.returncode == 0, (result.stdout + result.stderr).strip()


def shim_built():
    return SHIM.exists() and all((FRAMEWORKS_BUILD / f"{name}.framework" / name).exists()
                                 for name in FRAMEWORKS)


def missing_tools():
    """Programs the launcher needs that are not installed."""
    needed = {"darling": "darling", "unzip": "unzip"}
    if not PREBUILT_SHIM:
        needed.update({"clang": "clang", "ld.lld": "lld"})
    missing = [package for program, package in needed.items() if not shutil.which(program)]
    if not DARLING_SYSROOT.is_dir() and "darling" not in missing:
        missing.append(f"darling ({DARLING_SYSROOT})")
    return missing


def _missing_frameworks():
    relative = Path("System/Library/Frameworks")
    return [name for name in FRAMEWORKS
            if not (DARLING_PREFIX / relative / f"{name}.framework").exists()
            and not (DARLING_SYSROOT / relative / f"{name}.framework").exists()]


def prepare_prefix(env):
    """Puts the stub frameworks and the patched ffmpeg bridges into the
    Darling prefix. Its system folders belong to root, so programs inside
    Darling cannot write there; the files go straight into the prefix's
    upper layer (~/.darling) while Darling is stopped, then Darling sees them
    on its next start."""
    frameworks = _missing_frameworks()
    bridges = _patched_ffmpeg_bridges()
    if not frameworks and not bridges:
        return
    if not DARLING_PREFIX.is_dir():
        # Let Darling create the prefix first.
        subprocess.run(["darling", "shell", "/bin/true"], env=env, stdin=subprocess.DEVNULL,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=300)
    if darlingserver_running():
        restart_darling()
    target = DARLING_PREFIX / "System" / "Library" / "Frameworks"
    target.mkdir(parents=True, exist_ok=True)
    for name in frameworks:
        shutil.copytree(FRAMEWORKS_BUILD / f"{name}.framework", target / f"{name}.framework",
                        symlinks=True, dirs_exist_ok=True)
    for relative, path in bridges:
        target = DARLING_PREFIX / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(path, target)


def _initializer_offset(data):
    """File offset of the `_initializer` function in the x86_64 slice of a
    (fat) Mach-O library, or None."""
    slice_offset = 0
    if data[:4] == b"\xca\xfe\xba\xbe":
        for index in range(struct.unpack_from(">I", data, 4)[0]):
            cputype, _sub, offset, _size, _align = struct.unpack_from(">5I", data, 8 + index * 20)
            if cputype == 0x01000007:
                slice_offset = offset
                break
        else:
            return None
    if data[slice_offset:slice_offset + 4] != b"\xcf\xfa\xed\xfe":
        return None
    ncmds = struct.unpack_from("<I", data, slice_offset + 16)[0]
    position = slice_offset + 32
    segments, symtab = [], None
    for _ in range(ncmds):
        command, size = struct.unpack_from("<II", data, position)
        if command == 0x19:  # LC_SEGMENT_64
            segments.append(struct.unpack_from("<4Q", data, position + 24))
        elif command == 0x2:  # LC_SYMTAB
            symtab = struct.unpack_from("<4I", data, position + 8)
        position += size
    if not symtab:
        return None
    symoff, nsyms, stroff, _strsize = symtab
    for index in range(nsyms):
        strx, _type, _sect, _desc, value = struct.unpack_from("<IBBHQ", data, slice_offset + symoff + index * 16)
        name_start = slice_offset + stroff + strx
        if data[name_start:data.index(b"\0", name_start)] != b"_initializer":
            continue
        for vmaddr, vmsize, fileoff, _filesize in segments:
            if vmaddr <= value < vmaddr + vmsize:
                return slice_offset + fileoff + value - vmaddr
    return None


def _host_libraries():
    try:
        output = subprocess.run(["ldconfig", "-p"], capture_output=True, text=True).stdout
    except OSError:
        return set()
    return {line.split()[0] for line in output.splitlines()[1:] if line.strip()}


def _patched_ffmpeg_bridges():
    """Darling's bridges to host libraries (/usr/lib/native/libav*.dylib and
    others) load one exact host version from an initializer, and the game
    exits when it is missing ("Cannot load libavformat.so.60"). Roblox does
    not need these, so when the host has another version (or none, as in the
    Flatpak), the prefix gets copies whose initializer returns right away.
    Returns (path in the macOS root, patched file) pairs to install."""
    host = None
    patched = []
    for relative in NATIVE_LIBS:
        # A rootless prefix is a full copy of the macOS root, so the stock
        # bridge may already be in it; patch whichever copy Darling uses.
        installed = DARLING_PREFIX / relative
        stock = installed if installed.exists() else DARLING_SYSROOT / relative
        name = Path(relative).stem
        try:
            data = bytearray(stock.read_bytes())
        except OSError:
            continue
        wanted = re.search(rb"%s\.so\.\d+" % name.encode(), data)
        host = _host_libraries() if host is None else host
        if not wanted or wanted.group().decode() in host:
            continue
        offset = _initializer_offset(data)
        if offset is None or data[offset] != 0x55:  # push %rbp; 0xC3 = already patched
            continue
        data[offset] = 0xC3  # ret
        NATIVE_BUILD.mkdir(parents=True, exist_ok=True)
        (NATIVE_BUILD / f"{name}.dylib").write_bytes(data)
        (NATIVE_BUILD / f"{name}.dylib").chmod(0o755)
        patched.append((relative, NATIVE_BUILD / f"{name}.dylib"))
    return patched


def _process_state(pid):
    try:
        return Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()[0]
    except (OSError, IndexError):
        return None


def clear_stale_darling():
    """If the container's init is gone or a zombie (its parent never reaped
    it), darling refuses to start ("Cannot open mnt namespace file"); move
    its pid file and socket aside so a new server starts."""
    prefix = DARLING_PREFIX
    try:
        pid = int((prefix / ".init.pid").read_text().strip())
    except (OSError, ValueError):
        return
    if _process_state(pid) not in (None, "Z"):
        return
    for name in (".init.pid", ".darlingserver.sock"):
        path = prefix / name
        if path.exists():
            path.rename(prefix / (name + ".stale"))


def restart_darling():
    """Stops the prefix's darlingserver and its launchd, which otherwise
    stays behind as an orphan; the next darling command starts them again."""
    try:
        init = int((DARLING_PREFIX / ".init.pid").read_text().strip())
    except (OSError, ValueError):
        init = None
    for pid in _darlingservers():
        try:
            os.kill(pid, signal.SIGTERM)
        except OSError:
            pass
    time.sleep(2)
    if init and _process_state(init) not in (None, "Z"):
        try:
            os.kill(init, signal.SIGTERM)
        except OSError:
            pass
        time.sleep(1)
    clear_stale_darling()


def icon_argb_file():
    """Write the logo in _NET_WM_ICON layout for the shim (see MACOBLOX_ICON_ARGB)."""
    target = CACHE_DIR / "icon.argb"
    sources = [ICONS / f"crabblox-{size}.png" for size in (32, 64, 128)]
    if not any(s.exists() for s in sources):
        sources = [ICONS / f"macoblox-{size}.png" for size in (32, 64, 128)]
    if target.exists() and all(target.stat().st_mtime >= s.stat().st_mtime for s in sources if s.exists()):
        return target
    from gi.repository import GdkPixbuf  # only needed here

    words = bytearray()
    for source in sources:
        if not source.exists():
            continue
        pixbuf = GdkPixbuf.Pixbuf.new_from_file(str(source))
        if not pixbuf.get_has_alpha():
            pixbuf = pixbuf.add_alpha(False, 0, 0, 0)
        width, height, stride = pixbuf.get_width(), pixbuf.get_height(), pixbuf.get_rowstride()
        pixels = pixbuf.get_pixels()
        words += struct.pack("<II", width, height)
        for y in range(height):
            row = pixels[y * stride:y * stride + width * 4]
            for x in range(0, width * 4, 4):
                r, g, b, a = row[x], row[x + 1], row[x + 2], row[x + 3]
                words += struct.pack("<I", (a << 24) | (r << 16) | (g << 8) | b)
    CACHE_DIR.mkdir(parents=True, exist_ok=True)
    target.write_bytes(bytes(words))
    return target


LAUNCH_SCRIPT = r'''
project=$1 shim_dir=$2; shift 2
for kv in "$@"; do export "$kv"; done
export MESA_SHADER_CACHE_DIR="${XDG_CACHE_HOME:-$HOME/.cache}/mesa_shader_cache"
export __GL_SHADER_DISK_CACHE=1
export __GL_SHADER_DISK_CACHE_PATH="${XDG_CACHE_HOME:-$HOME/.cache}"
export mesa_glthread=true
app="$project/RobloxPlayer.app/Contents/MacOS"
cd "$app" || exit 1
# Nothing may run between these exports and exec: every program started
# after them would get the shim injected too.
export DYLD_FORCE_FLAT_NAMESPACE=1
export DYLD_INSERT_LIBRARIES="$shim_dir/libMacOBloxShims.dylib"
export DYLD_LIBRARY_PATH="$shim_dir:$app"
exec ./RobloxPlayer
'''


def host_vram_bytes():
    """Largest dedicated VRAM among the host GPUs (amdgpu exposes it in sysfs)."""
    best = 0
    for path in Path("/sys/class/drm").glob("card*/device/mem_info_vram_total"):
        try:
            best = max(best, int(path.read_text().strip()))
        except (OSError, ValueError):
            pass
    return best


class HostAudio:
    """Game sound played on the host. The shim writes raw float32 stereo
    44.1 kHz audio into a FIFO and pw-cat plays it through PipeWire, or pacat
    through PulseAudio where only that is reachable (the Flatpak). Darling's
    own audio (CoreAudio over PulseAudio on GCD) overflows Darling's
    workqueue thread stacks within seconds, so it is not used. The launcher
    keeps the FIFO open read/write for the whole session, so pw-cat never
    sees end of file and the game can reopen it any time."""

    NAME = "Roblox (Crabblox)"

    def __init__(self, fifo, keep, player):
        self.fifo, self.keep, self.player = fifo, keep, player

    @classmethod
    def _player_command(cls, fifo):
        runtime = Path(os.environ.get("XDG_RUNTIME_DIR", f"/run/user/{os.getuid()}"))
        pipewire = os.environ.get("PIPEWIRE_REMOTE") or (runtime / "pipewire-0").exists()
        if pipewire and shutil.which("pw-cat"):
            return ["pw-cat", "--playback", "--raw", "--format", "f32", "--rate", "44100",
                    "--channels", "2", "--latency", "60ms", "--media-role", "Game",
                    "-P", '{ application.name = "Roblox" application.icon-name = "macoblox" '
                          'media.name = "Roblox (Crabblox)" }',
                    str(fifo)]
        if shutil.which("pacat"):
            return ["pacat", "--playback", "--raw", "--format=float32le", "--rate=44100",
                    "--channels=2", "--latency-msec=60", "--client-name=Roblox",
                    "--stream-name=Roblox (Crabblox)", "--property=media.role=game", str(fifo)]
        return None

    @classmethod
    def start(cls):
        if not cls._player_command(""):
            return None
        CACHE_DIR.mkdir(parents=True, exist_ok=True)
        fifo = CACHE_DIR / f"audio-{os.getpid()}.fifo"
        if fifo.exists():
            fifo.unlink()
        os.mkfifo(fifo, 0o600)
        keep = os.open(fifo, os.O_RDWR)
        try:
            import fcntl
            fcntl.fcntl(keep, getattr(fcntl, "F_SETPIPE_SZ", 1031), 262144)
        except Exception:
            pass
        player = subprocess.Popen(
            cls._player_command(fifo),
            stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        return cls(fifo, keep, player)

    def stop(self):
        self.player.terminate()
        try:
            self.player.wait(timeout=3)
        except subprocess.TimeoutExpired:
            self.player.kill()
        os.close(self.keep)
        try:
            self.fifo.unlink()
        except OSError:
            pass


class RobloxSession:
    """One run of the client. poll() returns None while it is running."""

    def __init__(self, settings):
        self.settings = settings
        self.log_path = None
        self.process = None
        self.seen_roblox = False
        self.gone_since = None
        self.dns = None
        self.audio = None

    def environment(self):
        env = dict(os.environ)
        # Darling's Mesa receives X11 displays; a Wayland session may say otherwise.
        env["EGL_PLATFORM"] = "x11"
        if NOROOT_LIB:
            env["LD_PRELOAD"] = NOROOT_LIB
        return env

    def shim_variables(self):
        variables = [f"MACOBLOX_MOUSE_SENSITIVITY={self.settings['mouse_sensitivity']:.2f}"]
        vram = host_vram_bytes()
        if vram:
            variables.append(f"MACOBLOX_VRAM_BYTES={vram}")
        if self.settings.get("hide_menu_bar"):
            variables.append("MACOBLOX_HIDE_MENU_BAR=1")
        if self.dns:
            variables.append(f"MACOBLOX_DNS={self.dns.address}")
        if self.audio:
            variables.append(f"MACOBLOX_AUDIO_FIFO=/Volumes/SystemRoot{self.audio.fifo}")
        else:
            # Darling's own audio path crashes the game (see HostAudio).
            variables.append("MACOBLOX_AUDIO=0")
        for key, name in TRACE_ENV.items():
            if self.settings.get(key):
                variables.append(f"{name}=1")
        try:
            variables.append(f"MACOBLOX_ICON_ARGB=/Volumes/SystemRoot{icon_argb_file()}")
        except Exception:
            pass
        return variables

    def start(self):
        missing = missing_tools()
        if missing:
            raise RuntimeError(_("Install these first: {programs}", programs=", ".join(missing)))
        if not shim_built():
            ok, output = build_shim()
            if not ok:
                raise RuntimeError(_("Could not build the shim:\n{output}", output=output))
        env = self.environment()
        prepare_prefix(env)
        provider = self.settings.get("dns", "system")
        if provider != "system" and (provider != "custom" or self.settings.get("dns_custom")):
            from .dns import DnsForwarder
            self.dns = DnsForwarder(provider, self.settings.get("dns_custom", ""))
        self.audio = HostAudio.start()
        clear_stale_darling()
        if not darlingserver_running():
            # The first process after darlingserver starts sometimes fails to
            # check in; warm the server up with a trivial command first.
            subprocess.run(["darling", "shell", "/bin/true"], env=env, stdin=subprocess.DEVNULL,
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=120)
        LOGS.mkdir(parents=True, exist_ok=True)
        cleanup_logs(int(self.settings.get("keep_logs", 30)) - 1)
        self.log_path = LOGS / time.strftime("launch-%Y%m%d-%H%M%S.log")
        log = open(self.log_path, "wb")
        log.write(f"Crabblox {__version__} by Monster Dev\n".encode())
        log.flush()
        command = ["darling", "shell", "/bin/bash", "-c", LAUNCH_SCRIPT, "macoblox",
                   f"/Volumes/SystemRoot{DATA_DIR}", f"/Volumes/SystemRoot{SHIM.parent}",
                   *self.shim_variables()]
        self.process = subprocess.Popen(command, env=env, stdin=subprocess.DEVNULL,
                                        stdout=log, stderr=subprocess.STDOUT,
                                        start_new_session=True)
        log.close()

    def poll(self):
        """None while running, otherwise the exit status (or -1 if unknown)."""
        status = self.process.poll() if self.process else -1
        if status is not None:
            self.finish()
            return status
        # darling shell can outlive a Roblox that was killed; watch the game
        # processes themselves as well.
        if roblox_pids():
            self.seen_roblox = True
            self.gone_since = None
        elif self.seen_roblox:
            self.gone_since = self.gone_since or time.time()
            if time.time() - self.gone_since > 6:
                self.finish()
                return -1
        return None

    def finish(self):
        if self.dns:
            self.dns.stop()
            self.dns = None
        if self.audio:
            self.audio.stop()
            self.audio = None
