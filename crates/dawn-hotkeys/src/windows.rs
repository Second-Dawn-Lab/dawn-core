//! Windows: a low-level keyboard hook, the foreground app and the clipboard change counter.
//!
//! The hook only observes. `WH_KEYBOARD_LL` callbacks run on the thread that installed the hook,
//! inside the system's input path: every key press waits for the callback. Windows gives a slow
//! callback a limited time (`LowLevelHooksTimeout`, a few hundred milliseconds at most) and then
//! skips it, and after repeated timeouts may remove the hook. So the callback here only copies
//! the event into a channel and calls `CallNextHookEx`; the detector, the clipboard and the UI run
//! on other threads. The hook is removed when `Hotkeys` is dropped, and Windows removes it by
//! itself if the process dies, so a crash cannot leave input slowed down.

use std::cell::RefCell;
use std::io;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};

use windows::Win32::Foundation::{CloseHandle, HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::DataExchange::GetClipboardSequenceNumber;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{
    GetCurrentThreadId, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, VIRTUAL_KEY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetCursorPos, GetForegroundWindow, GetMessageW, GetWindowThreadProcessId, IsWindow, PostThreadMessageW,
    SetForegroundWindow, ShowWindow, SW_RESTORE, SetWindowsHookExW, UnhookWindowsHookEx, KBDLLHOOKSTRUCT, MSG, WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP, WM_QUIT,
    WM_SYSKEYDOWN, WM_SYSKEYUP,
};
use windows::core::PWSTR;

use crate::detector::{KeyEvent, Outcome, DEFAULT_WINDOW_MS};
use crate::engine::Engine;
use crate::shortcut::Shortcut;

enum Msg {
    Key(KeyEvent),
    Configure(Vec<(String, Shortcut)>),
    Quit,
}

thread_local! {
    /// Where the hook callback sends events. Only the hook thread's callback uses it.
    static SINK: RefCell<Option<Sender<Msg>>> = const { RefCell::new(None) };
}

unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // HC_ACTION == 0. Anything else must be passed on untouched.
    if code == 0 {
        // SAFETY: for WH_KEYBOARD_LL with HC_ACTION, lparam points to a KBDLLHOOKSTRUCT.
        let info = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        let down = match wparam.0 as u32 {
            WM_KEYDOWN | WM_SYSKEYDOWN => Some(true),
            WM_KEYUP | WM_SYSKEYUP => Some(false),
            _ => None,
        };
        if let Some(down) = down {
            // try_with: never panic inside a system callback.
            let _ = SINK.try_with(|sink| {
                if let Some(tx) = sink.borrow().as_ref() {
                    let _ = tx.send(Msg::Key(KeyEvent { vk: info.vkCode, down, time_ms: info.time as u64 }));
                }
            });
        }
    }
    // Never swallow a key: Ctrl+C must keep copying.
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

/// The running keyboard hook. Dropping it removes the hook and stops the threads.
pub struct Hotkeys {
    tx: Sender<Msg>,
    hook_thread_id: u32,
    hook_thread: Option<JoinHandle<()>>,
    worker: Option<JoinHandle<()>>,
}

impl Hotkeys {
    /// Install the hook. `on_event` runs on a worker thread with the shortcut's name and whether it
    /// fired or only saw the first press of a double shortcut.
    pub fn start(
        shortcuts: Vec<(String, Shortcut)>,
        on_event: impl Fn(&str, Outcome) + Send + 'static,
    ) -> io::Result<Self> {
        let (tx, rx) = mpsc::channel::<Msg>();

        let worker = thread::Builder::new().name("dawn-hotkeys-worker".into()).spawn(move || worker_loop(rx, on_event))?;

        // The hook thread reports whether the hook could be installed.
        let (ready_tx, ready_rx) = mpsc::channel::<io::Result<u32>>();
        let hook_tx = tx.clone();
        let hook_thread = thread::Builder::new().name("dawn-hotkeys-hook".into()).spawn(move || {
            SINK.with(|s| *s.borrow_mut() = Some(hook_tx));
            // SAFETY: plain Win32 calls; the hook is removed below before the thread ends.
            let hook = unsafe {
                let module = GetModuleHandleW(None).map(|m| HINSTANCE(m.0)).ok();
                SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), module, 0)
            };
            let hook = match hook {
                Ok(h) => h,
                Err(e) => {
                    let _ = ready_tx.send(Err(io::Error::other(e.to_string())));
                    return;
                }
            };
            let _ = ready_tx.send(Ok(unsafe { GetCurrentThreadId() }));
            // The message loop keeps the hook callbacks coming; WM_QUIT ends it.
            let mut msg = MSG::default();
            while unsafe { GetMessageW(&mut msg, None, 0, 0) }.0 > 0 {}
            let _ = unsafe { UnhookWindowsHookEx(hook) };
            SINK.with(|s| *s.borrow_mut() = None);
        })?;

        let hook_thread_id = match ready_rx.recv() {
            Ok(Ok(id)) => id,
            Ok(Err(e)) => {
                let _ = tx.send(Msg::Quit);
                return Err(e);
            }
            Err(_) => return Err(io::Error::other("the keyboard hook thread stopped unexpectedly")),
        };
        let _ = tx.send(Msg::Configure(shortcuts));
        Ok(Self { tx, hook_thread_id, hook_thread: Some(hook_thread), worker: Some(worker) })
    }

    /// Replace the shortcuts without reinstalling the hook.
    pub fn set_shortcuts(&self, shortcuts: Vec<(String, Shortcut)>) {
        let _ = self.tx.send(Msg::Configure(shortcuts));
    }
}

impl Drop for Hotkeys {
    fn drop(&mut self) {
        // Ask the hook thread's message loop to end; it unhooks itself.
        let _ = unsafe { PostThreadMessageW(self.hook_thread_id, WM_QUIT, WPARAM(0), LPARAM(0)) };
        if let Some(t) = self.hook_thread.take() {
            let _ = t.join();
        }
        let _ = self.tx.send(Msg::Quit);
        if let Some(t) = self.worker.take() {
            let _ = t.join();
        }
    }
}

fn worker_loop(rx: Receiver<Msg>, on_event: impl Fn(&str, Outcome)) {
    let mut engine = Engine::new(DEFAULT_WINDOW_MS);
    while let Ok(msg) = rx.recv() {
        match msg {
            Msg::Key(ev) => {
                for (name, outcome) in engine.on_event(ev) {
                    on_event(&name, outcome);
                }
            }
            Msg::Configure(list) => engine.set_shortcuts(list),
            Msg::Quit => break,
        }
    }
}

/// File name of the program that owns the foreground window, lower-cased ("chrome.exe").
pub fn foreground_process_name() -> Option<String> {
    // SAFETY: plain Win32 queries; the process handle is closed before returning.
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.0.is_null() {
            return None;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 {
            return None;
        }
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 520];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len).is_ok();
        let _ = CloseHandle(process);
        if !ok {
            return None;
        }
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        let name = path.rsplit(['\\', '/']).next().unwrap_or(&path);
        Some(name.to_lowercase())
    }
}

/// Windows bumps this number whenever the clipboard content changes.
pub fn clipboard_sequence() -> u32 {
    unsafe { GetClipboardSequenceNumber() }
}

/// Mouse pointer position in physical screen pixels.
pub fn cursor_position() -> Option<(i32, i32)> {
    let mut p = POINT::default();
    unsafe { GetCursorPos(&mut p) }.ok().map(|_| (p.x, p.y))
}

/// The foreground window as a plain number, to bring it back to the front later.
pub fn foreground_window() -> Option<isize> {
    let hwnd = unsafe { GetForegroundWindow() };
    (!hwnd.0.is_null()).then_some(hwnd.0 as isize)
}

/// The window still exists.
pub fn window_exists(hwnd: isize) -> bool {
    unsafe { IsWindow(Some(HWND(hwnd as *mut _))) }.as_bool()
}

/// Bring a window to the front and wait (briefly) until it is there.
pub fn focus_window(hwnd: isize) -> bool {
    let handle = HWND(hwnd as *mut _);
    if !unsafe { IsWindow(Some(handle)) }.as_bool() {
        return false;
    }
    unsafe {
        let _ = ShowWindow(handle, SW_RESTORE);
        let _ = SetForegroundWindow(handle);
    }
    for _ in 0..30 {
        if foreground_window() == Some(hwnd) {
            return true;
        }
        thread::sleep(std::time::Duration::from_millis(10));
    }
    false
}

/// Type Ctrl+V into the foreground window.
pub fn send_paste() {
    let key = |vk: u16, up: bool| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(vk),
                wScan: 0,
                dwFlags: if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    let inputs = [key(0x11, false), key(0x56, false), key(0x56, true), key(0x11, true)];
    unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
}

/// The left mouse button is held down right now (any window).
pub fn left_button_down() -> bool {
    (unsafe { GetAsyncKeyState(0x01) } as u16) & 0x8000 != 0
}
