# Unshufflz

**Clipboard manager you secretly dreamt of. Built in Rust for 10x engineers (100x, 1Kx, 1Mx too).**

A small clipboard history manager for Windows 10/11, similar to Clipboarder from 8GadgetPack.
It keeps your recent clipboard items (text, rich text, images, files and any other format).
While you hold the Win key, it shows them in an overlay on the side of the screen.

- A single 500 KB exe. It needs no runtime, browser engine or JavaScript.
- It uses about 2 MB of private RAM, and 6 threads.
- History is saved to disk and survives restarts and power loss.
- You can change the history size (1–100 items), the overlay width and its side in a settings window.

The full technical spec (enough to rebuild the app) is in [specs.md](specs.md).

## Install

Run `UnshufflzSetup-<version>.exe` and answer the two questions (install? start with Windows?).

- It installs only for the current user. No admin rights are needed.
- The app goes to `%LOCALAPPDATA%\Programs\Unshufflz\` (`unshufflz.exe` and `uninstall.exe`).
- It adds a Start menu shortcut **Unshufflz**.
- It adds an entry in **Settings → Apps → Installed apps**, used to uninstall.
- It can set the app to start when you sign in.
- When setup finishes, the app is running.

Running setup again updates an existing install. It closes the running app first; your history and settings are kept.

### Uninstall

Use **Settings → Apps → Installed apps → Unshufflz → Uninstall**, or run `uninstall.exe /uninstall`.
It closes the app and removes the program files, shortcut, autostart and the Apps entry.
It then asks whether to delete your saved history and settings too (default: keep).

### Command-line switches

| Command | Action |
|---|---|
| `UnshufflzSetup.exe` | Install with questions |
| `UnshufflzSetup.exe /S` | Silent install, start with Windows on |
| `UnshufflzSetup.exe /S /noautostart` | Silent install, start with Windows off |
| `uninstall.exe /uninstall` | Uninstall with questions |
| `uninstall.exe /uninstall /S` | Silent uninstall, keeps history and settings |
| `uninstall.exe /uninstall /S /purge` | Silent uninstall, deletes history and settings too |

## Usage

The app runs in the background and adds a tray icon.

| Keys | Result |
|---|---|
| Hold **Win** | Overlay appears at once with the current clipboard item highlighted, sliding in from the screen edge in 50 ms (Apple-style ease-out). A delay can be set in Settings. |
| Hold Win + **V** | Go 1 step back in history (older item) |
| Hold Win + **C** | Go 1 step forward in history (newer item) |
| Hold Win + **B** | Delete the highlighted item from history (and disk). If it was the current clipboard item, the next older item goes on the clipboard right away (or the clipboard is emptied if nothing is left). Press B again to delete more. |
| Release **Win** | The selected item becomes the clipboard content. It stays at its place in the list (the order does not change), and the next time the overlay opens on it. If you did not move, the clipboard stays as it is. Nothing is pasted automatically, so press Ctrl+V yourself. |
| Hold Win + **S** (current item is an image) | Save the image as PNG to `Pictures\Clipboard\Unshufflz_yyyyMMdd_HHmmss.png` (the folder can be changed in Settings). In history and on the clipboard, the image is replaced by the saved file, as if you had copied it in Explorer, so Ctrl+V pastes the file. If the current item is not an image, or Shift/Ctrl/Alt is held, Win+S does its normal Windows thing (Search; Win+Shift+S = snipping). |
| Hold Win, select a file item, then **drag its file panel with the mouse** | The files are dragged out like from Explorer: drop them into an Explorer window, the desktop, a browser upload, a chat app, etc. Press on one file name to drag only that file, or elsewhere in the panel to drag all of them. The drop always copies (or with Alt makes a shortcut); it never moves the original. You can release Win while dragging. |
| Hold Win + **Esc** | Close the overlay without changing anything |
| Hold Win + other key (Win+E, Win+D, …) | Overlay closes and the normal Windows shortcut runs |
| Quick Win tap (released within 300 ms, no other key) | The overlay flashes and hides, and the Start menu opens as usual. **This is expected and fine.** Holding Win longer and releasing it without pressing a key does not open Start. With **"Also show while the Start menu is open"** on, the overlay stays instead (see below). |

The overlay shows:
- **Header:** the position (`3 / 25`), the type and size of the content, when it was copied, and from which app.
- **Preview:**
  - Text: the wrapped text.
  - Image: the picture and its size.
  - Files: names and sizes. The panel can be dragged with the mouse (the cursor turns into a hand over it).
  - Other formats: a list of formats.
- **List:** the items, one line each, with the selected item highlighted. At most 10 rows are shown at once. With a larger history, the list scrolls as you move, so the selection stays visible (no scrollbar). The row numbers show where you are.

Tray icon menu (right click):
- **Settings…**
- **Clear history**: deletes all stored items from memory and disk.
- **Start with Windows**: on/off.
- **Exit**.

Tip: to paste a stack of copied items newest first, press Ctrl+V, then Win+V, and repeat. Each Win+V takes the next older item, because choosing an item does not move it in the list. Copying something new puts it at the top and starts over from there. Ctrl+V then Win+B works too, but it also deletes each pasted item.

Starting `unshufflz.exe` again (e.g. from the Start menu) while it is running opens the settings window.

## Settings

The **Settings** window (tray menu → Settings…) has:

| Setting | Values | Default |
|---|---|---|
| Overlay width | 5–50 % of the screen width (slider) | 15 % |
| Items to keep in history | 1–100 | 10 |
| Show delay when holding Win | 0–2000 ms | 0 (instant) |
| Also show while the Start menu is open | on / off | off |
| Save pictures (Win+S) to folder | any full folder path; **Browse…** opens the Windows folder picker | empty = `Pictures\Clipboard` (shown as a grey hint) |
| Position | Right side / Left side of the screen | Right |

Changes apply when you press **OK**.
- Making the history smaller removes the oldest items right away, from the list and from disk.
- The pictures folder must be a full path (e.g. `D:\Screenshots`; quotes from Explorer's "Copy as path" are removed). It is created on the first save if it does not exist. Leave it empty for the default.
- Settings are saved to `%LOCALAPPDATA%\Unshufflz\unshufflz.ini`.

The same file can be edited by hand while the app is not running. It has one more setting that is not in the window:

```ini
[General]
HoldDelayMs=0     ; 0..2000   how long Win must be held before the overlay shows (0 = instant)
WidthPercent=15   ; 5..50
Position=right    ; right | left
MaxItems=10       ; 1..100
MaxItemMB=200     ; 1..2048   clipboard content bigger than this is not stored
PicturesDir=      ; folder for Win+S pictures; empty = Pictures\Clipboard
ShowWithStart=0   ; 1 = also show the overlay while the Start menu is open
AnimationMs=50    ; 0..1000   slide-in duration; 0 = appear without animation
```

## Where data is stored

`%LOCALAPPDATA%\Unshufflz\`:
- `unshufflz.ini`: settings.
- `store\`: one `.item` file per history item, plus `index.dat`.

Only the items in the list are kept on disk. When an item drops out of the history (for example, after 1000 copies with a limit of 10), its file is deleted.
Files are written safely: a crash or power cut never leaves a broken file.
**The data is not encrypted.** Anyone with access to your user profile can read what you copied.

## Limitations

- With the default delay of 0, the overlay also flashes briefly when you use other Win shortcuts (Win+E, Win+D, …) and on a quick Win tap before Start opens. This is expected. Set a show delay (e.g. 250 ms) in Settings if you don't want it.
- While Win is held, **C**, **V** and **B** belong to Unshufflz. This blocks the built-in Win+V (Windows clipboard history), Win+C (Copilot) and Win+B (focus the tray icons). **S** is taken only when the current item is an image and no Shift/Ctrl/Alt is held, which blocks Windows Search in that case only.
- Win+S saves only the current clipboard item, not an older item you moved to in the overlay.
- Windows does not pass keys from admin (elevated) windows to normal apps. To use the overlay over admin windows, run Unshufflz as admin.
- Content marked private by password managers is not stored. Windows marks it with `ExcludeClipboardContentFromMonitorProcessing` or `CanIncludeInClipboardHistory=0`.
- File items store the file paths, not the file contents. Pasting them later needs the files to still exist.
- More items means more memory for image previews: about 1 MB at most per image item.

## Build

Requirements:
- Rust (stable, `x86_64-pc-windows-msvc`). Install with `winget install Rustlang.Rustup`.
- MSVC Build Tools (C++ workload).

```
powershell -ExecutionPolicy Bypass -File build.ps1
```

This runs all tests and builds the app and the installer. The installer goes to `dist\UnshufflzSetup-<version>.exe`.

To build by hand:

```
cargo test --release                         # app tests
cargo build --release                        # target\release\unshufflz.exe
cargo build --release -p unshufflz-setup   # target\release\unshufflz-setup.exe (embeds the app)
cargo test --release -p unshufflz-setup    # installer tests
```

The app must be built before the installer, because the installer contains it.
Exit any copy of the app running from `target\` first, or the build cannot overwrite it.
The version comes from the root `Cargo.toml`.

### Show with the Start menu

With **Also show while the Start menu is open** turned on, the overlay appears whenever the Start menu is open. That includes opening Start with a quick Win tap or with the mouse. It stays as long as Start is open:

| Step | Result |
|---|---|
| Press Win | Start opens **and** the overlay shows |
| Hold Win + V / C | Select an item (B and S work too) |
| Release Win | The selected item goes on the clipboard and the overlay hides. Start stays open. |
| Press Win again | Start closes; the overlay does not show |
| Press Win again | Start opens and the overlay shows again |
| Esc, or click elsewhere | Start closes and the overlay hides |

On newer Windows 11 versions, the Windows Search panel is the same program as Start, so the overlay shows with Search too.

### Start menu speed

The Windows Start menu itself is slower than the overlay. It gets focus 10–30 ms after Win is released, and its slide-up animation ends about 240 ms after release (measured on Windows 11). Windows has no setting for Start's speed alone. **Settings → Accessibility → Visual effects → Animation effects → Off** makes Start appear at once, but it turns off animations in all of Windows. The overlay's own animation is separate (`AnimationMs`).

## Troubleshooting

| Problem | Cause / fix |
|---|---|
| Overlay doesn't show over some window | That window runs as admin. Run Unshufflz as admin too. |
| Start menu opens after using the overlay | Should not happen. If it does, another keyboard tool may be interfering. |
| An item is missing from history | It was bigger than `MaxItemMB`, marked private, or the clipboard was locked by another app. |
| Second copy does not start | Only one copy runs (by design); starting it again opens the settings. |
| Overlay does not appear when expected | Start the app with a debug log (see below) and look at what it records. |

**Debug log**:
- Exit the app, then start it from a command prompt with the environment variable `UNSHUFFLZ_DEBUG=1`:
  ```
  set UNSHUFFLZ_DEBUG=1
  "%LOCALAPPDATA%\Programs\Unshufflz\unshufflz.exe"
  ```
- It writes `%LOCALAPPDATA%\Unshufflz\debug.log`, one line per event, with a millisecond timestamp:
  - key events while Win is held (normal typing is never logged);
  - what the app does: show, hide, actions;
  - when the Start menu opens and closes.
- Delete the file when done. Without the variable, nothing is logged.
