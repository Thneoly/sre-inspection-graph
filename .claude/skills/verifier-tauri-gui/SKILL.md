---
name: verifier-tauri-gui
description: Verify this project's Tauri desktop GUI by launching the real app, forcing an observable X11 surface when possible, driving the window, and capturing screenshots/logs. Use for desktop UI changes, Tauri command wiring, Cytoscape graph rendering, or any change whose runtime surface is the SRE Graph desktop window.
---

# Tauri GUI verifier — SRE Inspection Graph

This verifier captures runtime evidence for the real Tauri desktop app. It is **not** a test/typecheck runner. Do not run unit tests as verification evidence; launch the app and observe the GUI.

## Scope

Use this verifier when the change reaches:

- `desktop/src/**` React UI
- `desktop/src-tauri/**` Tauri commands / app startup
- `engine-wasm` changes visible through desktop commands such as `list_connectors` / `sync_all_now`
- `modules/connectors/**` changes that affect what the desktop renders
- Cytoscape topology rendering

## Golden path: force X11 for observable GUI automation

The default GNOME session may run Tauri as a Wayland surface. In that mode common X11 tools (`xwininfo`, XTest input, XGetImage screenshots) cannot see the window. For verifier automation, **force X11**:

```bash
cd /home/cc/Desktop/code/SRE/graph_data/desktop
GDK_BACKEND=x11 npm run tauri dev
```

Expected readiness evidence:

```text
VITE v... ready
Running `target/debug/sre-graph-desktop`
wasm runtime ready ... connectors=2 load_errors=0 names=["hello-world", "k8s-mini"]
```

Find the window:

```bash
xwininfo -root -tree | grep -i -E 'SRE Inspection Graph|sre-graph-desktop'
```

Expected shape:

```text
0x... "SRE Inspection Graph": ("sre-graph-desktop" "Sre-graph-desktop") 1280x800+...
```

## Driving input under X11

If `xdotool` exists, prefer it:

```bash
xdotool search --name "SRE Inspection Graph" windowactivate
xdotool key Tab Return
```

If `xdotool` is absent, use XTest via Python/ctypes:

```bash
python3 - <<'PY'
import ctypes, time
libX11 = ctypes.CDLL('libX11.so.6')
libXtst = ctypes.CDLL('libXtst.so.6')
d = libX11.XOpenDisplay(None)
# Replace with actual child window id from xwininfo, e.g. 0x3400003
win = 0x3400003
libX11.XRaiseWindow(d, win)
libX11.XSetInputFocus(d, win, 1, 0)  # RevertToParent
libX11.XFlush(d)
time.sleep(0.2)
TAB, RET = 23, 36
# Try focus traversal to the first button, then press Enter
for _ in range(3):
    libXtst.XTestFakeKeyEvent(d, TAB, True, 0)
    libXtst.XTestFakeKeyEvent(d, TAB, False, 0)
libXtst.XTestFakeKeyEvent(d, RET, True, 0)
libXtst.XTestFakeKeyEvent(d, RET, False, 0)
libX11.XFlush(d)
PY
```

For the Phase 1 topology page, successful activation of `Sync all now` should emit app logs like:

```text
wasm-guest: hello-world sync invoked
wasm-guest: k8s-mini sync: cluster=demo namespaces=2 with_topology=true
```

## Capturing screenshots under X11

If `scrot` / `maim` / `import` exists, use it. Otherwise use XGetImage via Python + Pillow:

```bash
python3 - <<'PY'
import ctypes
from PIL import Image
libX11 = ctypes.CDLL('libX11.so.6')
class XWindowAttributes(ctypes.Structure):
    _fields_ = [('x', ctypes.c_int), ('y', ctypes.c_int), ('width', ctypes.c_int), ('height', ctypes.c_int), ('border_width', ctypes.c_int), ('depth', ctypes.c_int), ('visual', ctypes.c_void_p), ('root', ctypes.c_ulong), ('class_', ctypes.c_int), ('bit_gravity', ctypes.c_int), ('win_gravity', ctypes.c_int), ('backing_store', ctypes.c_int), ('backing_planes', ctypes.c_ulong), ('backing_pixel', ctypes.c_ulong), ('save_under', ctypes.c_int), ('colormap', ctypes.c_ulong), ('map_installed', ctypes.c_int), ('map_state', ctypes.c_int), ('all_event_masks', ctypes.c_long), ('your_event_mask', ctypes.c_long), ('do_not_propagate_mask', ctypes.c_long), ('override_redirect', ctypes.c_int), ('screen', ctypes.c_void_p)]
class XImage(ctypes.Structure):
    _fields_ = [('width', ctypes.c_int), ('height', ctypes.c_int), ('xoffset', ctypes.c_int), ('format', ctypes.c_int), ('data', ctypes.c_char_p), ('byte_order', ctypes.c_int), ('bitmap_unit', ctypes.c_int), ('bitmap_bit_order', ctypes.c_int), ('bitmap_pad', ctypes.c_int), ('depth', ctypes.c_int), ('bytes_per_line', ctypes.c_int), ('bits_per_pixel', ctypes.c_int), ('red_mask', ctypes.c_ulong), ('green_mask', ctypes.c_ulong), ('blue_mask', ctypes.c_ulong)]
libX11.XOpenDisplay.restype = ctypes.c_void_p
libX11.XGetWindowAttributes.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.POINTER(XWindowAttributes)]
libX11.XGetImage.restype = ctypes.POINTER(XImage)
libX11.XGetImage.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_int, ctypes.c_int, ctypes.c_uint, ctypes.c_uint, ctypes.c_ulong, ctypes.c_int]
libX11.XDestroyImage.argtypes = [ctypes.POINTER(XImage)]
d = libX11.XOpenDisplay(None)
# Replace with actual window id from xwininfo
win = 0x3400003
attrs = XWindowAttributes(); libX11.XGetWindowAttributes(d, win, ctypes.byref(attrs))
imgp = libX11.XGetImage(d, win, 0, 0, attrs.width, attrs.height, 0xffffffff, 2)
img = imgp.contents
buf = ctypes.string_at(img.data, img.bytes_per_line * img.height)
out = Image.frombuffer('RGB', (img.width, img.height), buf, 'raw', 'BGRX', img.bytes_per_line, 1)
path = '/tmp/sre-graph-gui-verification.png'
out.save(path)
libX11.XDestroyImage(imgp)
print(path, attrs.width, attrs.height)
PY
```

For Cytoscape healthy-green nodes, an optional pixel sanity check:

```bash
python3 - <<'PY'
from PIL import Image
path = '/tmp/sre-graph-gui-verification.png'
img = Image.open(path).convert('RGB')
green = []
for y in range(img.height):
    for x in range(img.width):
        r, g, b = img.getpixel((x, y))
        if 40 <= r <= 100 and 140 <= g <= 210 and 60 <= b <= 120:
            green.append((x, y))
if green:
    print('green bbox', min(x for x,y in green), min(y for x,y in green), max(x for x,y in green), max(y for x,y in green), 'count', len(green))
else:
    print('green none')
PY
```

## Wayland fallback

If forcing `GDK_BACKEND=x11` fails or product behavior must be verified under Wayland, do **not** pretend X11 evidence covers pixels. Use Wayland-native tools if available:

- screenshot: `grim`, `gnome-screenshot`, `spectacle`, or portal screenshot tools
- input: `ydotool`, `wtype`, or a desktop automation tool already approved in the environment
- window discovery: compositor-specific tools may be needed; plain `xwininfo` will not see Wayland windows

If none are available, report:

```text
BLOCKED — Wayland GUI surface could not be captured/driven. X11-forced app path works/does not work: <evidence>. Install one of grim+ydotool/wtype or run with GDK_BACKEND=x11 for replayable verification.
```

## Cleanup

Stop the background Tauri task after capture. If launched by Bash background task, use `TaskStop`. Do not leave `npm run tauri dev` or Vite processes running.

## Report template

```markdown
## Verification: <feature>

**Verdict:** PASS | FAIL | BLOCKED

**Claim:** <what this desktop change is supposed to do>

**Method:** Project verifier `verifier-tauri-gui`; launched real Tauri app with `GDK_BACKEND=x11 npm run tauri dev`; drove the window via <xdotool|XTest>; captured screenshot via <tool>.

### Steps

1. ✅ App launched → <runtime ready log>
2. ✅ Window observed → <xwininfo line>
3. ✅ User action performed → <button click / key sequence + app log>
4. ✅ GUI rendered → <screenshot path + pixel/visual evidence>
5. 🔍 Probe → <adjacent behavior checked>

**Screenshot / sample:** <path or inline output>

### Findings

- <observations from running the app>
```
