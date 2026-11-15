#!/usr/bin/env python3
"""Wayland focus tracking against a real, headless sway. Run as root in WSL:
  python3 sway_test.py /path/to/mouse-kb-remap
Two terminal windows; a profile for one of them by window title (and one
for fullscreen) changes what F13 sends to the virtual controller."""
import glob, json, os, subprocess, sys, time
from evdev import UInput, InputDevice, list_devices, ecodes as e

APP = sys.argv[1]
DIR = "/tmp/mkr-sway"
RUN = "/tmp/mkr-sway-xdg"
os.makedirs(DIR, exist_ok=True)
os.makedirs(RUN, mode=0o700, exist_ok=True)
json.dump({
    "version": 5, "enabled": True, "close_to_tray": True,
    "profiles": [
        {"name": "Default", "bindings": {"key:F13": {"type": "pad", "button": "a"}}},
        {"name": "Game", "apps": ["title:game window"], "bindings": {"key:F13": {"type": "pad", "button": "b"}}},
        {"name": "Fullscreen foot", "apps": ["foot"], "fullscreen_only": True,
         "bindings": {"key:F13": {"type": "pad", "button": "x"}}},
    ],
}, open(f"{DIR}/config.json", "w"))

env = dict(os.environ, XDG_RUNTIME_DIR=RUN, WLR_BACKENDS="headless", WLR_RENDERER="pixman",
           WLR_LIBINPUT_NO_DEVICES="1")
env.pop("WAYLAND_DISPLAY", None)
open(f"{DIR}/sway.conf", "w").write("output HEADLESS-1 resolution 1280x720\n")
sway = subprocess.Popen(["sway", "--unsupported-gpu", "-c", f"{DIR}/sway.conf"], env=env,
                        stdout=open(f"{DIR}/sway.log", "w"), stderr=subprocess.STDOUT)
procs = [sway]
results = []
def check(name, ok, got=""):
    results.append((bool(ok), name, got))

def log():
    try:
        return open(f"{DIR}/mouse-kb-remap.log").read()
    except OSError:
        return ""

try:
    sock = None
    for _ in range(100):
        socks = glob.glob(f"{RUN}/sway-ipc.*.sock")
        if socks:
            sock = socks[0]
            break
        time.sleep(0.1)
    check("headless sway started", sock, open(f"{DIR}/sway.log").read()[-500:])
    if not sock:
        raise SystemExit
    wl = [n for n in os.listdir(RUN) if n.startswith("wayland-") and not n.endswith(".lock")][0]
    cenv = dict(env, SWAYSOCK=sock, WAYLAND_DISPLAY=wl)
    def swaymsg(*args):
        return subprocess.run(["swaymsg", "-s", sock, *args], capture_output=True, text=True, env=cenv).stdout
    for title in ["other window", "game window"]:
        procs.append(subprocess.Popen(["foot", "--title", title, "sleep", "300"], env=cenv,
                                      stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL))
        time.sleep(1.0)
    tree = json.loads(swaymsg("-t", "get_tree", "-r"))
    def titles(n):
        out = [n.get("name")] if n.get("pid") else []
        for k in ("nodes", "floating_nodes"):
            for c in n.get(k, []):
                out += titles(c)
        return out
    check("two windows open", {"other window", "game window"} <= set(titles(tree)), titles(tree))

    kb = UInput({e.EV_KEY: [e.KEY_A, e.KEY_SPACE, e.KEY_F13]}, name="test keyboard")
    aenv = dict(os.environ, MOUSE_KB_REMAP_CONFIG=f"{DIR}/config.json", SWAYSOCK=sock, DISPLAY=":0")
    aenv.pop("WAYLAND_DISPLAY", None)
    app = subprocess.Popen([APP, "--tray"], env=aenv, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    procs.append(app)
    end = time.time() + 12
    while time.time() < end and "engine: using" not in log():
        time.sleep(0.2)
    check("app follows sway", "following the focused window through sway" in log(), log()[-400:])

    def vpad():
        for p in list_devices():
            d = InputDevice(p)
            if d.name == "mouse-kb-remap virtual controller":
                return d
    def press_f13():
        kb.write(e.EV_KEY, e.KEY_F13, 1); kb.syn(); time.sleep(0.2)
        d = vpad()
        keys = d.active_keys() if d else []
        kb.write(e.EV_KEY, e.KEY_F13, 0); kb.syn(); time.sleep(0.2)
        return keys
    names = {e.BTN_SOUTH: "A", e.BTN_EAST: "B", e.BTN_NORTH: "X"}
    def sent():
        return ",".join(names.get(k, str(k)) for k in press_f13()) or "nothing"

    swaymsg('[title="other window"] focus'); time.sleep(0.5)
    check("other window focused -> Default (A)", sent() == "A", sent())
    swaymsg('[title="game window"] focus'); time.sleep(0.5)
    check("game window focused -> title profile (B)", sent() == "B", sent())
    swaymsg('[title="other window"] focus'); time.sleep(0.5)
    check("back -> Default (A)", sent() == "A", sent())
    swaymsg('[title="other window"] fullscreen enable'); time.sleep(0.5)
    check("other window fullscreen -> fullscreen profile (X)", sent() == "X", sent())
    swaymsg('[title="other window"] fullscreen disable'); time.sleep(0.5)
    check("fullscreen off -> Default (A)", sent() == "A", sent())
finally:
    for p in reversed(procs):
        p.terminate()
    time.sleep(0.5)
    for p in procs:
        if p.poll() is None:
            p.kill()
    for ok, name, got in results:
        print(f"{'PASS' if ok else 'FAIL'}  {name}  {got if not ok else ''}")
    print("\n".join(l for l in log().splitlines() if "engine:" in l or "virtual" in l))
