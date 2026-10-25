# Unshufflz — Specification

This document describes the app and its installer in enough detail to rebuild them from scratch.
User-facing usage is in [README.md](README.md). Every change to the app must be reflected here.

Current version: **0.3.1**. The version lives in the root `Cargo.toml`. The installer reads it from there, so `installer/Cargo.toml`'s own version does not matter.

## 1. Goals

- Keep the last **N** clipboard items (default 10, settable 1–100) of any kind: text, rich text, images, files, app-specific formats.
- Show them in an **overlay** on the right (or left) side of the screen while **Win** is held.
- Win+V = 1 step back (older), Win+C = 1 step forward (newer). Releasing Win puts the selected item on the clipboard. There is no automatic paste.
- Win+S = when the current item is an image: save it as PNG to `Pictures\Clipboard`, and replace it (history and clipboard) with the saved file.
- Win+B = delete the highlighted item. Deleting the current clipboard item puts the next older one on the clipboard.
- Choosing an item does not reorder the list; the overlay opens on the current clipboard item. So "Ctrl+V, Win+V" repeated pastes copied items newest first (and "Ctrl+V, Win+B" does too, deleting them).
- At most 10 list rows are visible. Longer histories scroll with the selection, without a scrollbar.
- A **settings window** from the tray: width, side, history size.
- Use very little memory and CPU. No browser engine, no JS, no .NET runtime.
- Save history to disk **asynchronously** and **crash-safe**. Items that drop out of history are removed from disk too.
- A per-user **installer / uninstaller** without admin rights.

## 2. Technology

| Item | Choice |
|---|---|
| Language | Rust 2021, stable, target `x86_64-pc-windows-msvc` |
| Windows API | `windows-sys` = **0.59.0** (pinned), raw Win32 calls |
| COM | `windows` = **0.58.0** (pinned): installer `IShellLinkW` / `IPersistFile`; app `IFileOpenDialog` (folder picker) and OLE drag of file items (`OleInitialize`, `SHCreateDataObject`, `SHDoDragDrop`, `ILCreateFromPathW`). App features: `Win32_Foundation`, `Win32_System_Com`, `Win32_System_Ole`, `Win32_UI_Shell`, `Win32_UI_Shell_Common` |
| Drawing | GDI (no Direct2D, no UI framework) |
| Hashing | `xxhash-rust` 0.8, feature `xxh3` (`xxh3_64`, streaming `Xxh3`) |
| PNG encoding | `png` 0.17 (pure Rust) |
| Release profile (workspace root) | `opt-level=3`, `lto=true`, `codegen-units=1`, `panic="abort"`, `strip=true` |
| Subsystem | `#![windows_subsystem = "windows"]` (no console), for both exes |

App `windows-sys` features: `Win32_Foundation`, `Win32_Security`, `Win32_Graphics_Dwm`, `Win32_Graphics_Gdi`, `Win32_Storage_FileSystem`, `Win32_System_Com`, `Win32_System_DataExchange`, `Win32_System_Diagnostics_Debug`, `Win32_System_LibraryLoader`, `Win32_System_Memory`, `Win32_System_Registry`, `Win32_System_SystemInformation`, `Win32_System_Threading`, `Win32_System_Time`, `Win32_UI_Accessibility`, `Win32_UI_Controls`, `Win32_UI_HiDpi`, `Win32_UI_Input_KeyboardAndMouse`, `Win32_UI_Shell`, `Win32_UI_WindowsAndMessaging`.

Installer features:
- `windows-sys`: `Win32_Foundation`, `Win32_System_Registry`, `Win32_System_Threading`, `Win32_UI_WindowsAndMessaging`.
- `windows`: `Win32_Foundation`, `Win32_System_Com`, `Win32_UI_Shell`, `Win32_Storage_FileSystem`.

### 2.1 Manifest

`app.manifest` is embedded into **both** exes by their `build.rs`, through the linker arguments `/MANIFEST:EMBED` and `/MANIFESTINPUT:<path>` (`cargo:rustc-link-arg-bins`). It declares:
- `Microsoft.Windows.Common-Controls` 6.0.0.0: themed controls in the settings window;
- `requestedExecutionLevel level="asInvoker"`. Without it, Windows would ask for admin rights for an exe whose name contains "setup";
- `supportedOS` Windows 10/11;
- `dpiAwareness` PerMonitorV2. The app also calls `SetProcessDpiAwarenessContext(PER_MONITOR_AWARE_V2)`.

## 3. Repository layout

| Path | Role |
|---|---|
| `Cargo.toml` | App package `unshufflz` and the workspace (`members = [".", "installer"]`, `default-members = ["."]`) |
| `build.rs` | Embeds the manifest into the app |
| `app.manifest` | See 2.1 |
| `build.ps1` | Test + build everything, copy installer to `dist\UnshufflzSetup-<version>.exe` |
| `src/main.rs` | Entry: single instance (a second launch opens settings), DPI, start the app |
| `src/app.rs` | `App` state, overlay window, window procedure, message loop, history operations, applying settings |
| `src/hook.rs` | Keyboard hook thread; pure state machine `step()` / `on_hold()` |
| `src/clip.rs` | Clipboard capture / restore, `Fmt` / `Payload`, format constants |
| `src/history.rs` | `Item`, `full_hash`, `content_key`, `keep_visible`, `payload_size`, `RAM_LIMIT` |
| `src/imagesave.rs` | Win+S: DIB parsing / conversion to RGBA, PNG encoding, file naming, Pictures folder, "copied file" payload |
| `src/preview.rs` | `Preview` / `Kind` / `Thumb`; preview building; DROPFILES parsing; DIB thumbnails |
| `src/paint.rs` | `Fonts`, `LIST_ROWS`, `OPACITY`; `render` (draw + `UpdateLayeredWindow`), `compose` (slide shift + alpha) |
| `src/drag.rs` | `init` (OleInitialize), `drag_files`: shell drag of file paths out of the overlay (section 10a) |
| `src/anim.rs` | Slide-in curve: `cubic_bezier`, `SLIDE`, `slide_offset` |
| `src/settings.rs` | Settings window |
| `src/store.rs` | Disk format, safe writes, writer thread, loading, cleanup |
| `src/tray.rs` | Tray icon, menu, autostart registry |
| `src/config.rs` | `Config`, `Side`, INI parse / format / save |
| `src/util.rs` | UTF-16 helpers, `human_size`, `now_unix`, `format_time`, paths, `replace_file`, `write_file_safe`, `window_exe_name`, `debug` (diagnostic log) |
| `installer/Cargo.toml` | Package `unshufflz-setup` |
| `installer/build.rs` | Manifest, `APP_EXE` path, `APP_VERSION` |
| `installer/src/main.rs` | Install / uninstall logic |

`.gitignore`: `/target`, `/dist`.

## 4. Startup and shutdown (app)

Startup (`main` → `app::run`):
0. `OleInitialize` on the main thread (needed for drag and drop, section 10a).
1. `CreateMutexW("Local\\UnshufflzSingleInstance")`. If it fails or the mutex already exists, another copy is running:
   - `FindWindowW("UnshufflzOverlay")`, then `AllowSetForegroundWindow(its pid)` so it may come to the front;
   - post `WM_APP_SETTINGS` to it (it opens its settings window), then exit.
2. `SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)`.
3. Load the config (section 13).
4. Register class `UnshufflzOverlay` and create the overlay window (section 10). It stays hidden.
5. No `SetLayeredWindowAttributes`: the layered window is drawn only with `UpdateLayeredWindow` (section 11).
6. `RegisterWindowMessageW("TaskbarCreated")`. When Explorer restarts, the tray icon is added again.
7. Thumbnail width = `GetSystemMetrics(SM_CXSCREEN) * WidthPercent / 100 - 32`.
8. Start the store writer thread (section 9).
9. Load up to `MaxItems` items from disk (section 9.5).
10. Capture what is on the clipboard now.
11. `AddClipboardFormatListener`, add the tray icon, start the hook thread.
    - `SetWinEventHook(EVENT_SYSTEM_FOREGROUND, …, WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS)`: its callback posts `WM_APP_FOREGROUND` (lParam = new foreground window) to the overlay window.
    - `start_open` starts as `is_start_menu(GetForegroundWindow())`.
12. Message loop: `GetMessageW`. If the settings window is open and `IsDialogMessageW(settings, &msg)` handles the message (Tab, Enter, Esc), skip it. Otherwise `TranslateMessage` + `DispatchMessageW`.

Shutdown:
- Tray **Exit** or `WM_CLOSE` (sent by the installer) → `DestroyWindow`.
- `WM_DESTROY` → `RemoveClipboardFormatListener`, remove the tray icon, `PostQuitMessage`.
- After the loop ends, `store.shutdown()` waits for the writer to finish its queue.
- `WM_ENDSESSION` (wParam ≠ 0) also calls `store.shutdown()`. It is safe to call twice.

## 5. Threads and messages

| Thread | Does |
|---|---|
| Main | Overlay window, settings window, clipboard listener, timers, tray, **all** history state |
| Hook | `WH_KEYBOARD_LL` hook + message loop + hold timer; owns the key state machine |
| Writer | Writes item files and the index, deletes unused files |

| Message | Value | From → to | wParam / lParam |
|---|---|---|---|
| `WM_APP_HOOK` | `WM_APP+1` | hook → main | wParam = action (1 SHOW, 2 PREV, 3 NEXT, 4 COMMIT, 5 CANCEL, 6 DELETE, 7 SAVE, 8 TAP) |
| `WM_APP_SAVED` | `WM_APP+2` | writer → main | lParam = item id |
| `WM_APP_TRAY` | `WM_APP+3` | shell → main | lParam low word = mouse message |
| `WM_APP_SETTINGS` | `WM_APP+4` | second instance → main | open the settings window |
| `WM_APP_FOREGROUND` | `WM_APP+5` | WinEvent callback → main | lParam = new foreground window |

Main window timers:

| Id | Interval | Purpose |
|---|---|---|
| 1 `TIMER_CAPTURE` | 80 ms, one-shot | Started on each `WM_CLIPBOARDUPDATE`, so a burst of updates is read once |
| 2 `TIMER_WATCH` | 150 ms, while visible | Hide the overlay if no Win key is down (`GetAsyncKeyState`), except in ShowWithStart mode while Start is open or `tap_pending` |
| 3 `TIMER_TAP` | 600 ms, one-shot | ShowWithStart: after a quick tap, hide the overlay if Start did not open |

App state lives in a thread-local `RefCell<Option<App>>`. Handlers use `try_borrow_mut`, so a re-entrant message is skipped instead of panicking. A capture that hits a busy state restarts timer 1.

## 6. Keyboard hook (`hook.rs`)

### 6.1 Constants

| Name | Value |
|---|---|
| VK_LWIN / VK_RWIN | 0x5B / 0x5C |
| C / V / B / S / Esc | 0x43 / 0x56 / 0x42 / 0x53 / 0x1B |
| Modifiers (passed through, tracked) | Shift 0x10/0xA0/0xA1 = bit 1, Ctrl 0x11/0xA2/0xA3 = bit 2, Alt 0x12/0xA4/0xA5 = bit 4 |
| Mask key | 0xE8 |
| Injection marker (`dwExtraInfo`) | 0x434C4950 |
| Hold delay | `HoldDelayMs` (default 0 = show at once) |
| Tap time `TAP_MS` | 300 ms |
| Stale press `STALE_MS` | 1500 ms |

### 6.2 Hook thread

- `SetWindowsHookExW(WH_KEYBOARD_LL, proc, GetModuleHandleW(NULL), 0)`, then a `GetMessageW` loop.
- The hold timer is a thread timer (`SetTimer(NULL, 0, HoldDelayMs, NULL)`), handled as `WM_TIMER` with hwnd NULL. Only one is active at a time.
- If `HoldDelayMs` is 0, no timer is used: when `step()` asks to start the timer, the callback runs `on_hold()` at once and posts SHOW. The overlay is visible about 7 ms after Win down (measured).
- `HoldDelayMs` lives in a global atomic; `hook::set_hold_ms` changes it (settings window).
- The callback ignores events carrying the marker. For other events it runs `step()` and applies the result in this order: stop timer, start timer, post actions, inject, then return 1 (swallow) or `CallNextHookEx`.

### 6.3 State machine (pure, unit-tested)

State: `phase` ∈ {Idle, Armed, Shown, Cancelled, Passthrough}; `win` = the Win VK that started the sequence; `mods` = bits of the modifiers held now; `image` = whether the current clipboard item is an image; `now` = this event's time (`KBDLLHOOKSTRUCT.time`, ms); `down_time` = when Win went down; `acted` = C/V/B/S was pressed during this hold; `last_win` = time of the last event of the held Win key.
- `image` is copied from a global `AtomicBool` before each event. The main thread sets it with `hook::set_image_on_clipboard` after every history change (in `save_index`) and after loading.
- Before any rule, `mods` is updated: a modifier down sets its bit, up clears it (in every phase).
Output: `swallow`, `inject`, `posts[]`, `start_timer`, `stop_timer`.

Rules for `step(vk, down)`, checked in this order:
0. **Missed release**: phase is not Idle, this is the held Win key going down, and `now − last_win > STALE_MS` → phase = Idle and stop_timer, then continue with rule 1.
   - Auto-repeat comes at least about once a second while Win is the last key pressed, and stops once another key is pressed. So a later Win down can only be a new press after a Win up the hook never saw (lock screen, UAC prompt, admin window).
   - Without this rule, that press would be taken as a repeat and ignored.
1. **Idle**: Win down → Armed, `win=vk`, `down_time=last_win=now`, `acted=false`, start_timer. Everything passes.
2. **vk == win** (sets `last_win=now`):
   - down (auto-repeat) → nothing.
   - up → stop_timer, phase = Idle, and:
     - from Shown, a **tap** (`!acted` and `now − down_time < TAP_MS`, using wrapping subtraction) → post **TAP** and pass the Win up, so **the Start menu opens (or closes, if it was open). This is intended**: with a short or zero delay the overlay is already visible on a quick tap, and the tap must still open Start;
     - from Shown otherwise → post COMMIT, swallow, inject (Start stays closed);
     - from Cancelled → swallow, inject;
     - otherwise → pass.
3. The other Win key, a modifier, or phase Passthrough → pass, no change.
4. **C/V/B**, and **S** when `image && mods == 0` (in Armed / Shown / Cancelled) → swallow both down and up. On down: `acted = true`; if not Shown, then stop_timer, phase = Shown, post SHOW. Then post PREV (V), NEXT (C), DELETE (B) or SAVE (S). Otherwise S falls to rule 6 (normal Windows shortcut).
5. **Esc** in Shown or Cancelled → swallow. On down in Shown → Cancelled, post CANCEL.
6. **Any other key**: on down, post CANCEL if Shown, stop_timer, phase = Passthrough. The key passes.

`on_hold()`: Armed → Shown, post SHOW.

### 6.4 Start menu suppression

The real Win up is swallowed. `SendInput` then sends, all with the marker:
1. 0xE8 down
2. 0xE8 up
3. Win up (`KEYEVENTF_KEYUP | KEYEVENTF_EXTENDEDKEY`)

Windows sees a key between Win down and Win up, so the Start menu stays closed. C/V/B (and S, when taken) never reach Windows.

## 7. Clipboard (`clip.rs`)

### 7.1 Types

```
Fmt { id: u32, name: String, data: Vec<u8> }   // name only for registered formats (id >= 0xC000)
Payload = Vec<Fmt>
```

### 7.2 Capture

1. If `GetClipboardSequenceNumber() == ignore_seq` (our own restore), stop.
2. `OpenClipboard`: up to 10 tries, 15 ms apart.
3. Privacy: stop if any of these hold:
   - `ExcludeClipboardContentFromMonitorProcessing` is present;
   - `Clipboard Viewer Ignore` is present;
   - `CanIncludeInClipboardHistory` is present and its first DWORD is 0.
4. Enumerate all formats and skip:
   - `CF_BITMAP` 2, `CF_METAFILEPICT` 3, `CF_PALETTE` 9;
   - `CF_TEXT` 1 / `CF_OEMTEXT` 7 when `CF_UNICODETEXT` 13 exists;
   - `CF_DIB` 8 when `CF_DIBV5` 17 exists;
   - 0x80–0x8F, 0x200–0x3FF;
   - registered formats with an empty name, `DataObject` or `Ole Private Data`.
5. Read each kept format:
   - `CF_ENHMETAFILE` 14: `GetEnhMetaFileBits`.
   - Others: `GlobalSize` + `GlobalLock` + copy.
   - The size is added up first. Over `MaxItemMB` drops the whole item.
6. `CloseClipboard`.
7. Source app: `GetClipboardOwner` (or the foreground window) → process id → `QueryFullProcessImageNameW` → exe file name.

### 7.3 Clear

`clip::clear`: open the clipboard (same retry), `EmptyClipboard`, `CloseClipboard`. The caller then saves `ignore_seq`.

### 7.4 Restore

1. Open the clipboard (same retry), then `EmptyClipboard`.
2. For each format:
   - Get the id: a registered format is re-registered by name; otherwise the stored id is used.
   - EMF: `SetEnhMetaFileBits` → `SetClipboardData`; `DeleteEnhMetaFile` on failure.
   - Others: `GlobalAlloc(GMEM_MOVEABLE)` + copy → `SetClipboardData`; `GlobalFree` on failure.
3. `CloseClipboard`, then `ignore_seq = GetClipboardSequenceNumber()`.

## 8. History (`app.rs`, `history.rs`)

```
Item { id: u64, key: u64, time: u64 (unix s), source: String, size: u64,
       payload: Option<Arc<Payload>>, preview: Preview }
RAM_LIMIT = 1 MiB; capacity = Config.max_items (1..100)
App { …, cur: usize, sel: usize, scroll: usize, … }
```

- `full_hash` (→ `id`): xxh3 over every format: id as u32 LE (or the name bytes for registered formats), data length as u64 LE, data.
- `content_key` (→ `key`): the same hash over one main format, the first found of CF_HDROP, CF_DIBV5, CF_DIB, CF_UNICODETEXT. If none exist, it equals `full_hash`.
- `cur` = index of the item that is on the clipboard now. Choosing an item in the overlay does **not** reorder the list; it only moves `cur`. So "Ctrl+V, Win+V" repeated pastes the history newest first (D, C, B, A). `cur` is not saved; at startup it is found again from the clipboard (see Add). Every list change keeps `cur` pointing at the same item (or clamps it to `len−1`). The Win+S image flag uses item `cur`.

Operations:
- **Add**:
  1. Same `id` as item `cur` → ignore.
  2. At startup only (the first capture after load): if any item has the same `id`, set `cur` to it and stop (the order is kept across restarts).
  3. Otherwise remove every item with the same `key`.
  4. Build the preview, send the item to the writer, insert it at position 0 with time = now.
  5. **Cut the list to `max_items`**.
  6. cur = 0, sel = 0, send the index to the writer, repaint if visible.
- **Saved**: if size > RAM_LIMIT, drop the payload from RAM.
- **Select / commit** of item `i` ≠ `cur`:
  1. Get the payload from RAM or from disk. If that fails: beep, remove the item (fix `cur`), save the index.
  2. Restore it to the clipboard.
  3. `cur = i`. The list order and the item's time are unchanged. Update the Win+S image flag.
- **Delete** (`delete(i)`, Win+B, i = sel):
  1. Remove item `i`.
  2. If `i < cur`: cur − 1. If `i == cur`, the deleted item was the clipboard content:
     - `cur = min(cur, len−1)`, so the next older item (or the newer one if the oldest was deleted);
     - if items remain, restore item `cur` (payload from RAM or disk; an unreadable item is dropped and the next one is tried);
     - if none remain, empty the clipboard (`clip::clear`).
     
     Either way, save `ignore_seq`.
  3. If `i ≠ cur`, the clipboard is not touched.
  4. Set `sel = min(sel, len−1)` (0 if empty) and save the index. The writer then deletes the item's file.
- **Save image** (`save_image`, Win+S):
  1. Only if item `cur` is of kind Image; get its payload (RAM or disk).
  2. `imagesave::save_png(payload, target_dir(cfg.pictures_dir))`. On failure: `MessageBeep(MB_ICONWARNING)`, nothing else changes.
  3. Build a new item from `file_payload(path)` (source `unshufflz.exe`, time = now, sent to the writer) and put it **in place of** item `cur` (same position). The old file is deleted by the next cleanup. Save the index.
  4. Restore the new item to the clipboard and save `ignore_seq`.
- **Clear**: empty the list, cur = 0, save an empty index.
- **Apply settings** (`apply_settings(new_config)`; the settings window passes a full `Config`):
  1. Pass `hold_ms` to the hook (`hook::set_hold_ms`) and replace the config.
  2. If the list is longer than `max_items`, cut it, clamp `cur` to `max_items−1`, and save the index (the writer deletes the dropped files).
  3. Save the INI.
  4. If the width changed: recompute the thumbnail width and rebuild the previews of image items, from the payload in RAM or read from disk.

Selection and scrolling:
- **SHOW**: if hidden, sel = cur and scroll = `keep_visible(cur, 0, LIST_ROWS)`.
- **PREV**: sel+1, limited to len−1. **NEXT**: sel−1, limited to 0. After either: `scroll = keep_visible(sel, scroll, LIST_ROWS)`.
- **COMMIT**: hide; if `sel ≠ cur` and `sel < len`, select the item. **CANCEL**: hide.
- **TAP**: like CANCEL, except in ShowWithStart mode (below).
- **DELETE**: show if hidden, `delete(sel)`, then `keep_visible`, repaint.
- **SAVE**: show if hidden, `save_image()`, sel = cur, `keep_visible`, repaint.
- **ShowWithStart mode** (`Config.show_with_start`): the overlay is also shown while the Start menu is open.
  - `is_start_menu(hwnd)`: the window's process (`util::window_exe_name`) is `StartMenuExperienceHost.exe` (Windows 10 / older 11) or `SearchHost.exe` (newer Windows 11, where Start and Search share a process; checked on 26200: the open Start menu's foreground window belongs to SearchHost). Case-insensitive.
  - `on_foreground(hwnd)`:
    - `open = is_start_menu(hwnd)`; nothing happens unless it differs from `start_open`. `start_open` is tracked even when the mode is off.
    - If the mode is on and Start **opened**: clear `tap_pending`, stop timer 3, show (sel = 0).
    - If the mode is on and Start **closed**: hide (sel = 0), unless a Win key is down; then the key release decides.
  - SHOW while `start_open` and the overlay is hidden → ignored. Pressing Win with Start open closes Start, so the overlay must not flash. C/V/B/S in the same press still show it, because their actions call `show()`.
  - TAP while Start is closed and the overlay is visible: set `tap_pending`, start timer 3 (600 ms), keep the overlay. Either Start opens (`on_foreground` takes over), or the timer hides it.
  - TAP in any other case → hide.
  - The watch timer (2) does not hide the overlay while `start_open || tap_pending`.
  - COMMIT and CANCEL hide the overlay as usual, also while Start stays open (Start was masked, so it stays open).
- `keep_visible(sel, scroll, rows)` moves the window as little as possible:
  - `sel < scroll` → `sel`;
  - `sel ≥ scroll+rows` → `sel+1−rows`;
  - otherwise → `scroll`.

## 9. Storage (`store.rs`)

### 9.1 Location

`%LOCALAPPDATA%\Unshufflz\store\` (or `%TEMP%\Unshufflz\store` if LOCALAPPDATA is unset). It is created at start.

### 9.2 File formats (little-endian)

**Item file** `<id as 16 lowercase hex>.item`:
```
"CLPI" | u32 version=1 | u32 count |
count × ( u32 format id | u16 name len | name UTF-8 | u64 data len | data ) |
u64 xxh3_64 of all bytes above
```

**Index** `index.dat`:
```
"CLPX" | u32 version=1 | u32 count |
count × ( u64 id | u64 time | u16 source len | source UTF-8 (≤ 65535 bytes) ) |
u64 xxh3_64 of all bytes above
```
Entries are newest first. A file is valid only if it is at least 8 bytes long, the checksum matches, the magic and version match, and all lengths stay inside the file.

### 9.3 Safe write

1. Write `<name>.tmp` through a 64 KB `BufWriter`, computing the checksum while writing.
2. `flush` and `sync_all` (FlushFileBuffers).
3. `util::replace_file`: `MoveFileExW(REPLACE_EXISTING | WRITE_THROUGH)`.
4. On error, delete the tmp file.

Item files never change once written. If one already exists, the write is skipped.

### 9.4 Writer thread

- Jobs: `Item(id, Arc<Payload>)`, `Index(Vec<Entry>)`, `Quit`.
- It waits for one job, then takes everything else already queued as one batch:
  - Find the **last** Index job in the batch and its id set.
  - **Item** jobs before that index whose id is not in it are **skipped**. They were already pushed out of history (e.g. 1000 fast copies with a limit of 10), so they never touch the disk.
  - Other Item jobs are written (or skipped if the file exists), then `WM_APP_SAVED` is posted.
  - Only the last Index is written. After a successful write, clean up: delete `*.item` files not in it, and all `*.tmp` files.
  - **Quit** → finish the batch, then exit.
- The main thread always sends an Item before the Index that lists it. So the index never points at a missing file, and any dropped item's file is deleted by the next cleanup.

### 9.5 Loading

1. Read `index.dat`. If it is missing or invalid, scan the `*.item` files instead, newest modified time first.
2. Read each item file and skip invalid ones. Take at most `max_items`.
3. Build each preview. Keep the payload in RAM only if its size ≤ 1 MiB.

If `max_items` was lowered while the app was not running, the extra items are not loaded and their files are deleted after the next index write.

## 10. Overlay window

- Class `UnshufflzOverlay` (class cursor `IDC_ARROW`), window title `Unshufflz`, style `WS_POPUP`. Extended style `TOPMOST | TOOLWINDOW | NOACTIVATE | LAYERED | TRANSPARENT`. `TRANSPARENT` (mouse goes through) is removed while a file item is shown, see 10a. Opacity 245/255, applied per pixel in `render`.
- `WM_MOUSEACTIVATE` → `MA_NOACTIVATE`. `WM_ERASEBKGND` → 1. `WM_DPICHANGED` → 0. `WM_CLOSE` → default (destroys the window, which exits the app).
- **Show**:
  1. Take the monitor under the cursor.
  2. Width = `max(200, monitor width × WidthPercent / 100)`. Height = work area height.
  3. `y = work.top`. `x = work.left` for **Left**, or `work.right − width` for **Right**. Store this as `app.frame`; `app.dpi` = `GetDpiForMonitor(MDT_EFFECTIVE_DPI)`.
  4. `slide` = ±width (fully at the screen edge, nothing visible) if `AnimationMs > 0`, else 0. Then `render()` (the first `UpdateLayeredWindow` also sets position and size).
  5. `SetWindowPos(HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW)`, then start timer 2.
  6. **Slide-in** (if `AnimationMs > 0`): a loop on the main thread until the offset is 0:
     - `slide = ±slide_offset(width, elapsed_ms, AnimationMs)`, then `render()`, then `DwmFlush()` (wait for the next screen refresh; `Sleep(5)` if it fails);
     - sign: + (content shifted right) for side Right, − for Left;
     - the loop blocks the main thread for the duration (50 ms by default). Keys keep queueing (the hook runs on its own thread) and are handled afterwards in order;
     - measured at 60 Hz: 3 frames, done 47–61 ms after Win down. With 1000 ms: 53 frames, done after 916 ms (the eased offset rounds to 0 before the end).
  7. Hiding is instant (no animation).
- **Hide**: `SW_HIDE`, stop timer 2.

## 10a. Dragging files out (`drag.rs`, `app.rs`)

When the shown (selected) item is a file item, its files can be dragged with the mouse from the overlay's file panel to Explorer, the desktop, a browser or any app that accepts dropped files.
- `render` (section 11) records `app.drag_area` = the preview rect (window coordinates) when the shown item is **Files**, else an empty rect; and `app.drag_rows` = the drawn file rows as (top, bottom, file index).
- After each render (`show`, `invalidate`), `update_click_through`: `WS_EX_TRANSPARENT` is cleared if `drag_area` is not empty, set otherwise (`SetWindowLongW(GWL_EXSTYLE)`). So for all other kinds the overlay stays click-through as before.
- `WM_SETCURSOR`: `IDC_HAND` while the cursor is inside `drag_area` (and the overlay is visible), else `IDC_ARROW`. Returns 1.
- `WM_LBUTTONDOWN` at (x, y):
  1. `drag_paths(x, y)` (inside the app borrow): empty if the point is outside `drag_area`. Otherwise the item's payload (RAM or disk, as in section 8) → its CF_HDROP → `parse_hdrop`. If the point is on a file row, only that file; elsewhere in the panel (folder line, `+N more`, empty space), all files.
  2. Outside the app borrow: `DragDetect(hwnd, screen point)`; if the mouse moved past the drag threshold with the button down, `drag::drag_files`. The drag runs outside the borrow so hook messages that arrive during it (Win released → commit/hide) are still handled; the overlay may hide during the drag and the drag goes on.
- `drag_files(paths)`: `ILCreateFromPathW` for each path (missing files are skipped), `SHCreateDataObject(pidlFolder = desktop, absolute PIDLs)` (works for files from different folders). The desktop is an **empty ID list** (2 zero bytes); `pidlFolder = NULL` gives an empty data object (no CF_HDROP), so every drop target refuses it, then `SHDoDragDrop(hwnd, data, NULL drop source, DROPEFFECT_COPY | DROPEFFECT_LINK)`. The shell's default drop source shows drag images. **Move is not offered**, so a drop never moves the original file (Shift is ignored; Alt gives a shortcut). `ILFree` the PIDLs afterwards. Debug log: `main drag N files: <result>`.
- Usage: hold Win (overlay shows), select a file item with V/C, then press the mouse on the file panel and drag. Win may be released during the drag. With ShowWithStart, the overlay also stays after Win is released while Start is open, so it can be dragged from then too.

## 11. Drawing (`paint.rs`)

`render(app)`:
1. Create a 32-bit top-down DIB section of the window size (`app.frame`) and draw the overlay into it with GDI (`draw`).
2. `GdiFlush`, then `compose(pixels, w, h, slide, OPACITY)`:
   - every row is shifted by `slide` px (positive = right, negative = left);
   - the uncovered pixels become 0 (fully transparent);
   - the rest become premultiplied BGRA with alpha `OPACITY` = 245 (each channel × 245 / 255). GDI's alpha byte is ignored.
3. `UpdateLayeredWindow(hwnd, screen DC, pos = frame x/y, size = frame w/h, src DC, (0,0), 0, BLENDFUNCTION{AC_SRC_OVER, 0, 255, AC_SRC_ALPHA}, ULW_ALPHA)`.

Pixels, alpha and position change in one call, so the screen never shows a half-updated frame. An earlier version clipped with `SetWindowRgn` plus a separate redraw; one frame could show the new edge with old content, and the accent bar went missing at the moving edge.
There is no WM_PAINT drawing: `WM_PAINT` only validates. Every change calls `invalidate()`, which renders at once while the overlay is visible.

Text is drawn on the opaque background first, then the whole panel gets alpha, so ClearType stays crisp.

Scale: `s(v) = v·dpi/96`, where dpi = `app.dpi` (the monitor's effective DPI, minimum 96).

Fonts (Segoe UI, ClearType, height `-(pt·dpi/72)`): title 12 pt weight 600; body 10 pt; small 9 pt.

Colors:

| Use | Color |
|---|---|
| Background | `#202020` |
| Main text | `#F0F0F0` |
| Secondary text | `#A8A8A8` |
| Accent bar | `#0078D7` |
| Selected row | `#104070` |
| Separator lines | `#404040` |

Accent bar: 3 px wide, on the edge facing the screen center:
- side Right → left edge of the overlay; content `x0 = s(3)+pad`, `x1 = width−pad`.
- side Left → right edge; content `x0 = pad`, `x1 = width−s(3)−pad`.

All text goes through one helper (`text`: select font, set color, `DrawTextW` with `DT_NOPREFIX`). It **skips empty strings**: an empty Rust slice has a dangling pointer (`0x2`), and `DrawTextW` reads it even with length 0, which crashed the app (access violation in USER32) whenever an item with an empty summary or text was drawn.

Layout (`pad = s(14)`):
1. Title row (height s(24)): "Clipboard" on the left and `"{sel+1} / {n}"` on the right.
   - If the history is empty, it shows "History is empty" at `y+s(40)` instead of everything below.
2. `y += s(28)`: info line (small, secondary, height s(18)).
3. `y += s(18)`: `"<time> · <source>"`. Time is `HH:MM:SS` for today, otherwise `DD.MM.YYYY HH:MM`.
4. `y += s(26)`: separator; `y += s(12)`.
5. List:
   - `LIST_ROWS = 10`; `rows = min(n, 10)`; `row = s(24)`.
   - `first = min(keep_visible(sel, scroll, 10), n − rows)`.
   - `list_top = height − pad − row·rows`; separator at `list_top − s(8)`.
6. Preview area: from `y` to `list_top − s(16)`.
7. List rows: items `first .. first+rows`. Each row shows the item number (`i+1`; small, secondary, width s(20)) and its summary line at `x0+s(22)`. The selected row has a background from `x0−s(6)` to `x1+s(6)` and main-color text. There is no scrollbar.

Preview area by kind:
- **Text / Rich / Other**: `DrawTextW` with word break, tab expansion, edit-control wrapping.
- **Image**: the thumbnail fitted (never enlarged), centered horizontally, `StretchBlt` HALFTONE. "(no preview)" if there is none.
- **Files**:
  - The parent folder (small, path ellipsis), row height s(20), then a gap of s(6).
  - Rows of name (body) and size (small, right-aligned, column s(72)).
  - `+N more` when fewer than 2 rows of space are left.
  - If the item has a thumbnail (single image file, see 12): it is drawn s(10) below the file row, fitted into the rest of the preview area (never enlarged), centered horizontally. It is inside the drag area, so dragging it drags the file.
- Image thumbnails (both kinds) are drawn by `thumb(dc, t, r)`: `k = min(rw/w, rh/h, 1)`, `StretchBlt` HALFTONE at `(r.left + (rw−dw)/2, r.top)`.

## 12. Preview building (`preview.rs`)

Order of checks:
1. **Files** (CF_HDROP):
   - Parse DROPFILES: `pFiles` u32 at 0, `fWide` u32 at 16; names separated by NUL, ending with an empty name. Invalid data gives an empty list.
   - For up to 500 paths, `fs::metadata` gives `folder`, the readable size (added to the total), or `missing`.
   - info: `Files · {n} file(s), {m} folder(s) · {total}`. text = parent folder of the first path. line = first name, plus ` (+N more)`.
   - **Thumbnail of an image file**: only when the item is exactly **one existing file** whose extension (any case) is one of `png jpg jpeg jfif gif bmp dib webp tif tiff ico heic avif`.
     - `SHCreateItemFromParsingName(path)` → `IShellItemImageFactory::GetImage(SIZE{maxW, maxW}, SIIGBF_THUMBNAILONLY)`: Explorer's thumbnail, from its thumbnail cache. The shell fits the image into a square of the larger requested side, so a maxW square keeps the width ≤ maxW. No thumbnail → nothing extra is drawn.
     - COM must be initialized on the calling thread. Previews are built on the main thread, which has `OleInitialize` (`drag::init`, called before `load`).
     - The bitmap is read with `GetDIBits` (32-bit top-down), its alpha is **flattened** onto the background color `#202020` (`flatten`), and it is copied into a new DIB section (`Thumb`). The shell's bitmap is deleted.
     - `flatten(px, bg)`: if every alpha is 0, the bitmap has no alpha, leave it. If no channel is above its alpha anywhere, it is premultiplied: `out = c + bg·(255−a)/255` (max 255); otherwise straight: `out = (c·a + bg·(255−a))/255`. The result is opaque RGB, so it draws like a clipboard image.
     - Rebuilt with the other thumbnails when the overlay width changes (any item that has a thumbnail).
2. **Text** (CF_UNICODETEXT):
   - Kind = Rich if `Rich Text Format` or `HTML Format` is present. Label: Text / Rich text (RTF) / (HTML) / (RTF, HTML).
   - info: `{label} · {chars} chars · {lines} line(s)`, lines = `\n` count + 1.
   - text = the first 4000 UTF-16 units.
   - line = the first non-empty trimmed line, at most 120 chars, tabs → spaces.
3. **Image** (CF_DIBV5, else CF_DIB):
   - info: `Image · {w} × {h} · {bpp}-bit`. line: `Image {w} × {h}`.
   - A thumbnail is made only for uncompressed DIBs (BI_RGB / BI_BITFIELDS, bpp 1/4/8/16/24/32) whose pixel data fits in the buffer.
   - Pixel data offset = header size, + 12 if a 40-byte header uses BI_BITFIELDS, + 4 × color count. Color count = `biClrUsed` (max 256), or `2^bpp` if bpp ≤ 8.
   - Size: `scale = min(maxW/w, 2·maxW/|h|, 1)`, with maxW ≥ 64.
   - 32-bit top-down DIB section, filled with `StretchDIBits` HALFTONE.
4. **Other**: info `Other · {count} formats · {size}`. text = lines `"<name> — <size>"`. line = first format name.

`human_size` (1024-based): `N B`, `x.x KB`, `x.x MB`, `x.xx GB`.

## 11a. Slide-in curve (`anim.rs`)

- `SLIDE` = CSS `cubic-bezier(0.32, 0.72, 0, 1)`: Apple-style ease-out, as used by iOS for sliding panels. Fast start, long soft landing.
- `cubic_bezier(curve, t)`: 0 for t ≤ 0, 1 for t ≥ 1. Otherwise it solves x(s) = t by 40 bisection steps (x is monotonic since x1, x2 ∈ [0,1]) and returns y(s), where x(s) = 3(1−s)²s·x1 + 3(1−s)s²·x2 + s³ and likewise for y.
- `slide_offset(width, elapsed, duration)` = `round((1 − cubic_bezier(SLIDE, elapsed/duration)) × width)`; 0 if duration ≤ 0.

## 12a. Saving images (`imagesave.rs`, Win+S)

**Folder**:
- `target_dir(PicturesDir)`: the `PicturesDir` setting (trimmed) if it is not empty, otherwise `default_dir()`.
- `default_dir()` = `SHGetKnownFolderPath(FOLDERID_Pictures)` + `Clipboard`. The known folder follows OneDrive / folder redirection; the string is freed with `CoTaskMemFree`.
- Fallback when the known folder fails: `%USERPROFILE%\Pictures\Clipboard`.
- The folder is created when needed.

**File name**: `Unshufflz_yyyyMMdd_HHmmss.png` in local time (`GetLocalTime`, 24-hour clock).
- The file is opened with `create_new`, so an existing file is never overwritten. If the name is taken, `_2`, `_3`, … up to 999 is added.
- The data is written through a `BufWriter`, then `sync_all`. On error the partial file is deleted.

**Content**:
1. If a registered format named `PNG` (any case) exists, its bytes are written unchanged.
2. Otherwise CF_DIBV5 (else CF_DIB) is converted to top-down RGBA (`dib_to_rgba`):
   - `dib_info` parses the header. The pixel data offset is computed as in section 12. The pixel data must fit in the buffer, and compression must be BI_RGB or BI_BITFIELDS.
   - **32-bit**, with BI_RGB or BI_BITFIELDS using the standard masks R=0x00FF0000, G=0x0000FF00, B=0x000000FF: read directly, BGRA → RGBA. If every alpha byte is 0, all alpha is set to 255 (apps often leave alpha 0 for opaque images); otherwise alpha is kept.
   - **24-bit** BI_RGB: read directly, alpha 255.
   - **Anything else** (1/4/8-bit palette, 16-bit, other masks): `StretchDIBits` 1:1 (COLORONCOLOR) into a 32-bit top-down DIB section, `GdiFlush`, then read BGRX, alpha 255.
   - Bottom-up rows (height > 0) are flipped.
3. PNG encoding (`encode_png`, `png` crate, 8-bit, default compression): RGB if all pixels are opaque (smaller file), else RGBA.

**Clipboard payload for the saved file** (`file_payload`), the same as Explorer's Copy:
- `CF_HDROP`: a DROPFILES header (20 bytes, `pFiles = 20`, `fWide = 1`), then the UTF-16 path and two NULs.
- Registered format `Preferred DropEffect`: DWORD 1 (DROPEFFECT_COPY).

Measured: a 1920×1080 image saved in about 240 ms (697 KB PNG).

## 13. Config (`config.rs`)

File: `%LOCALAPPDATA%\Unshufflz\unshufflz.ini`. It is read once at start and written by the settings window (OK).

| Key | Default | Range / values |
|---|---|---|
| `HoldDelayMs` | 0 | 0–2000 |
| `WidthPercent` | 15 | 5–50 |
| `Position` | right | `right` / `left` (any case) |
| `MaxItems` | 10 | 1–100 (`MAX_ITEMS_LIMIT`) |
| `MaxItemMB` | 200 | 1–2048 |
| `ShowWithStart` | 0 | 0 / 1 (any non-zero number = on) |
| `AnimationMs` | 50 | 0–1000 (0 = no slide-in); INI only |
| `PicturesDir` | empty (= `Pictures\Clipboard`) | folder path, taken as-is (trimmed); `=` inside the path is fine because only the first `=` splits the line |

Parsing:
- Lines are `key=value`; keys are trimmed and case-insensitive.
- Numbers are clamped to their range.
- Unknown keys, sections and bad values are ignored.

Saving (`config::save`):
- Writes all eight keys under `[General]` with CRLF line ends: HoldDelayMs, WidthPercent, Position, MaxItems, MaxItemMB, PicturesDir, ShowWithStart, AnimationMs.
- Uses `util::write_file_safe`: `.tmp`, `sync_all`, `replace_file`.

## 14. Settings window (`settings.rs`)

Opened from tray **Settings…** or by a second launch (`WM_APP_SETTINGS`). Only one copy can be open; opening it again brings it to the front.
`settings::open(cfg: Config)` keeps a copy of the config. OK changes only the fields the window shows and passes the whole config to `app::apply_settings`.

- `InitCommonControlsEx(ICC_BAR_CLASSES)`.
- Class `UnshufflzSettings`: arrow cursor, `COLOR_BTNFACE` background. Title "Unshufflz Settings". Style `WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU` (fixed size).
- Client area 360 × 362 (scaled by the window's DPI, sized with `AdjustWindowRectExForDpi`), centered on the work area of the monitor under the cursor.
- Font: `lfMessageFont` from `SystemParametersInfoForDpi(SPI_GETNONCLIENTMETRICS)`, sent to every control with `WM_SETFONT`.
- Controls (x, y, w, h in 96-dpi units):

| Control | Id | Rect | Details |
|---|---|---|---|
| STATIC "Overlay width (% of screen width)" | – | 16,16,260,20 | |
| STATIC value (right-aligned) | – | 276,16,68,20 | `"{n} %"`, updated on `WM_HSCROLL` |
| Trackbar `msctls_trackbar32` | 100 | 10,40,340,30 | `TBS_AUTOTICKS`, range 5–50, ticks every 5, page 5 |
| STATIC "Items to keep in history (1-100)" | – | 16,88,260,20 | |
| EDIT (client edge, `ES_NUMBER`, right-aligned) | 103 | 284,85,60,24 | max 3 characters |
| STATIC "Show delay when holding Win (ms, 0-2000)" | – | 16,122,260,20 | |
| EDIT (client edge, `ES_NUMBER`, right-aligned) | 104 | 284,119,60,24 | max 4 characters |
| Checkbox "Also show while the Start menu is open" | 107 | 16,152,328,22 | `BS_AUTOCHECKBOX`; `ShowWithStart` |
| STATIC "Save pictures (Win+S) to folder" | – | 16,186,328,20 | |
| EDIT (client edge, `ES_AUTOHSCROLL`) | 105 | 16,208,240,24 | `PicturesDir`; cue banner (`EM_SETCUEBANNER`) = `default_dir()` |
| Button "Browse…" | 106 | 262,207,82,26 | folder picker |
| STATIC "Position" | – | 16,242,260,20 | |
| Radio "Right side of the screen" | 101 | 24,264,300,22 | `WS_GROUP`, auto radio |
| Radio "Left side of the screen" | 102 | 24,288,300,22 | auto radio |
| Button OK (default) | IDOK | 168,322,84,28 | `WS_GROUP` |
| Button Cancel | IDCANCEL | 260,322,84,28 | |

- Focus starts on the trackbar. Tab moves between controls, Enter = OK, Esc = Cancel (through `IsDialogMessageW` in the main loop).
- **OK**:
  - width = trackbar position (5–50);
  - side = Right if the right radio is checked, else Left;
  - count = the edit text parsed as a number (10 if invalid), clamped to 1–100;
  - delay = the delay edit parsed as a number (0 if invalid), at most 2000;
  - show with Start = checkbox state;
  - pictures folder = `clean_dir(text)`: trimmed, surrounding `"` removed. It must be empty or an absolute path (`D:\x`, `\\server\share\x`). Otherwise a warning box ("Please enter a full folder path …") is shown, focus moves to the box, and the window stays open;
  - then `app::apply_settings` (section 8) and close.
- **Browse…**:
  1. COM `IFileOpenDialog` (`CoInitializeEx` STA on the main thread), options `FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST`, title "Folder for saved pictures", owner = the settings window.
  2. It starts in the folder from the box (or `default_dir()`), or the nearest existing parent of it (`SHCreateItemFromParsingName` + `SetFolder`).
  3. On OK, the chosen path (`SIGDN_FILESYSPATH`) is put in the box. Cancel changes nothing.
- **Cancel** / close button: close without changes.
- `WM_DESTROY` frees the font and the control state.

## 15. Tray (`tray.rs`)

- `Shell_NotifyIconW(NIM_ADD)`: uID 1, callback `WM_APP_TRAY`, tooltip `Unshufflz (hold Win: V back, C forward, B delete, S save image)`.
- Icon: drawn in code at `SM_CXSMICON` size; ARGB DIB + 1-bit mask → `CreateIconIndirect`. Shapes, as fractions of the size (x0, y0, x1, y1), checked in this order:
  - clip `#505050`: 0.34, 0.03, 0.66, 0.20
  - paper `#FFFFFF`: 0.25, 0.25, 0.75, 0.86
  - board `#0078D7`: 0.13, 0.10, 0.87, 0.97
  - everything else transparent.
  
  If that fails, `IDI_APPLICATION` is used.
- Menu (right click: `WM_RBUTTONUP` or `WM_CONTEXTMENU`):
  - Settings… (4)
  - Clear history (1)
  - Start with Windows (2, checked when enabled)
  - separator
  - Exit (3)
  
  It uses `SetForegroundWindow` + `TrackPopupMenu(TPM_RETURNCMD | TPM_RIGHTBUTTON)`, then posts `WM_NULL`.
- Autostart: `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, value `Unshufflz` = `"<exe path>"`. Enabled = the value exists.

## 16. Installer (`installer/`)

One exe, `unshufflz-setup.exe` (shipped as `dist\UnshufflzSetup-<version>.exe`). It contains the app (`include_bytes!(env!("APP_EXE"))`, i.e. `target\release\unshufflz.exe`). `build.rs` fails with a clear message if the app has not been built. `APP_VERSION` is the first `version = "…"` line in the root `Cargo.toml`.

### 16.1 Paths (per user, no admin)

| Item | Path |
|---|---|
| Install folder | `%LOCALAPPDATA%\Programs\Unshufflz\` |
| App | `…\unshufflz.exe` |
| Uninstaller | `…\uninstall.exe` (a copy of the setup exe) |
| Start menu shortcut | `%APPDATA%\Microsoft\Windows\Start Menu\Programs\Unshufflz.lnk` |
| User data (history, settings) | `%LOCALAPPDATA%\Unshufflz\` |
| Uninstall registry key | `HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\Unshufflz` |
| Autostart | `HKCU\…\Run`, value `Unshufflz` |

### 16.2 Arguments

Case-insensitive; `/` or `-` prefix; unknown arguments are ignored.

| Argument | Meaning |
|---|---|
| `/uninstall` | Uninstall instead of install |
| `/S` or `/silent` | No dialogs |
| `/noautostart` | Install without autostart (the silent default is on) |
| `/purge` | Uninstall also deletes user data (silent default: keep) |
| `/tempcopy` | Internal: this is the temp copy started by uninstall.exe |

Dialogs are `MessageBoxW` with the title "Unshufflz Setup".

### 16.3 Install

1. If not silent, ask:
   - "Install Unshufflz {ver} for the current user? Folder: …" (or "Update Unshufflz to version {ver}?" if `unshufflz.exe` already exists). No → exit.
   - "Start Unshufflz automatically when you sign in to Windows?"
2. **Stop the running app**:
   1. `FindWindowW("UnshufflzOverlay")`, then `OpenProcess(SYNCHRONIZE)`.
   2. Post `WM_CLOSE`; the app saves pending history and exits.
   3. Wait up to 5 s with `WaitForSingleObject`.
   4. Repeat up to 5 times while a window is found.
3. Create the install folder.
4. Write the app: `unshufflz.new`, then rename over `unshufflz.exe`, retrying up to 50 × 100 ms.
5. Copy the setup exe to `uninstall.exe` the same way (skipped if setup already runs as that file).
6. Create the shortcut through COM:
   1. `CoInitializeEx(APARTMENTTHREADED)`, then `CoCreateInstance(ShellLink)` for an `IShellLinkW`.
   2. `SetPath` = app, `SetWorkingDirectory` = install folder, `SetDescription` = "Clipboard history overlay (hold the Win key)".
   3. Save through `IPersistFile::Save`.
   
   Failure is not fatal: the user gets a warning (unless silent).
7. Uninstall key values:

| Value | Type | Content |
|---|---|---|
| DisplayName | REG_SZ | Unshufflz |
| DisplayVersion | REG_SZ | version |
| Publisher | REG_SZ | Unshufflz |
| DisplayIcon | REG_SZ | app path |
| InstallLocation | REG_SZ | install folder |
| UninstallString | REG_SZ | `"<uninstall.exe>" /uninstall` |
| QuietUninstallString | REG_SZ | `"<uninstall.exe>" /uninstall /S` |
| NoModify, NoRepair | REG_DWORD | 1 |
| EstimatedSize | REG_DWORD | (app + setup size) / 1024 |

8. Autostart: set the Run value to `"<app path>"`, or delete it.
9. Start the app.
10. If not silent, show "installed and running" with a short usage hint.

Errors show "Setup failed: …" (unless silent) and set exit code 1. User data is never touched by install or update.

### 16.4 Uninstall

1. If running from inside the install folder and not `/tempcopy`:
   1. Copy self to `%TEMP%\unshufflz-uninstall-<pid>.exe`.
   2. Start that copy with `/uninstall /tempcopy` (plus `/S` and `/purge` if given).
   3. Exit. A running exe cannot delete its own folder, so the copy does the work.
2. If not silent, ask "Remove Unshufflz from this computer?" (No → exit), then "Also delete your saved clipboard history and settings?" (default No).
3. Stop the running app (as in install).
4. Delete the Run value, the uninstall key (`RegDeleteKeyW`) and the shortcut.
5. Delete the install folder, retrying 50 × 100 ms while the original uninstall.exe exits.
6. If purge: delete `%LOCALAPPDATA%\Unshufflz`.
7. If not silent, show "Unshufflz was removed."
8. As `/tempcopy`, always (also on No or error), it deletes itself: `cmd /c ping 127.0.0.1 -n 3 > nul & del /f /q "<self>"` with `CREATE_NO_WINDOW`.

## 17. Build

- `build.ps1`:
  1. Refuses to run while a copy of the app from `target\` is running.
  2. Runs `cargo test --release`, `cargo build --release`, `cargo build --release -p unshufflz-setup`, `cargo test --release -p unshufflz-setup`, stopping on any failure.
  3. Copies the installer to `dist\UnshufflzSetup-<version>.exe`.
- A plain `cargo build` builds only the app (`default-members`). The installer must be built after the app.

## 17a. Debug log (`util::debug`)

- On only when the environment variable `UNSHUFFLZ_DEBUG` is `1` (read once, `OnceLock`).
- Each call appends `"<GetTickCount64 ms> <thread id> <message>\r\n"` to `%LOCALAPPDATA%\Unshufflz\debug.log`. The message is built only when logging is on.
- Logged:
  - hook events (vk, down/up, event time, flags, phase before → after, output), but only when the phase before or after is not Idle, so normal typing is never written;
  - main-thread actions;
  - the moment the overlay is shown;
  - slide-in frame count and duration;
  - Start menu open/close.

## 18. Tests

`cargo test --release` (app, 50 tests):

| Module | Tests |
|---|---|
| hook | missed Win up: a Win down 10 s later is a new press, while real auto-repeat (every 33 ms for 2.5 s) is still ignored; quick tap; quick tap with the overlay already shown → TAP and Win up passes (Start opens), also across tick-counter wrap; a quick V is not a tap; hold → SHOW, release → COMMIT + inject; V/C navigation and swallowing; B → SHOW + DELETE, swallowed, Start menu masked; S taken only with an image and no modifiers (Win+S Search and Win+Shift+S pass); Win+E passthrough; other key while shown → CANCEL; modifiers; Esc cancel + mask; Esc before overlay; C/V without Win |
| store | item round trip; bit flip / truncation / empty rejected; index round trip + cleanup; broken index → scan; writer thread + double shutdown; **1000 items with a limit of 10 → exactly 10 item files + index on disk, newest 10 loaded in order** |
| preview | DROPFILES parsing; text / RTF / files / image previews; thumbnail scaling; small image not enlarged; bad DIBs rejected |
| history | duplicate key with extra formats; registered formats hashed by name; different text; `keep_visible` scrolling |
| config | defaults and overrides incl. Position and MaxItems; clamping; format → parse round trip incl. PicturesDir, ShowWithStart and AnimationMs; AnimationMs default 50, 0 allowed, capped at 1000 |
| anim | curve: ends at 0 and 1, monotonic, ease-out (> 0.5 at t = 0.25), linear curve stays linear; offsets: full width at start, 0 at and after the end, 0 when the duration is 0, mid-values in range |
| paint | `compose`: settled frame gets uniform premultiplied alpha (255 and 128); shift right / left with a transparent gap; fully shifted out = all transparent |
| settings | folder box: empty OK, quotes removed, UNC OK, relative and drive-relative (`D:pics`) rejected |
| imagesave | `target_dir` (empty → default `…\Clipboard`, else the setting); 24-bit bottom-up conversion; 32-bit alpha rules (all-zero → opaque, real alpha kept); 8-bit palette through GDI; broken / compressed DIBs rejected; PNG round trip (RGBA, and RGB when opaque); file names incl. `_2`; saving twice in one second makes two files; a `PNG` format is written as-is; file payload parses back to the path |
| util | sizes; UTF-16 helpers; time format |
| drag | the shell data object for two files in different folders offers CF_HDROP (guards against the empty-object `pidlFolder = NULL` mistake) |

`cargo test --release -p unshufflz-setup` (4 tests): argument parsing; install paths; path comparison helpers; the embedded app starts with `MZ`.

Manual checks done:
- capture of text and images;
- saving and reloading after restart;
- memory: about 1.7 MB private when empty, about 1.9 MB with 10 items;
- simulated Win hold + V presses: overlay shown, clipboard went back 2 items;
- Win+S (simulated) with a 1920×1080 image: PNG saved to `OneDrive\Pictures\Clipboard`, 1920×1080, about 240 ms; clipboard afterwards held the file (file-drop list) and no image. The test file was deleted.
- Zero delay (simulated): overlay visible 7 ms after Win down; quick tap (~100 ms) hid it and opened Start; 600 ms hold without keys did not open Start.
- Settings window with the new delay field: screenshot checked; OK wrote `HoldDelayMs=0`.
- Pictures folder setting:
  - screenshot shows the grey default-folder hint;
  - Browse… opened the Windows folder picker at `Pictures\Clipboard`;
  - a relative path showed the warning and kept the window open;
  - a quoted path was saved without quotes as `PicturesDir`;
  - Win+S then saved `Unshufflz_….png` into that folder, and the clipboard held that file;
  - the setting was set back to empty afterwards.
- Slide-in, after the change to `UpdateLayeredWindow`:
  - slow motion (`AnimationMs=1000`): screenshots at 150 / 300 / 600 / 1200 ms show the accent bar at the moving edge (x = 553 → 360 → 284 → 269), eased; 53 frames;
  - real (`50`): 3 frames, done after 59–61 ms;
  - the settled overlay looks as before.
- Start menu timing for comparison (Windows 11): focus 9–31 ms after Win release; its slide-up ends 235–255 ms after release (screen sampled about every 18 ms).
- ShowWithStart mode (simulated keys, debug log on):
  1. tap → Start open and overlay shown;
  2. Win+V+V → overlay shown;
  3. release → overlay hidden, clipboard 2 items back, Start still open;
  4. Win press with Start open → no overlay at any point, and Start closed;
  5. tap → Start and overlay;
  6. Esc → both closed.

  A screenshot showed the overlay at the right edge, next to the open Start menu.
- Debug log: a run without filtering recorded ordinary typing; it was deleted, and logging was limited to keys while Win is held.
- Win+B (simulated): clipboard went from "del B" back to "del A", and the item was removed from the index;
- settings window: opened by a second launch; screenshot checked (themed controls); OK wrote `WidthPercent=20`, `Position=left`, `MaxItems=25` to the INI, and the values were set back afterwards.

Not yet checked by hand:
- a real install / uninstall (the automatic run was blocked by a permission rule because it turns on autostart);
- the overlay on the left side, and with more than 10 items;
- image and file previews on screen;
- multiple monitors;
- admin windows.

## 19. Known limits

- Win+V / Win+C / Win+B system shortcuts are replaced while Win is held; Win+S (Search) only while the current item is an image.
- Win+S works on the current clipboard item only, not on the item highlighted in the overlay.
- Admin windows give no keys to the hook unless the app runs as admin.
- Stored data is not encrypted.
- File items store paths only.
- Dragging files out of the overlay offers copy and link only, never move.
- Thumbnails ignore alpha. RLE / JPEG / PNG-compressed DIBs get no thumbnail but are still stored and restored.
- If the hook misses a Win up, timer 2 hides the overlay; the next Win press is recognised as new by the stale-press rule (6.3, rule 0).
- ShowWithStart: the Windows Search panel shares SearchHost.exe with Start on newer Windows 11, so the overlay also shows with Search. If Start is set to open on the left edge and the overlay is on the left, Start covers it (Start is drawn in a higher layer than topmost windows).
- The settings window does not re-layout when moved to a monitor with a different DPI (it is sized for the DPI it opened on).
