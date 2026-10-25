# Unshufflz — Specification

This document describes the app in enough detail to rebuild it from scratch.
User-facing usage is in [README.md](README.md). Every change to the app must be reflected here.

## 1. Goals

- Keep the last **10** clipboard items of any kind: text, rich text, images, files, app-specific formats.
- Show them in an **overlay** on the right side of the screen while **Win** is held.
- Win+V = 1 step back (older), Win+C = 1 step forward (newer). Releasing Win puts the selected item on the clipboard. There is no automatic paste.
- Use very little memory and CPU. No browser engine, no JS, no .NET runtime.
- Save history to disk **asynchronously** and **crash-safe**.

## 2. Technology

| Item | Choice |
|---|---|
| Language | Rust 2021, stable, target `x86_64-pc-windows-msvc` |
| Windows API | `windows-sys` = **0.59.0** (pinned), raw Win32 calls |
| Drawing | GDI (no Direct2D, no UI framework) |
| Hashing | `xxhash-rust` 0.8, feature `xxh3` (`xxh3_64`, streaming `Xxh3`) |
| Release profile | `opt-level=3`, `lto=true`, `codegen-units=1`, `panic="abort"`, `strip=true` |
| Subsystem | `#![windows_subsystem = "windows"]` (no console) |

`windows-sys` features: `Win32_Foundation`, `Win32_Security`, `Win32_Graphics_Gdi`, `Win32_Storage_FileSystem`, `Win32_System_DataExchange`, `Win32_System_Diagnostics_Debug`, `Win32_System_LibraryLoader`, `Win32_System_Memory`, `Win32_System_Registry`, `Win32_System_SystemInformation`, `Win32_System_Threading`, `Win32_System_Time`, `Win32_UI_HiDpi`, `Win32_UI_Input_KeyboardAndMouse`, `Win32_UI_Shell`, `Win32_UI_WindowsAndMessaging`.

There is no build script, manifest or resource file. DPI awareness is set in code and the tray icon is drawn in code.

## 3. Source layout

| File | Role |
|---|---|
| `src/main.rs` | Entry point: single-instance mutex, DPI awareness, `config::load()`, `app::run()` |
| `src/app.rs` | `App` state, overlay window, window procedure, message loop, history operations |
| `src/hook.rs` | Keyboard hook thread; pure state machine `step()` / `on_hold()` |
| `src/clip.rs` | Clipboard capture / restore, `Fmt` / `Payload` types, format constants |
| `src/history.rs` | `Item` struct, `full_hash`, `content_key`, `payload_size`, limits |
| `src/preview.rs` | `Preview` / `Kind` / `Thumb`; builds previews; DROPFILES parsing; DIB thumbnails |
| `src/paint.rs` | `Fonts`; drawing the overlay |
| `src/store.rs` | Disk format, safe writes, writer thread, loading, cleanup |
| `src/tray.rs` | Tray icon, its menu, autostart registry |
| `src/config.rs` | `Config`, INI parsing |
| `src/util.rs` | `wide` / `w` UTF-16 helpers, `human_size`, `now_unix`, `format_time`, paths |

## 4. Startup and shutdown

Startup (`main` → `app::run`):
1. `CreateMutexW("Local\\UnshufflzSingleInstance")`. If it fails or `GetLastError() == ERROR_ALREADY_EXISTS`, exit silently.
2. `SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)`.
3. Load the config (section 13).
4. Register window class `UnshufflzOverlay` and create the overlay window (section 10). It stays hidden.
5. `SetLayeredWindowAttributes(alpha=245, LWA_ALPHA)`.
6. `RegisterWindowMessageW("TaskbarCreated")`. When Explorer restarts and sends this message, the tray icon is added again.
7. Thumbnail width = `GetSystemMetrics(SM_CXSCREEN) * WidthPercent / 100 - 32`.
8. Start the store writer thread (section 9).
9. Load the history from disk (section 9.5).
10. Capture what is on the clipboard now (it is added unless it duplicates the newest item).
11. `AddClipboardFormatListener(hwnd)`, add the tray icon, start the hook thread.
12. Run the message loop (`GetMessageW` / `TranslateMessage` / `DispatchMessageW`).

Shutdown:
- Tray **Exit** → `DestroyWindow`.
- `WM_DESTROY` → `RemoveClipboardFormatListener`, remove the tray icon, `PostQuitMessage`.
- After the loop ends, `store.shutdown()` sends Quit to the writer and waits for it, so all queued writes finish.
- `WM_ENDSESSION` (wParam ≠ 0, Windows is shutting down) also calls `store.shutdown()`. `shutdown()` is safe to call twice.

## 5. Threads and messages

| Thread | Does |
|---|---|
| Main | Overlay window, clipboard listener, timers, tray, **all** history state |
| Hook | `WH_KEYBOARD_LL` hook + message loop + hold timer. Owns the key state machine. |
| Writer | Writes item files and the index, deletes unused files |

Rust `std::sync::mpsc` channel from main to writer.

| Message | Value | From → to | wParam / lParam |
|---|---|---|---|
| `WM_APP_HOOK` | `WM_APP+1` | hook → main | wParam = action (1 SHOW, 2 PREV, 3 NEXT, 4 COMMIT, 5 CANCEL) |
| `WM_APP_SAVED` | `WM_APP+2` | writer → main | lParam = item id (u64 as isize) |
| `WM_APP_TRAY` | `WM_APP+3` | shell → main | lParam low word = mouse message |

Main window timers:

| Id | Interval | Purpose |
|---|---|---|
| 1 `TIMER_CAPTURE` | 80 ms, one-shot | Started on each `WM_CLIPBOARDUPDATE`, so a burst of updates is read once |
| 2 `TIMER_WATCH` | 150 ms, repeating while visible | Safety net: hide the overlay if no Win key is down (`GetAsyncKeyState(VK_LWIN/VK_RWIN) & 0x8000`) |

App state lives in a thread-local `RefCell<Option<App>>` on the main thread. Every handler uses `try_borrow_mut`, so a re-entrant message (e.g. during the tray menu loop) is skipped instead of panicking. A capture that hits a busy state restarts timer 1.

## 6. Keyboard hook (`hook.rs`)

### 6.1 Constants

| Name | Value |
|---|---|
| VK_LWIN / VK_RWIN | 0x5B / 0x5C |
| C / V / Esc | 0x43 / 0x56 / 0x1B |
| Modifiers (ignored) | 0x10–0x12, 0xA0–0xA5 |
| Mask key | 0xE8 (unassigned VK) |
| Injection marker (`dwExtraInfo`) | 0x434C4950 |
| Hold delay | `HoldDelayMs` (default 250 ms) |

### 6.2 Hook thread

- `SetWindowsHookExW(WH_KEYBOARD_LL, proc, GetModuleHandleW(NULL), 0)`, then a `GetMessageW` loop.
- The hold timer is a thread timer (`SetTimer(NULL, 0, HoldDelayMs, NULL)`). Its `WM_TIMER` (hwnd = NULL) is handled in the loop. Only one timer is active; starting a new one kills the old.
- The callback ignores events whose `dwExtraInfo` is the marker (our own injected keys). For all other events it calls `step()` and applies the result, in this order: stop timer, start timer, post actions, inject, then return 1 (swallow) or `CallNextHookEx`.
- The callback must be fast. It only does state changes and `PostMessageW`.

### 6.3 State machine (pure, unit-tested)

State: `phase` ∈ {Idle, Armed, Shown, Cancelled, Passthrough}, `win` = the Win VK that started the sequence.
Output: `swallow`, `inject`, `posts[]`, `start_timer`, `stop_timer`.

Rules for `step(vk, down)`, checked in this order:
1. **Idle**: Win down → Armed, `win=vk`, start_timer. Everything passes.
2. **vk == win**:
   - down (auto-repeat) → nothing.
   - up → stop_timer, phase = Idle, and:
     - from Shown → post COMMIT, swallow, inject;
     - from Cancelled → swallow, inject;
     - otherwise → pass (a quick tap opens Start).
3. The other Win key, a modifier, or phase Passthrough → pass, no change.
4. **C or V** (in Armed / Shown / Cancelled) → swallow both down and up. On down: if not Shown, then stop_timer, phase = Shown, post SHOW. Then post PREV (V) or NEXT (C).
5. **Esc** in Shown or Cancelled → swallow down and up. On down in Shown: phase = Cancelled, post CANCEL.
6. **Any other key**: on down, post CANCEL if Shown, stop_timer, phase = Passthrough. The key passes.

`on_hold()` (timer fired): if Armed → Shown and post SHOW.

### 6.4 Start menu suppression (inject)

The real Win up is swallowed. Instead, `SendInput` sends three keyboard events, all with the marker:
1. 0xE8 down
2. 0xE8 up
3. Win up (`KEYEVENTF_KEYUP | KEYEVENTF_EXTENDEDKEY`)

Windows sees a key between Win down and Win up, so it does not open the Start menu.
C/V never reach Windows, so Win+V and Win+C system shortcuts do not fire.

## 7. Clipboard (`clip.rs`)

### 7.1 Types

```
Fmt { id: u32, name: String, data: Vec<u8> }   // name only for registered formats (id >= 0xC000)
Payload = Vec<Fmt>
```

### 7.2 Capture

Triggered by timer 1, which follows `WM_CLIPBOARDUPDATE`.
1. If `GetClipboardSequenceNumber() == ignore_seq` (our own restore), stop.
2. `OpenClipboard(hwnd)`: up to 10 tries, `Sleep(15)` between.
3. Privacy: stop if any of these hold:
   - `ExcludeClipboardContentFromMonitorProcessing` is present;
   - `Clipboard Viewer Ignore` is present;
   - `CanIncludeInClipboardHistory` is present and its first DWORD is 0.
4. Enumerate all formats (`EnumClipboardFormats`). Skip:
   - `CF_BITMAP` (2), `CF_METAFILEPICT` (3), `CF_PALETTE` (9): GDI handles, which Windows synthesizes again on paste;
   - `CF_TEXT` (1), `CF_OEMTEXT` (7) if `CF_UNICODETEXT` (13) exists;
   - `CF_DIB` (8) if `CF_DIBV5` (17) exists;
   - 0x80–0x8F (owner display / DSP formats), 0x200–0x3FF (private and GDI-object ranges);
   - registered formats with an empty name, `DataObject` or `Ole Private Data`.
5. For each kept format, `GetClipboardData`:
   - `CF_ENHMETAFILE` (14): bytes via `GetEnhMetaFileBits`.
   - Others: `GlobalSize` + `GlobalLock` + copy + `GlobalUnlock`. A NULL handle, size 0 or failed lock skips the format.
   - The size is added up before copying. If the total exceeds `MaxItemMB`, the whole item is dropped.
6. `CloseClipboard`. If no formats were kept, do nothing.
7. Source app: `GetClipboardOwner()` (or `GetForegroundWindow()` if NULL) → `GetWindowThreadProcessId` → `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)` → `QueryFullProcessImageNameW` → exe file name only (e.g. `chrome.exe`). Empty on failure.

### 7.3 Restore

1. Open the clipboard (same retry), then `EmptyClipboard`.
2. For each format:
   - Get the id: a registered format is re-registered by name with `RegisterClipboardFormatW`; otherwise the stored id is used.
   - `CF_ENHMETAFILE`: `SetEnhMetaFileBits` → `SetClipboardData`. On failure, `DeleteEnhMetaFile`.
   - Others: `GlobalAlloc(GMEM_MOVEABLE, max(len,1))`, copy, `SetClipboardData`. On failure, `GlobalFree`.
3. `CloseClipboard`. Then `ignore_seq = GetClipboardSequenceNumber()`.

## 8. History (`app.rs`, `history.rs`)

```
Item { id: u64, key: u64, time: u64 (unix s), source: String, size: u64,
       payload: Option<Arc<Payload>>, preview: Preview }
MAX = 10, RAM_LIMIT = 1 MiB
```

- `full_hash` (→ `id`): xxh3 over every format, in order. For each format it hashes: the id as u32 LE (or the name bytes for registered formats), the data length as u64 LE, then the data.
- `content_key` (→ `key`): the same hash over one main format only, the first found of CF_HDROP, CF_DIBV5, CF_DIB, CF_UNICODETEXT. If none exist, it equals `full_hash`.

Operations:
- **Add** (new capture):
  1. If `id` equals the newest item's id → ignore.
  2. Otherwise remove every item with the same `key`. This drops the same content copied again, even with different extra formats.
  3. Build the preview. Send the item to the writer, insert it at position 0 with time = now, and cut the list to 10.
  4. Set selection = 0, send the index to the writer, and repaint if visible.
- **Saved** (`WM_APP_SAVED`): if the item's size > RAM_LIMIT, drop its payload from RAM.
- **Select / commit** of item `i` (> 0):
  1. Get the payload from RAM, or read it from disk (`read_item`). If that fails: `MessageBeep(MB_ICONWARNING)`, remove the item, save the index, stop.
  2. Restore it to the clipboard.
  3. Move the item to position 0. Its time is unchanged. If it is large, its payload is dropped again.
  4. Save the index.
- **Clear** (tray): empty the list and save an empty index. The writer cleanup then deletes all item files.

Selection: `sel` = 0 for the newest item. SHOW sets sel=0 if the overlay was hidden. PREV: `sel+1` limited to `len-1`. NEXT: `sel-1` limited to 0. COMMIT: if `0 < sel < len`, select the item; then sel=0. CANCEL: hide, sel=0.

## 9. Storage (`store.rs`)

### 9.1 Location

`%LOCALAPPDATA%\Unshufflz\store\` (falls back to `%TEMP%\Unshufflz\store` if LOCALAPPDATA is unset). It is created at start.

### 9.2 File formats (all integers little-endian)

**Item file** `<id as 16 lowercase hex digits>.item`:
```
"CLPI"            4 bytes
version = 1       u32
count             u32
repeat count:
  format id       u32
  name length     u16
  name            UTF-8 (empty for standard formats)
  data length     u64
  data            bytes
checksum          u64 = xxh3_64 of all bytes above
```

**Index** `index.dat`:
```
"CLPX"            4 bytes
version = 1       u32
count             u32
repeat count (newest first):
  id              u64
  time            u64 (unix seconds)
  source length   u16
  source          UTF-8 exe name (cut to 65535 bytes)
checksum          u64 = xxh3_64 of all bytes above
```

A file is valid only if it is at least 8 bytes long, the checksum matches, the magic and version match, and all lengths stay inside the file.

### 9.3 Safe write procedure

1. Write `<name>.tmp` through a 64 KB `BufWriter`. The checksum is computed while writing (the data is not copied first).
2. `flush`, then `File::sync_all()` (FlushFileBuffers).
3. `MoveFileExW(tmp, target, MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH)`.
4. On any error, delete the tmp file. The old target stays untouched.

Result: after a crash or power loss there is either the complete old file or the complete new file. Item files never change once written (the name is a content hash). If the file already exists, the write is skipped.

### 9.4 Writer thread

- Jobs: `Item(id, Arc<Payload>)`, `Index(Vec<Entry>)`, `Quit`.
- It waits for one job, then takes everything else already queued as one batch, and runs the batch in order:
  - **Item** → write it (or skip if the file exists), then post `WM_APP_SAVED`.
  - **Index** → written only if it is the last index in the batch. After a successful write, clean up: delete `*.item` files whose id is not in the index, and all `*.tmp` files.
  - **Quit** → finish the batch, then exit.
- The main thread always sends an Item before the Index that lists it. So the index never points at a file that has not been written.

### 9.5 Loading

1. Read and check `index.dat`. If it is missing or invalid, scan `*.item` files instead: each name is parsed as a hex id, time = file modified time, source empty, newest first.
2. For each entry, read and check the item file. Skip missing or invalid ones. Take at most 10.
3. For each loaded item:
   - `key` = `content_key`;
   - build the preview;
   - keep the payload in RAM only if its size ≤ 1 MiB.

## 10. Overlay window

- Class `UnshufflzOverlay`, no cursor and no background brush; title `Unshufflz`.
- Style `WS_POPUP`. Extended style `WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_LAYERED | WS_EX_TRANSPARENT`, alpha 245/255.
  - It is never activated, never in the taskbar or Alt+Tab, and mouse clicks pass through it.
  - `WM_MOUSEACTIVATE` → `MA_NOACTIVATE`. `WM_ERASEBKGND` → 1. `WM_DPICHANGED` → 0 (ignored; the size is set on each show).
- **Show**:
  1. Take the monitor under the cursor (`MonitorFromPoint`, nearest).
  2. Width = `max(200, monitor width * WidthPercent / 100)`. Height = work area height.
  3. `x = work.right - width`, `y = work.top`.
  4. `SetWindowPos(HWND_TOPMOST, …, SWP_NOACTIVATE | SWP_SHOWWINDOW)`, then start timer 2.
- **Hide**: `ShowWindow(SW_HIDE)`, stop timer 2.
- It only repaints while visible (`InvalidateRect`).

## 11. Drawing (`paint.rs`)

Double buffered: draw into a memory bitmap of the window size, then `BitBlt`. Text uses `SetBkMode(TRANSPARENT)` and `DT_NOPREFIX`.
Scale: `s(v) = v * dpi / 96`, where dpi = `GetDpiForWindow` (minimum 96).

Fonts (Segoe UI, ClearType, recreated when the DPI changes; height = `-(pt * dpi / 72)`):

| Font | Size | Weight |
|---|---|---|
| title | 12 pt | 600 |
| body | 10 pt | 400 |
| small | 9 pt | 400 |

Colors:

| Use | Color |
|---|---|
| Background | `#202020` |
| Main text | `#F0F0F0` |
| Secondary text | `#A8A8A8` |
| Accent bar (left, 3 px) | `#0078D7` |
| Selected row | `#104070` |
| Separator lines | `#404040` |

Layout (`pad = s(14)`, `x0 = s(3)+pad`, `x1 = width-pad`, `y` starts at `pad`):
1. Title row (height s(24)): "Clipboard" on the left (title font) and `"{sel+1} / {n}"` on the right.
   - If the history is empty, it shows "History is empty" (body font, secondary color) at `y+s(40)` instead of everything below.
2. `y += s(28)`: info line (small, secondary, single line with ellipsis), height s(18).
3. `y += s(18)`: `"<time> · <source>"` (or just the time) in small font.
   - Time is `HH:MM:SS` for today, otherwise `DD.MM.YYYY HH:MM`, in local time.
4. `y += s(26)`: 1 px separator; `y += s(12)`.
5. List: `row = s(24)`, `list_top = height - pad - row*n`. Separator at `list_top - s(8)`.
6. Preview area: from `y` to `list_top - s(16)`, between x0 and x1.
7. List rows: index number (small, secondary, width s(20)) and the item's summary line at `x0+s(22)` (body font; main color if selected, else secondary). The selected row gets a background from `x0-s(6)` to `x1+s(6)`.

Preview area by kind:
- **Text / Rich / Other**: `DrawTextW` with `DT_WORDBREAK | DT_EXPANDTABS | DT_EDITCONTROL`, clipped to the area.
- **Image**: the thumbnail scaled to fit (never enlarged), centered horizontally, top aligned, `StretchBlt` with `HALFTONE`. "(no preview)" if there is no thumbnail.
- **Files**:
  - First, the parent folder (small, secondary, path ellipsis), height s(20), then a gap of s(6).
  - Then one row per file, height s(20): the name on the left (body, main color, ellipsis); the size on the right in a column s(72) wide (small, secondary).
  - When there is no room for 2 more rows, the last row shows `+N more` instead.

## 12. Preview building (`preview.rs`)

```
Preview { kind: Text|Rich|Image|Files|Other, info: String, text: Vec<u16>,
          thumb: Option<Thumb{hbmp,w,h}>, files: Vec<(name, size)>, line: Vec<u16> }
```
Built once per item. `Thumb` deletes its HBITMAP on drop. The checks below run in this order.

1. **Files** (CF_HDROP present):
   - Parse DROPFILES: `pFiles` = u32 at 0, `fWide` = u32 at 16. Names start at `pFiles`, separated by NUL, ending with an empty name. UTF-16 if `fWide`, else bytes read as UTF-8 (lossy). If the data is under 20 bytes or the offset is outside it, the list is empty.
   - For up to 500 paths, `std::fs::metadata` gives the size: `"folder"` for a folder, the readable size for a file (added to the total), `"missing"` on error.
   - info: `Files · {n} file(s), {m} folder(s) · {total}`. If there are no files and no folders: `{count} items`.
   - text = parent folder of the first path.
   - line = first name, plus ` (+N more)` if there is more than one.
2. **Text** (CF_UNICODETEXT present):
   - UTF-16 up to the first NUL.
   - Kind = Rich if `Rich Text Format` or `HTML Format` is present. Label: `Text`, `Rich text (RTF)`, `Rich text (HTML)`, or `Rich text (RTF, HTML)`.
   - info: `{label} · {chars} chars · {lines} line(s)`, where lines = count of `\n` + 1.
   - text = the first 4000 UTF-16 units.
   - line = the first non-empty trimmed line, at most 120 chars, tabs replaced by spaces.
3. **Image** (CF_DIBV5, else CF_DIB):
   - info: `Image · {w} × {h} · {bpp}-bit`. line: `Image {w} × {h}`.
   - Thumbnail (only if all of these hold):
     - header ≥ 40 bytes and fits in the data;
     - width > 0, height ≠ 0;
     - compression is BI_RGB (0) or BI_BITFIELDS (3);
     - bpp is one of 1, 4, 8, 16, 24, 32;
     - pixel data fits in the buffer.
   - Pixel data offset = header size, + 12 if the header is 40 bytes and BI_BITFIELDS, + 4 × color count. Color count = `biClrUsed` (max 256), or `2^bpp` if bpp ≤ 8, else 0.
   - Size: `scale = min(maxW/w, 2*maxW/|h|, 1)`, with maxW at least 64.
   - The thumbnail is a 32-bit top-down DIB section, filled with `StretchDIBits` (HALFTONE).
4. **Other**:
   - info: `Other · {count} formats · {total size}`.
   - text = one line per format: `"<name> — <size>"`. Standard formats use names like Text, SYLK, DIF, TIFF, OEM text, DIB, Pen data, RIFF, Wave, Unicode text, Enhanced metafile, Files, Locale, DIB v5; others show `Format {id}`.
   - line = the first format's name.

Size format (`human_size`, 1024-based): `N B`; `x.x KB`; `x.x MB`; `x.xx GB`.

## 13. Config (`config.rs`)

`unshufflz.ini` in the exe's folder. It is read once at start. Each line `key=value` with a whole-number value is used. Keys are case-insensitive and trimmed. Other lines (sections, comments, bad values) are ignored.

| Key | Default | Range (clamped) |
|---|---|---|
| `HoldDelayMs` | 250 | 50–2000 |
| `WidthPercent` | 15 | 5–50 |
| `MaxItemMB` | 200 | 1–2048 (stored as bytes, MB × 2^20) |

## 14. Tray (`tray.rs`)

- `Shell_NotifyIconW(NIM_ADD)`: uID 1, callback `WM_APP_TRAY`, tooltip `Unshufflz (hold Win, V = back, C = forward)`.
- Icon: drawn in code at `SM_CXSMICON` size as a 32-bit ARGB DIB with a 1-bit mask, then `CreateIconIndirect`. Shapes, as fractions of the size (x0, y0, x1, y1):
  - clip `#505050`: (0.34, 0.03, 0.66, 0.20)
  - paper `#FFFFFF`: (0.25, 0.25, 0.75, 0.86)
  - board `#0078D7`: (0.13, 0.10, 0.87, 0.97)
  - everything else transparent.
  - If creating the bitmap fails, `IDI_APPLICATION` is used.
- Right click (`WM_RBUTTONUP` or `WM_CONTEXTMENU`) opens a popup menu:
  1. Clear history (1)
  2. Start with Windows (2, checked when enabled)
  3. separator
  4. Exit (3)
  
  It uses `SetForegroundWindow` + `TrackPopupMenu(TPM_RETURNCMD | TPM_RIGHTBUTTON)`, then posts `WM_NULL`.
- Autostart: `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, value `Unshufflz` (REG_SZ) = `"<full exe path>"` in quotes. Enabled = the value exists.

## 15. Tests

`cargo test --release` runs 29 unit tests:

| Module | Tests |
|---|---|
| hook | quick tap passes; hold → SHOW, release → COMMIT + inject; V/C navigation and swallowing; Win+E passthrough; other key while shown → CANCEL; modifiers ignored; Esc cancel + mask; Esc before overlay passes; C/V without Win ignored |
| store | item round trip, no leftover tmp; bit flip / truncation / empty rejected; index round trip + cleanup of unused items and tmp files; broken index → scan fallback; writer thread persists and double shutdown is safe |
| preview | DROPFILES parsing incl. bad offset; text preview line and count; RTF → Rich; file preview with a real file and a missing one; image thumbnail scaling 1000×500 → 200×100; small image not enlarged; truncated / RLE / negative-width DIB rejected |
| history | same text with extra HTML = same key, different id; registered formats hashed by name; different text = different key |
| config | defaults and overrides; clamping |
| util | sizes; UTF-16 helpers; time format |

Manual checks done:
- text and image capture;
- saving and reloading after restart;
- private memory ≈ 1.7 MB empty, ≈ 1.9 MB with 10 items including a 1920×1080 image;
- simulated Win hold + V presses (via `SendInput`): the overlay showed and the clipboard went back 2 items.

Not yet checked by hand:
- image and file previews on screen;
- multiple monitors;
- admin windows.

## 16. Known limits

- Win+V / Win+C system shortcuts are replaced while Win is held.
- Admin windows: the hook gets no keys from them unless the app runs as admin (UIPI).
- Stored data is not encrypted.
- File items store paths only.
- Image thumbnails ignore alpha (32-bit images are drawn as plain colors).
- DIBs with RLE, JPEG or PNG compression get no thumbnail. They are still stored and restored.
- If the hook misses a Win up (e.g. the secure desktop appeared), the overlay is hidden by timer 2. The hook state fixes itself on the next Win release.
