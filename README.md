# Unshufflz

A small clipboard history manager for Windows 10/11, similar to Clipboarder from 8GadgetPack.
It keeps the last 10 clipboard items (text, rich text, images, files, any other format)
and shows them in an overlay on the right side of the screen while the Win key is held.

This file is the full spec. It should be enough to rebuild the app if the code is lost.

## Usage

| Action | Result |
|---|---|
| Hold **Win** for 250 ms | Overlay appears and shows the current clipboard item |
| Hold Win, press **V** | Go 1 step back in history (older) |
| Hold Win, press **C** | Go 1 step forward in history (newer) |
| Release **Win** | Selected item is put on the clipboard and moved to the top of history. If nothing was changed, the clipboard stays as it is. No automatic paste. |
| Hold Win, press **Esc** | Close the overlay without changing the clipboard |
| Hold Win, press any other key (Win+E, Win+D…) | Overlay closes, the normal Windows shortcut runs |
| Quick Win tap | Start menu opens as normal |

Tray icon (right click): **Clear history**, **Start with Windows** (checkbox), **Exit**.

Notes:
- While the overlay is in use, the built-in Win+V (Windows clipboard history) and Win+C (Copilot) are replaced: any C or V pressed with Win held goes to Unshufflz.
- Windows does not send key events from elevated (admin) windows to a non-admin hook. To use the overlay over admin windows, run Unshufflz as admin.
- Only one copy runs at a time (named mutex `Local\UnshufflzSingleInstance`).

## Build and test

- Requires the Rust toolchain (`stable-x86_64-pc-windows-msvc`) and MSVC Build Tools.
- `cargo build --release` → `target\release\unshufflz.exe` (about 340 KB, single file, no runtime needed).
- `cargo test --release` runs the unit tests.
- Dependencies: `windows-sys` 0.59.0 (raw Win32 API) and `xxhash-rust` (xxh3 hashing).
- Release profile: LTO, `codegen-units=1`, `panic=abort`, stripped.

Measured with 10 items in history (including a 1920×1080 image): about 2 MB private memory and 6 threads.

## Settings

Optional `unshufflz.ini` next to the exe. All keys are optional; section headers are ignored; key names are case-insensitive.

```
HoldDelayMs=250   ; 50..2000  time Win must be held before the overlay shows
WidthPercent=15   ; 5..50     overlay width as % of the monitor width
MaxItemMB=200     ; 1..2048   clipboard contents bigger than this are not stored
```

## Architecture

Pure Win32 + GDI, no UI framework. Code is in `src/`:

| File | Role |
|---|---|
| `main.rs` | Single-instance mutex, per-monitor DPI awareness v2 (`SetProcessDpiAwarenessContext`), starts the app |
| `app.rs` | Main window (the overlay), message loop, history logic, clipboard listener, commit/cancel |
| `hook.rs` | Low-level keyboard hook on its own thread; pure state machine `step()` + side effects |
| `clip.rs` | Clipboard capture and restore |
| `history.rs` | `Item` struct, hashing (`full_hash`, `content_key`), limits |
| `preview.rs` | Builds the preview for each item once (text, thumbnail, file list) |
| `paint.rs` | GDI drawing of the overlay with double buffering |
| `store.rs` | Disk storage with safe writes, background writer thread |
| `tray.rs` | Tray icon (drawn in code), menu, autostart registry value |
| `config.rs` | INI parsing |
| `util.rs` | UTF-16 helpers, size and time formatting, paths |

### Threads

1. **Main thread**: overlay window, clipboard listener, timers, tray, all history state.
2. **Hook thread**: `WH_KEYBOARD_LL` hook and its message loop. It only decides and posts `WM_APP+1` (wParam = action) to the main window, so it stays fast (Windows removes slow hooks).
3. **Writer thread**: writes item files and the index. It posts `WM_APP+2` (lParam = item id) to the main window when an item file is on disk.

### Keyboard state machine (`hook.rs`)

Phases: `Idle`, `Armed`, `Shown`, `Cancelled`, `Passthrough`. Win = LWin (0x5B) or RWin (0x5C); the Win key that started the sequence is remembered.

- `Idle` + Win down → `Armed`, start hold timer (thread timer, `HoldDelayMs`). Win down always passes through.
- Hold timer fires in `Armed` → `Shown`, post SHOW.
- C/V down in `Armed`/`Shown`/`Cancelled` → swallow; if not `Shown`, become `Shown` and post SHOW; then post PREV (V) or NEXT (C). C/V up is swallowed too.
- Esc in `Shown` → swallow, post CANCEL, become `Cancelled`. Esc in `Cancelled` is swallowed.
- Shift/Ctrl/Alt (0x10–0x12, 0xA0–0xA5) and the other Win key → pass, no change.
- Any other key down in `Armed`/`Shown` → pass, post CANCEL if `Shown`, become `Passthrough`.
- Repeated Win down (auto-repeat) → ignored.
- Win up:
  - in `Shown` → post COMMIT, swallow it, then inject: mask key 0xE8 down/up + Win up. The mask key stops the Start menu from opening.
  - in `Cancelled` → the same, without COMMIT.
  - otherwise → pass (a quick tap opens Start).
- Injected keys carry `dwExtraInfo = 0x434C4950` and are ignored by the hook.

Main window actions: SHOW (show overlay with selection 0 = newest), PREV (selection +1, limited to the oldest), NEXT (selection −1, limited to 0), COMMIT (hide; if selection > 0, restore that item and move it to the top), CANCEL (hide).
Safety net: while visible, a 150 ms timer hides the overlay if neither Win key is down (`GetAsyncKeyState`).

### Clipboard capture (`clip.rs`)

- `AddClipboardFormatListener`; on `WM_CLIPBOARDUPDATE` an 80 ms timer collects bursts into one read.
- Skip if `GetClipboardSequenceNumber()` equals the number saved right after our own restore.
- `OpenClipboard` is retried 10 × 15 ms.
- Privacy: skip if the format `ExcludeClipboardContentFromMonitorProcessing` or `Clipboard Viewer Ignore` is present, or `CanIncludeInClipboardHistory` holds DWORD 0 (password managers use these).
- All formats from `EnumClipboardFormats` are stored, except:
  - GDI handles and synthesized formats: CF_BITMAP, CF_METAFILEPICT, CF_PALETTE; CF_TEXT/CF_OEMTEXT when CF_UNICODETEXT exists; CF_DIB when CF_DIBV5 exists. Windows creates these again on paste.
  - Owner-display 0x80–0x8F, private/GDI-object ranges 0x200–0x3FF.
  - Registered formats `DataObject` and `Ole Private Data` (OLE pointers that are invalid later).
- CF_ENHMETAFILE is stored as bytes (`GetEnhMetaFileBits`) and restored with `SetEnhMetaFileBits`. Everything else is HGLOBAL memory copied as bytes.
- Registered formats (id ≥ 0xC000) are stored by name and registered again on restore, because their ids change between sessions.
- If the total size is over `MaxItemMB`, the item is not stored.
- Source app: exe name of `GetClipboardOwner()`'s process (falls back to the foreground window).
- Restore: `EmptyClipboard`, then `SetClipboardData` for each format, then save the new sequence number to ignore our own update.

### History (`app.rs`, `history.rs`)

- Up to 10 items, newest first.
- `id` = xxh3 of all formats (format id or name + length + data). It is also the file name on disk.
- `key` = xxh3 of the main content only: CF_HDROP, else CF_DIBV5, else CF_DIB, else CF_UNICODETEXT, else all formats.
- New capture: if `id` equals the newest item's id → ignore. Otherwise remove any item with the same `key` (a duplicate, possibly with different extra formats) and insert the new one at the top.
- Restoring an item keeps its original time.
- Items larger than 1 MB drop their data from RAM once the writer has saved them, and load it from disk on restore. If that file is missing or broken: beep and remove the item.

### Preview (`preview.rs`)

Built once per item (on capture or on load):
- **Files** (CF_HDROP): file name and size of each file (or "folder" / "missing"), up to 500 entries; header "Files · N files, M folders · total size"; parent folder line.
- **Text** (CF_UNICODETEXT): the first 4000 UTF-16 characters; header shows the character and line count. It is labelled "Rich text (RTF/HTML)" if `Rich Text Format` or `HTML Format` is present.
- **Image** (CF_DIBV5 or CF_DIB): header "Image · W × H · bpp"; a thumbnail at most (15% of the primary screen width − 32) px wide and twice that tall, never enlarged, made with `StretchDIBits` + HALFTONE. Only uncompressed DIBs (BI_RGB, BI_BITFIELDS) with pixel data fully inside the buffer get a thumbnail.
- **Other**: list of format names and sizes.
- Order of checks: Files, Text, Image, Other. So Excel cells (text + bitmap) count as text.
- A one-line summary is used in the history list.

### Overlay (`app.rs`, `paint.rs`)

- Window styles: `WS_POPUP`; extended `TOPMOST | TOOLWINDOW | NOACTIVATE | LAYERED | TRANSPARENT`; alpha 245. It never takes focus and ignores mouse clicks.
- Position: monitor under the mouse cursor; width = `WidthPercent` of the monitor width (at least 200 px); full height of the work area; aligned to the right edge.
- Drawing (scaled by window DPI, Segoe UI): dark background `#202020`, 3 px blue bar on the left (`#0078D7`).
  - Header: "Clipboard" and "i / n"; info line; "time · source.exe" (time is HH:MM:SS for today, otherwise DD.MM.YYYY HH:MM).
  - Preview area: wrapped text, centered thumbnail, or file rows (name left, size right, "+N more").
  - Bottom: the list of all items (number + summary line); the selected row is highlighted `#104070`.

### Storage (`store.rs`)

Folder: `%LOCALAPPDATA%\Unshufflz\store\`.

- `<id as 16 hex digits>.item`: `"CLPI"`, u32 version=1, u32 count, then for each format: u32 id, u16 name length, name (UTF-8), u64 data length, data; then u64 xxh3 of everything before.
- `index.dat`: `"CLPX"`, u32 version=1, u32 count, then for each item (newest first): u64 id, u64 time (unix seconds), u16 length, source exe (UTF-8); then u64 xxh3.
- All numbers are little-endian.

Safe writes:
1. Write to `*.tmp`, `FlushFileBuffers` (`sync_all`), then `MoveFileExW(REPLACE_EXISTING | WRITE_THROUGH)`. After a crash or power loss there is either the old file or the new one, never a half-written one. A checksum check catches anything else.
2. Item files never change once written (the name is the content hash). If the file already exists, it is not written again.
3. The writer handles jobs in order: an item's file is always written before the index that lists it. When several index jobs are queued, only the newest is written.
4. After each index write, item files not in the index and leftover `*.tmp` files are deleted.

Loading: read `index.dat`; if missing or broken, scan `*.item` files and order them by modified time. Items whose file is missing or broken are skipped.
On exit (tray Exit or Windows shutdown via `WM_ENDSESSION`), the writer finishes its queue before the process ends.

### Autostart

`HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, value `Unshufflz` = `"<full exe path>"`.

## Tests

Unit tests (`cargo test`) cover:
- keyboard state machine: quick tap, hold, V/C navigation, other shortcuts, modifiers, Esc, keys without Win;
- storage: item/index round trip, checksum and truncation detection, removal of unused files, fallback when the index is broken, writer thread;
- previews: DROPFILES parsing, text/rich text/file/image previews, thumbnail scaling, rejection of truncated or compressed DIBs;
- duplicate detection, INI parsing and limits, size/time formatting.

Manual checks done: capture of text and images, saving and reloading after restart, memory use, simulated Win hold + V presses (the overlay showed and the clipboard went back 2 items).
