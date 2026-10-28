# Handy Shortcut

Turns the "hold to talk" button on the **teenage engineering handy
microphone** into a keyboard shortcut for your PC, such as the Transcribe
Shortcut in the [Handy](https://handy.computer) speech-to-text app. Hold the
button on the mic, talk, let go, and Handy transcribes.

(Despite the shared name, the handy microphone and the Handy app are
unrelated; this app connects the two.)

## How it works

The handy microphone's hold-to-talk button doesn't send anything the computer
can see; it works inside the mic. But every press and release puts a loud
"thump" on the audio signal, and the two point in opposite directions (a press
jumps up first, a release jumps down first). The app listens for those thumps
and holds the shortcut from a press until the next release, or presses it
once (see **Button action** below). Only spikes shaped like a button thump
count (near full scale, about 2-3 ms wide), so speech and plosives don't
trigger it.

## Compatibility

- **Tested with:** the teenage engineering handy microphone, connected through
  an Audient EVO4 audio interface (input "Line 2"), on Windows 11.
- The thumps measured there peak at full scale (1.00) and last about 2.7-2.8 ms.
  The default thresholds (min peak 0.9, min width 2.3 ms) are tuned for that;
  speech spikes on the same setup reached 2.0 ms.
- Other interfaces should work as long as the thump reaches the app
  unprocessed. Watch the Activity log while pressing the button: if real
  presses show up as "ignored", lower **Min peak** or **Min width** to fit
  the numbers you see.
- Other mics with a mute or push-to-talk switch may work too if their switch
  makes a similar thump, but they haven't been tried.

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
- **Button action** picks what the shortcut does:
  - *Hold while button is down* (default): the shortcut is held from button
    press to release, for push-to-talk.
  - *Press on button down and up*: the shortcut is pressed once when the
    button goes down and once when it comes up, for apps that toggle (like
    Handy's toggle mode).
  - *Press on button down only*: the shortcut is pressed once per button press.
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
