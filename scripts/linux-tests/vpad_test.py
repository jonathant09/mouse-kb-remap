#!/usr/bin/env python3
"""End-to-end test of the Linux virtual controller, mouse -> stick and
hiding (grabbing) a copied controller. Run as root inside WSL:
  python3 vpad_test.py /path/to/mouse-kb-remap
Uses fake uinput devices only; the app gets its own scratch config."""
import json, os, select, subprocess, sys, time
from evdev import UInput, InputDevice, AbsInfo, list_devices, ecodes as e

APP = sys.argv[1]
DIR = "/tmp/mkr-test"
os.makedirs(DIR, exist_ok=True)
CONFIG = {
    "version": 5, "enabled": True, "close_to_tray": True,
    "pad": {"copy_to_virtual": True, "hide_real": True, "mouse_stick_hide": True,
            "mouse_stick_speed": 10, "mouse_stick_deadzone": 20},
    "macros": [{"name": "Pad combo", "steps": [
        {"type": "pad_tap", "button": "y", "hold_ms": 150}, {"type": "delay", "ms": 50},
        {"type": "pad_tap", "button": "rb", "hold_ms": 150}]}],
    "profiles": [{"name": "Default", "mouse_stick": "right", "bindings": {
        "key:F13": {"type": "pad", "button": "a"},
        "key:F14": {"type": "macro", "macro": "Pad combo"},
        "pad:b": {"type": "key", "key": "F16"}}}],
}
json.dump(CONFIG, open(f"{DIR}/config.json", "w"))

stick = AbsInfo(0, -32768, 32767, 16, 128, 0)
kb = UInput({e.EV_KEY: [e.KEY_A, e.KEY_SPACE, e.KEY_F13, e.KEY_F14, e.KEY_F16]}, name="test keyboard")
mouse = UInput({e.EV_KEY: [e.BTN_LEFT, e.BTN_RIGHT], e.EV_REL: [e.REL_X, e.REL_Y, e.REL_WHEEL]}, name="test mouse")
pad = UInput({e.EV_KEY: [e.BTN_SOUTH, e.BTN_EAST, e.BTN_NORTH, e.BTN_WEST, e.BTN_TL, e.BTN_TR,
                         e.BTN_SELECT, e.BTN_START, e.BTN_MODE, e.BTN_THUMBL, e.BTN_THUMBR],
              e.EV_ABS: [(e.ABS_X, stick), (e.ABS_Y, stick), (e.ABS_RX, stick), (e.ABS_RY, stick),
                         (e.ABS_Z, AbsInfo(0, 0, 255, 0, 0, 0)), (e.ABS_RZ, AbsInfo(0, 0, 255, 0, 0, 0))]},
             name="test gamepad", vendor=0x1234, product=0x5678)
time.sleep(0.5)

env = dict(os.environ, MOUSE_KB_REMAP_CONFIG=f"{DIR}/config.json")
app = subprocess.Popen([APP, "--tray"], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

def find(name, timeout=10):
    end = time.time() + timeout
    while time.time() < end:
        for p in list_devices():
            try:
                d = InputDevice(p)
                if d.name == name:
                    return d
            except OSError:
                pass
        time.sleep(0.1)
    return None

def log():
    try:
        return open(f"{DIR}/mouse-kb-remap.log").read()
    except OSError:
        return ""

results = []
def check(name, ok, got=""):
    results.append((bool(ok), name, got))

def tap(dev, code, hold=0.06):
    dev.write(e.EV_KEY, code, 1); dev.syn(); time.sleep(hold)
    dev.write(e.EV_KEY, code, 0); dev.syn()

try:
    # The scanner takes the fake devices within a few seconds.
    end = time.time() + 12
    while time.time() < end and log().count("engine: using") < 3:
        time.sleep(0.2)
    time.sleep(0.5)
    # Pressing F13 makes the profile want the controller, as on Windows it's
    # plugged in when the profile uses it (here: right away).
    vp = find("mouse-kb-remap virtual controller")
    check("virtual controller created", vp is not None)
    if vp is None:
        raise SystemExit
    info = vp.info
    check("it has Xbox 360 ids", (info.vendor, info.product) == (0x045E, 0x028E), f"{info.vendor:04x}:{info.product:04x}")
    caps = vp.capabilities()
    check("buttons and axes like xpad", e.BTN_SOUTH in caps.get(e.EV_KEY, []) and
          any(a[0] == e.ABS_HAT0X for a in caps.get(e.EV_ABS, [])))

    kb.write(e.EV_KEY, e.KEY_F13, 1); kb.syn(); time.sleep(0.15)
    check("F13 held -> A", e.BTN_SOUTH in vp.active_keys())
    kb.write(e.EV_KEY, e.KEY_F13, 0); kb.syn(); time.sleep(0.15)
    check("F13 released -> A released", e.BTN_SOUTH not in vp.active_keys())

    # Macro with controller steps: Y, then RB.
    tap(kb, e.KEY_F14)
    seen = []
    end = time.time() + 0.6
    while time.time() < end:
        k = tuple(sorted(vp.active_keys()))
        if not seen or seen[-1] != k:
            seen.append(k)
        time.sleep(0.005)
    y_at = next((i for i, s in enumerate(seen) if e.BTN_WEST in s), None)
    rb_at = next((i for i, s in enumerate(seen) if e.BTN_TR in s), None)
    check("macro: Y, then RB, then nothing", y_at is not None and rb_at is not None and y_at < rb_at
          and seen[-1] == () and all(len(s) <= 1 for s in seen), seen)
    time.sleep(0.3)

    # Mouse -> right stick, the pointer kept still (nothing forwarded).
    out = find("mouse-kb-remap virtual input")
    for _ in range(60):
        mouse.write(e.EV_REL, e.REL_X, 8); mouse.syn(); time.sleep(0.004)
    rx = vp.absinfo(e.ABS_RX).value
    check("mouse right -> right stick right", rx > 8000, rx)
    forwarded = []
    while out and select.select([out.fd], [], [], 0)[0]:
        forwarded += [ev for ev in out.read() if ev.type == e.EV_REL]
    check("pointer kept still (no motion forwarded)", not forwarded, len(forwarded))
    time.sleep(0.3)
    check("stick centred after", vp.absinfo(e.ABS_RX).value == 0, vp.absinfo(e.ABS_RX).value)
    ry = []
    for _ in range(40):
        mouse.write(e.EV_REL, e.REL_Y, -40); mouse.syn(); time.sleep(0.004)
    ry = vp.absinfo(e.ABS_RY).value
    check("mouse up fast -> right stick full up (evdev minimum)", ry <= -32000, ry)
    time.sleep(0.3)

    # Copying the real controller: A passes through, B is remapped (left out).
    reader = InputDevice(pad.device.path)
    while select.select([reader.fd], [], [], 0)[0]:
        reader.read()
    pad.write(e.EV_KEY, e.BTN_SOUTH, 1); pad.syn(); time.sleep(0.15)
    check("real A -> copied to virtual A", e.BTN_SOUTH in vp.active_keys())
    pad.write(e.EV_ABS, e.ABS_X, -20000); pad.syn(); time.sleep(0.15)
    check("real left stick -> copied", vp.absinfo(e.ABS_X).value < -15000, vp.absinfo(e.ABS_X).value)
    pad.write(e.EV_KEY, e.BTN_SOUTH, 0); pad.write(e.EV_ABS, e.ABS_X, 0); pad.syn(); time.sleep(0.15)
    pad.write(e.EV_KEY, e.BTN_EAST, 1); pad.syn(); time.sleep(0.15)
    check("real B (remapped) -> not copied", e.BTN_EAST not in vp.active_keys())
    pad.write(e.EV_KEY, e.BTN_EAST, 0); pad.syn(); time.sleep(0.15)
    # Hidden: another program reading the real controller gets nothing.
    got = []
    while select.select([reader.fd], [], [], 0.05)[0]:
        got += [ev for ev in reader.read() if ev.type == e.EV_KEY]
    check("real controller hidden from other readers (grabbed)", not got, len(got))
    check("log says it's hidden", "engine: hiding test gamepad" in log())
finally:
    app.terminate()
    try:
        app.wait(5)
    except subprocess.TimeoutExpired:
        app.kill()
    for ok, name, got in results:
        print(f"{'PASS' if ok else 'FAIL'}  {name}  {got if not ok else ''}")
    print(log())
