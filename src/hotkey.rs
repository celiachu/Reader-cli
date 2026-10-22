#[cfg(windows)]
use std::sync::mpsc::Sender;
use std::sync::mpsc::{channel, Receiver};
#[cfg(windows)]
use std::thread;

pub struct GlobalHotkeyListener {
    #[cfg(windows)]
    worker: Option<(thread::JoinHandle<()>, u32)>,
    #[cfg(not(windows))]
    _phantom: (),
}

impl GlobalHotkeyListener {
    #[cfg(windows)]
    pub fn start_f12() -> (Self, Receiver<()>) {
        let (tx, rx) = channel();
        let (ready_tx, ready_rx) = channel();

        let handle = thread::Builder::new()
            .name("global-hotkey-listener".into())
            .spawn(move || {
                run_windows_hotkey_loop(tx, ready_tx);
            });

        match handle {
            Ok(handle) => match ready_rx.recv() {
                Ok(thread_id) => (
                    Self {
                        worker: Some((handle, thread_id)),
                    },
                    rx,
                ),
                Err(_) => {
                    let _ = handle.join();
                    (Self { worker: None }, rx)
                }
            },
            Err(_) => (Self { worker: None }, rx),
        }
    }

    #[cfg(not(windows))]
    pub fn start_f12() -> (Self, Receiver<()>) {
        let (_tx, rx) = channel();
        (Self { _phantom: () }, rx)
    }
}

#[cfg(windows)]
impl Drop for GlobalHotkeyListener {
    fn drop(&mut self) {
        if let Some((handle, thread_id)) = self.worker.take() {
            unsafe {
                PostThreadMessageW(thread_id, WM_QUIT, 0, 0);
            }
            let _ = handle.join();
        }
    }
}

#[cfg(not(windows))]
impl Drop for GlobalHotkeyListener {
    fn drop(&mut self) {}
}

#[cfg(windows)]
const WM_HOTKEY: u32 = 0x0312;
#[cfg(windows)]
const WM_QUIT: u32 = 0x0012;
#[cfg(windows)]
const VK_F12: u32 = 0x77;
#[cfg(windows)]
const MOD_NOREPEAT: u32 = 0x4000;
#[cfg(windows)]
const HOTKEY_ID: i32 = 0x5244; // "RD"

#[cfg(windows)]
#[repr(C)]
struct Point {
    x: i32,
    y: i32,
}

#[cfg(windows)]
#[repr(C)]
struct Msg {
    hwnd: *mut std::ffi::c_void,
    message: u32,
    wparam: usize,
    lparam: isize,
    time: u32,
    pt: Point,
}

#[cfg(windows)]
#[link(name = "user32")]
extern "system" {
    fn RegisterHotKey(hwnd: *mut std::ffi::c_void, id: i32, fs_modifiers: u32, vk: u32) -> i32;
    fn UnregisterHotKey(hwnd: *mut std::ffi::c_void, id: i32) -> i32;
    fn GetMessageW(
        msg: *mut Msg,
        hwnd: *mut std::ffi::c_void,
        msg_filter_min: u32,
        msg_filter_max: u32,
    ) -> i32;
    fn PostThreadMessageW(id_thread: u32, msg: u32, wparam: usize, lparam: isize) -> i32;
}

#[cfg(windows)]
#[link(name = "kernel32")]
extern "system" {
    fn GetCurrentThreadId() -> u32;
}

#[cfg(windows)]
fn run_windows_hotkey_loop(tx: Sender<()>, ready_tx: Sender<u32>) {
    let thread_id = unsafe { GetCurrentThreadId() };

    // RegisterHotKey for the calling thread by passing NULL for hwnd.
    let registered =
        unsafe { RegisterHotKey(std::ptr::null_mut(), HOTKEY_ID, MOD_NOREPEAT, VK_F12) != 0 };

    // Notify parent thread that the listener is initialized.
    let _ = ready_tx.send(thread_id);

    if !registered {
        // If registering F12 failed (e.g. key already registered by another application),
        // we still run message pump until WM_QUIT so cleanup remains sound.
    }

    unsafe {
        let mut msg: Msg = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            if msg.message == WM_HOTKEY && msg.wparam == HOTKEY_ID as usize {
                let _ = tx.send(());
            }
        }
        if registered {
            UnregisterHotKey(std::ptr::null_mut(), HOTKEY_ID);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn global_hotkey_listener_initializes_and_drops_cleanly() {
        let (listener, rx) = GlobalHotkeyListener::start_f12();
        assert!(rx.try_recv().is_err());
        drop(listener);
    }
}
