# Global hotkeys (`dawn-hotkeys`)

How the double-press shortcut (default Ctrl+C+C) is detected. How Owl-L uses it (popup, replace, smart target) is documented in the Owl-L repo, `docs/quick-access.md`.

## How it works

1. **`dawn-hotkeys`** installs a Windows low-level keyboard hook (`WH_KEYBOARD_LL`) on its own thread. The callback only copies the key event into a channel and calls `CallNextHookEx`; it never swallows a key, so a normal Ctrl+C keeps copying.
2. A **worker thread** runs one pure state machine per shortcut (`Detector`). It fires when the key is pressed twice inside the window (400 ms) with the modifiers held. Auto-repeat, another key in between, a late second press or the wrong modifiers start over; three quick presses fire once.

## Why a hook and the `windows` crate

Registering Ctrl+C with `RegisterHotKey` would take the key away from every other program. A low-level hook can observe keys instead. The official `windows` bindings are the smallest dependency that exposes it; the wrapper crates found either register hotkeys (taking the key) or do work inside the callback.

## Hook timeouts and safety

Windows waits for a low-level hook callback on every key press. If it takes longer than `LowLevelHooksTimeout` (a few hundred milliseconds at most) Windows skips it, and after repeated timeouts may drop the hook. The callback therefore does nothing but a channel send. If the process dies, Windows removes the hook by itself. On quit the hook is removed explicitly; with the shortcut disabled in Settings no hook is installed at all, and nothing runs until the shortcut is pressed.

## Limits

- Windows only. `dawn-hotkeys` has the same public functions as stubs on other systems.
- Running as administrator in the foreground app can stop a normal-privilege hook from seeing keys.


The app decides what to do after the second press (reading the clipboard, ignoring blocked programs, showing a popup); the crate only reports that the shortcut fired and which program was in the foreground.
