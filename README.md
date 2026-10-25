# Unshufflz

A small clipboard history manager for Windows 10/11, similar to Clipboarder from 8GadgetPack.
It keeps the last 10 clipboard items (text, rich text, images, files and any other format).
While you hold the Win key, it shows them in an overlay on the right side of the screen.

- A single 340 KB exe. It needs no runtime, browser engine or JavaScript.
- It uses about 2 MB of private RAM, and 6 threads.
- History is saved to disk and survives restarts and power loss.

The full technical spec (enough to rebuild the app) is in [specs.md](specs.md).

## Usage

Start `unshufflz.exe`. It runs in the background and adds a tray icon.

| Keys | Result |
|---|---|
| Hold **Win** (250 ms) | Overlay appears with the current clipboard item |
| Hold Win + **V** | Go 1 step back in history (older item) |
| Hold Win + **C** | Go 1 step forward in history (newer item) |
| Release **Win** | The selected item becomes the clipboard content and moves to the top of the list. If you did not move, the clipboard stays as it is. Nothing is pasted automatically, so press Ctrl+V yourself. |
| Hold Win + **Esc** | Close the overlay without changing anything |
| Hold Win + other key (Win+E, Win+D, …) | Overlay closes and the normal Windows shortcut runs |
| Quick Win tap | Start menu opens as usual |

The overlay shows:
- **Header:** the position (`3 / 10`), the type and size of the content, when it was copied, and from which app.
- **Preview:**
  - Text: the wrapped text.
  - Image: the picture and its size.
  - Files: names and sizes.
  - Other formats: a list of formats.
- **List:** all stored items, one line each, with the selected item highlighted.

Tray icon menu (right click):
- **Clear history**: deletes all stored items from memory and disk.
- **Start with Windows**: on/off.
- **Exit**.

## Install

1. Build it (see below) or take `target\release\unshufflz.exe`.
2. Copy the exe anywhere, e.g. `%LOCALAPPDATA%\Programs\Unshufflz\`.
3. Run it, then tick **Start with Windows** in the tray menu if you want autostart.

To remove it:
1. Untick **Start with Windows**, then Exit.
2. Delete the exe and the `%LOCALAPPDATA%\Unshufflz` folder.

## Settings

Optional file `unshufflz.ini` next to the exe. All keys are optional:

```ini
[General]
HoldDelayMs=250   ; 50..2000  how long Win must be held before the overlay shows
WidthPercent=15   ; 5..50     overlay width as % of the monitor width
MaxItemMB=200     ; 1..2048   clipboard content bigger than this is not stored
```

Restart the app after changing it.

## Where data is stored

`%LOCALAPPDATA%\Unshufflz\store\`: one `.item` file per history item plus `index.dat`.
Files are written safely: a crash or power cut never leaves a broken file.
**The data is not encrypted.** Anyone with access to your user profile can read what you copied.

## Limitations

- While Win is held, **C** and **V** belong to Unshufflz. This blocks the built-in Win+V (Windows clipboard history) and Win+C (Copilot).
- Windows does not pass keys from admin (elevated) windows to normal apps. To use the overlay over admin windows, run Unshufflz as admin.
- Content marked private by password managers is not stored. Windows marks it with `ExcludeClipboardContentFromMonitorProcessing` or `CanIncludeInClipboardHistory=0`.
- File items store the file paths, not the file contents. Pasting them later needs the files to still exist.
- Only 10 items are kept.

## Build

Requirements:
- Rust (stable, `x86_64-pc-windows-msvc`). Install with `winget install Rustlang.Rustup`.
- MSVC Build Tools (C++ workload).

```
cargo build --release      # -> target\release\unshufflz.exe
cargo test --release       # unit tests (29)
```

The exe must not be running while you build, or the build cannot overwrite it. Use tray → Exit first.

## Troubleshooting

| Problem | Cause / fix |
|---|---|
| Overlay doesn't show over some window | That window runs as admin. Run Unshufflz as admin too. |
| Start menu opens after using the overlay | Should not happen. If it does, another keyboard tool may be interfering. |
| An item is missing from history | It was bigger than `MaxItemMB`, marked private, or the clipboard was locked by another app. |
| Second copy does not start | Only one copy can run (by design). |
