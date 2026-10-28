# Handy Shortcut

Turns the hardware "hold to talk" button on a microphone into a held keyboard
shortcut, such as [Handy](https://handy.computer)'s Transcribe Shortcut.

The computer can't see the button, but every press and release puts a loud
"thump" on the mic signal, and the two point in opposite directions (a press
jumps up first, a release jumps down first). The app listens for those thumps
and holds the shortcut from a press until the next release. Only spikes shaped
like a button thump count (near full scale, about 2-3 ms wide), so speech and
plosives don't trigger it.

This is a Rust rewrite of the earlier Python version (`handy_shortcut.py`).
It's a single 1 MB exe that needs no runtime and uses about 3 MB of private
memory. It reads the same settings file, so existing settings carry over.

## Using it

- Run `Handy Shortcut.exe`. It lives in the system tray: the icon is grey when
  stopped, green when listening, orange while the shortcut is held and red
  while it waits for the microphone (for example when it's unplugged; it retries
  every 3 seconds).
- Closing the window hides it to the tray. Left-click the tray icon to open it,
  and right-click for Listening on/off and **Quit**.
- **Record...** sets the shortcut: click it, then press the shortcut (Esc
  cancels). You can also type it, e.g. `ctrl+space` or `ctrl+shift+f9`.
- **Start with Windows** starts it in the tray when you log in.
- If presses come out as releases, tick **Swap press / release**.
- The Activity log shows every spike, including ignored ones, with its peak and
  width. Use these to tune **Min peak** and **Min width** if needed.

In Handy, set Shortcut Behavior to push-to-talk (hold) if offered. 'Auto' also
works, because the key is held for as long as the button is down.

Settings and the log are in `%APPDATA%\Handy Shortcut\`.

## Building

Needs [Rust](https://rustup.rs) and the Visual Studio C++ Build Tools:

```powershell
winget install Rustlang.Rustup
winget install Microsoft.VisualStudio.2022.BuildTools --override "--quiet --wait --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
```

Then:

```powershell
.\build.ps1          # -> dist\Handy Shortcut.exe
.\build.ps1 -Test    # run the unit tests first
```

## Code

| File | What it does |
| --- | --- |
| `src/detector.rs` | Finds button thumps in the audio (has unit tests) |
| `src/bridge.rs` | Opens the mic (WASAPI shared mode, so Handy can use it too) and turns thumps into key presses on a worker thread |
| `src/keys.rs` | Parses shortcuts and presses/releases them with `SendInput` |
| `src/app.rs` | Window, tray icon and menu |
| `src/settings.rs` | `settings.json` |
| `src/util.rs` | Start with Windows, single instance, local time |
| `src/art.rs`, `src/icons.rs`, `build.rs` | Draws the icons and embeds the exe icon and manifest |
