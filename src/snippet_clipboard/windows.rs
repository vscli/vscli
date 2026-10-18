//! Windows text clipboard access runs exclusively in the native clipboard worker.
use super::{AtomicBool, MAX_BYTES, Ordering};
use anyhow::{Context, Result, bail, ensure};
use std::{
    ffi::c_void,
    ptr,
    time::{Duration, Instant},
};
type Handle = *mut c_void;
const UNICODE_TEXT: u32 = 13;
#[link(name = "user32")]
unsafe extern "system" {
    fn OpenClipboard(owner: Handle) -> i32;
    #[cfg(test)]
    fn GetOpenClipboardWindow() -> Handle;
    fn CloseClipboard() -> i32;
    fn EmptyClipboard() -> i32;
    fn IsClipboardFormatAvailable(format: u32) -> i32;
    fn GetClipboardData(format: u32) -> Handle;
    fn SetClipboardData(format: u32, memory: Handle) -> Handle;
    fn CreateWindowExW(
        ex_style: u32,
        class: *const u16,
        name: *const u16,
        style: u32,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        parent: Handle,
        menu: Handle,
        instance: Handle,
        parameter: *mut c_void,
    ) -> Handle;
    fn DestroyWindow(window: Handle) -> i32;
}
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GlobalAlloc(flags: u32, bytes: usize) -> Handle;
    fn GlobalFree(memory: Handle) -> Handle;
    fn GlobalLock(memory: Handle) -> *mut c_void;
    fn GlobalUnlock(memory: Handle) -> i32;
    fn GlobalSize(memory: Handle) -> usize;
}
fn current(cancel: &AtomicBool) -> Result<()> {
    ensure!(
        !cancel.load(Ordering::Acquire),
        "Clipboard request canceled"
    );
    Ok(())
}
// OpenClipboard ownership belongs to this thread, even for a null window.
struct Clipboard(bool, std::marker::PhantomData<std::rc::Rc<()>>);
impl Clipboard {
    fn open(owner: Handle, cancel: &AtomicBool) -> Result<Self> {
        let started = Instant::now();
        loop {
            current(cancel)?;
            // SAFETY: owner is null for reading or a live window on this same thread.
            if unsafe { OpenClipboard(owner) } != 0 {
                return Ok(Self(true, std::marker::PhantomData));
            }
            let error = std::io::Error::last_os_error();
            if started.elapsed() >= Duration::from_millis(300) {
                return Err(error).context("Cannot open Windows clipboard");
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    fn close(mut self) -> Result<()> {
        // SAFETY: this guard owns the successful OpenClipboard on the current thread.
        if unsafe { CloseClipboard() } == 0 {
            bail!(
                "Windows clipboard close failed; an authorized write may have completed: {}",
                std::io::Error::last_os_error()
            );
        }
        self.0 = false;
        Ok(())
    }
}
impl Drop for Clipboard {
    fn drop(&mut self) {
        if self.0 {
            // SAFETY: the guard retains its original successful OpenClipboard ownership.
            unsafe {
                CloseClipboard();
            }
        }
    }
}
struct Window(Handle);
impl Window {
    fn new() -> Result<Self> {
        let class: Vec<u16> = "STATIC\0".encode_utf16().collect();
        let name = [0u16];
        // SAFETY: the built-in STATIC class and empty name are NUL-terminated for
        // this synchronous call. HWND_MESSAGE(-3) creates an invisible message-only
        // owner on this worker thread; no application document/view is involved.
        let window = unsafe {
            CreateWindowExW(
                0,
                class.as_ptr(),
                name.as_ptr(),
                0,
                0,
                0,
                0,
                0,
                (-3isize) as Handle,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
            )
        };
        ensure!(
            !window.is_null(),
            "Cannot create Windows clipboard owner: {}",
            std::io::Error::last_os_error()
        );
        Ok(Self(window))
    }
}
impl Drop for Window {
    fn drop(&mut self) {
        // SAFETY: this thread created and exclusively owns this message-only window.
        unsafe {
            DestroyWindow(self.0);
        }
    }
}
struct Allocation(Handle);
impl Drop for Allocation {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: non-null means the allocation has not transferred to the system.
            unsafe {
                GlobalFree(self.0);
            }
        }
    }
}
struct Locked {
    handle: Handle,
    pointer: *mut u16,
}
impl Locked {
    fn new(handle: Handle) -> Result<Self> {
        // SAFETY: handle is a live movable allocation, privately owned or obtained
        // for CF_UNICODETEXT while this thread holds the clipboard open.
        let pointer = unsafe { GlobalLock(handle) }.cast::<u16>();
        ensure!(
            !pointer.is_null(),
            "Cannot lock Windows clipboard text: {}",
            std::io::Error::last_os_error()
        );
        Ok(Self { handle, pointer })
    }
}
impl Drop for Locked {
    fn drop(&mut self) {
        // SAFETY: this guard owns one matching successful GlobalLock. A zero return
        // can simply mean the lock count reached zero; it does not require a retry.
        unsafe {
            GlobalUnlock(self.handle);
        }
    }
}
pub(super) fn read(cancel: &AtomicBool) -> Result<String> {
    let clipboard = Clipboard::open(ptr::null_mut(), cancel)?;
    current(cancel)?;
    // SAFETY: clipboard format inspection and data access occur under our open.
    if unsafe { IsClipboardFormatAvailable(UNICODE_TEXT) } == 0 {
        clipboard.close()?;
        return Ok(String::new());
    }
    // SAFETY: CF_UNICODETEXT supplies a movable memory handle retained by the
    // system until this clipboard open is closed; this code never frees it.
    let handle = unsafe { GetClipboardData(UNICODE_TEXT) };
    ensure!(
        !handle.is_null(),
        "Cannot read Windows clipboard text: {}",
        std::io::Error::last_os_error()
    );
    // SAFETY: handle remains live under this open clipboard and is a global block.
    let bytes = unsafe { GlobalSize(handle) };
    ensure!(bytes >= 2, "Windows clipboard allocation is too small");
    let text = {
        let locked = Locked::new(handle)?;
        // CF_UNICODETEXT guarantees initialized UTF-16 through a NUL. Read
        // only that prefix, without borrowing unused allocation padding.
        let mut units = Vec::new();
        let mut terminated = false;
        // GlobalSize may include allocator padding. Cap the actual scan rather
        // than rejecting a valid maximum-sized string because its block grew.
        for index in 0..(bytes / 2).min(MAX_BYTES + 1) {
            // SAFETY: the lock remains live and index is within GlobalSize.
            let unit = unsafe { locked.pointer.add(index).read() };
            if unit == 0 {
                terminated = true;
                break;
            }
            units.push(unit);
        }
        ensure!(
            terminated,
            "Windows clipboard text has no bounded NUL terminator"
        );
        let text = String::from_utf16(&units)
            .context("Windows clipboard contains unpaired UTF-16 surrogates")?;
        ensure!(
            text.len() <= MAX_BYTES,
            "Windows clipboard text exceeds 1 MiB"
        );
        text
    };
    current(cancel)?;
    clipboard.close()?;
    Ok(text)
}
pub(super) fn write(text: &str, cancel: &AtomicBool) -> Result<()> {
    current(cancel)?;
    ensure!(
        text.len() <= MAX_BYTES,
        "Windows clipboard text exceeds 1 MiB"
    );
    let units: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let bytes = units
        .len()
        .checked_mul(2)
        .context("Windows clipboard allocation overflow")?;
    // SAFETY: bounded size and GMEM_MOVEABLE | GMEM_ZEROINIT satisfy
    // SetClipboardData. RAII retains ownership until a successful transfer.
    let handle = unsafe { GlobalAlloc(0x0042, bytes) };
    ensure!(
        !handle.is_null(),
        "Cannot allocate Windows clipboard text: {}",
        std::io::Error::last_os_error()
    );
    let mut allocation = Allocation(handle);
    {
        let locked = Locked::new(handle)?;
        // SAFETY: the private allocation has exactly bytes space, the vectors do not
        // alias it, and its lock stays live for the whole bounded copy.
        unsafe {
            ptr::copy_nonoverlapping(units.as_ptr(), locked.pointer, units.len());
        }
    }
    let owner = Window::new()?;
    let clipboard = Clipboard::open(owner.0, cancel)?;
    current(cancel)?;
    // SAFETY: the original live message-only owner is associated with our open.
    // Preparation/allocation completed before this first externally visible effect.
    ensure!(
        unsafe { EmptyClipboard() } != 0,
        "Cannot empty Windows clipboard: {}",
        std::io::Error::last_os_error()
    );
    // SAFETY: the allocation is unlocked and movable, and our live window owns
    // the clipboard. Successful transfer makes the OS responsible for freeing it.
    let transferred = unsafe { SetClipboardData(UNICODE_TEXT, handle) };
    ensure!(
        !transferred.is_null(),
        "Windows clipboard write failed after clearing; contents may have changed: {}",
        std::io::Error::last_os_error()
    );
    allocation.0 = ptr::null_mut();
    clipboard.close()?;
    Ok(())
}

#[cfg(test)]
pub(super) fn hold_open_for_test() -> Result<impl Drop> {
    struct Held {
        // Fields drop in declaration order: close before destroying the owner.
        _clipboard: Clipboard,
        _window: Window,
    }
    impl Drop for Held {
        fn drop(&mut self) {}
    }
    let window = Window::new()?;
    let clipboard = Clipboard::open(window.0, &AtomicBool::new(false))?;
    // SAFETY: the window and clipboard open are both live on this thread.
    ensure!(
        unsafe { GetOpenClipboardWindow() } == window.0,
        "Windows did not associate the clipboard open with the holder window"
    );
    Ok(Held {
        _clipboard: clipboard,
        _window: window,
    })
}
