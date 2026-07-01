//! Windows tray backend: a `Shell_NotifyIconW` icon owned by a hidden
//! message-only window, all driven through `libloading`-resolved Win32 calls.
//!
//! The window and the tray icon are created lazily on the first `pump` so they
//! live on whichever thread runs the event loop (`run` or `spawn`) — Win32
//! requires the icon's window and its message pump to share a thread. Shell
//! click callbacks arrive as our `WM_APP+1` message and become [`Event`]s; the
//! context menu is a native popup shown on right-click.

mod ffi;

use std::os::raw::c_void;
use std::sync::OnceLock;
use std::time::Duration;

use ffi::*;

use super::{Backend, Init};
use crate::error::{Error, Result};
use crate::event::Event;
use crate::icon::Icon;
use crate::menu::{Menu, MenuId, MenuItem};
use crate::notification::Notification;

/// Message id (in the `WM_APP` range) the shell uses for icon callbacks.
const CALLBACK_MSG: u32 = WM_APP + 1;
/// Our single tray icon's id within the window.
const ICON_UID: u32 = 1;

// Two Win32 entry points are needed inside the window procedure *before* the
// per-window state pointer is available (during window creation). They are the
// same for every tray, so we stash them globally when the first backend loads.
static DEF_WNDPROC: OnceLock<unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT> =
    OnceLock::new();
static GET_WNDLONGPTR: OnceLock<unsafe extern "system" fn(HWND, i32) -> isize> = OnceLock::new();

pub(crate) fn new(init: Init) -> Result<Box<dyn Backend>> {
    Ok(Box::new(WindowsBackend::new(init)?))
}

struct State {
    win: Win,
    hinstance: HINSTANCE,
    hwnd: HWND,
    hicon: HICON,
    class_name: Vec<u16>,
    icon: (i32, i32, Vec<u8>),
    tooltip: String,
    menu: Option<Menu>,
    pending: Vec<Event>,
    started: bool,
}

// Touched only from the single event-loop thread (see module docs).
unsafe impl Send for State {}

pub(crate) struct WindowsBackend {
    state: Box<State>,
}

impl WindowsBackend {
    fn new(init: Init) -> Result<WindowsBackend> {
        let win = Win::load()?;
        let hinstance = unsafe { (win.GetModuleHandleW)(std::ptr::null()) };
        let icon = (
            init.icon.width as i32,
            init.icon.height as i32,
            init.icon.rgba.clone(),
        );
        let state = Box::new(State {
            win,
            hinstance,
            hwnd: std::ptr::null_mut(),
            hicon: std::ptr::null_mut(),
            class_name: Vec::new(),
            icon,
            tooltip: init.tooltip,
            menu: init.menu,
            pending: Vec::new(),
            started: false,
        });
        Ok(WindowsBackend { state })
    }

    fn ensure_started(&mut self) -> Result<()> {
        if !self.state.started {
            unsafe { self.state.start()? };
            self.state.started = true;
        }
        Ok(())
    }
}

impl State {
    /// Registers the window class, creates the message-only window, builds the
    /// icon, and adds it to the tray. Runs on the event-loop thread.
    unsafe fn start(&mut self) -> Result<()> {
        let _ = DEF_WNDPROC.set(self.win.DefWindowProcW);
        let _ = GET_WNDLONGPTR.set(self.win.GetWindowLongPtrW);

        let state_ptr = self as *mut State;
        self.class_name = wide(&format!("ldtray_wndclass_{:x}", state_ptr as usize));

        unsafe {
            let cursor = (self.win.LoadCursorW)(std::ptr::null_mut(), IDC_ARROW as *const u16);
            let mut class: WNDCLASSEXW = std::mem::zeroed();
            class.cbSize = std::mem::size_of::<WNDCLASSEXW>() as u32;
            class.lpfnWndProc = Some(wndproc);
            class.hInstance = self.hinstance;
            class.hCursor = cursor;
            class.lpszClassName = self.class_name.as_ptr();
            let atom = (self.win.RegisterClassExW)(&class);
            if atom == 0 {
                return Err(Error::Backend("RegisterClassExW failed".into()));
            }

            let name = wide("ldtray");
            let hwnd = (self.win.CreateWindowExW)(
                0,
                self.class_name.as_ptr(),
                name.as_ptr(),
                0,
                0,
                0,
                0,
                0,
                HWND_MESSAGE as HWND,
                std::ptr::null_mut(),
                self.hinstance,
                std::ptr::null_mut(),
            );
            if hwnd.is_null() {
                return Err(Error::Backend("CreateWindowExW failed".into()));
            }
            self.hwnd = hwnd;
            (self.win.SetWindowLongPtrW)(hwnd, GWLP_USERDATA, state_ptr as isize);

            self.hicon = create_hicon(&self.win, self.icon.0, self.icon.1, &self.icon.2);

            let mut data = self.base_notify_data();
            data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP;
            data.uCallbackMessage = CALLBACK_MSG;
            data.hIcon = self.hicon;
            copy_wide(&mut data.szTip, &self.tooltip);
            if (self.win.Shell_NotifyIconW)(NIM_ADD, &mut data) == 0 {
                return Err(Error::Backend("Shell_NotifyIcon(NIM_ADD) failed".into()));
            }
            let mut version = self.base_notify_data();
            version.uVersion = NOTIFYICON_VERSION_4;
            (self.win.Shell_NotifyIconW)(NIM_SETVERSION, &mut version);
        }
        Ok(())
    }

    /// A zeroed `NOTIFYICONDATAW` addressed to our icon.
    fn base_notify_data(&self) -> NOTIFYICONDATAW {
        let mut data: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
        data.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        data.hWnd = self.hwnd;
        data.uID = ICON_UID;
        data
    }

    /// Handles a shell callback message (version-4 packing: the event is in the
    /// low word of `lParam`).
    unsafe fn handle_callback(&mut self, _wparam: WPARAM, lparam: LPARAM) {
        let event = (lparam as u32) & 0xFFFF;
        match event {
            WM_LBUTTONUP => self.pending.push(Event::LeftClick),
            WM_LBUTTONDBLCLK => self.pending.push(Event::DoubleClick),
            WM_MBUTTONUP => self.pending.push(Event::MiddleClick),
            WM_RBUTTONUP | WM_CONTEXTMENU => {
                self.pending.push(Event::RightClick);
                unsafe { self.show_menu() };
            }
            _ => {}
        }
    }

    /// Pops up the native context menu and queues the chosen item's event.
    unsafe fn show_menu(&mut self) {
        let Some(menu) = self.menu.clone() else {
            return;
        };
        unsafe {
            let hmenu = build_hmenu(&self.win, menu.items());
            if hmenu.is_null() {
                return;
            }
            let mut pt = POINT { x: 0, y: 0 };
            (self.win.GetCursorPos)(&mut pt);
            // Required so the menu dismisses when the user clicks elsewhere.
            (self.win.SetForegroundWindow)(self.hwnd);
            let cmd = (self.win.TrackPopupMenu)(
                hmenu,
                TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_LEFTALIGN,
                pt.x,
                pt.y,
                0,
                self.hwnd,
                std::ptr::null(),
            );
            (self.win.DestroyMenu)(hmenu);
            if cmd > 0 {
                self.pending.push(Event::Menu(MenuId(cmd as u32)));
            }
        }
    }

    /// Re-applies the current icon to the tray (after `set_icon`).
    unsafe fn refresh_icon(&mut self) {
        unsafe {
            let old = self.hicon;
            self.hicon = create_hicon(&self.win, self.icon.0, self.icon.1, &self.icon.2);
            let mut data = self.base_notify_data();
            data.uFlags = NIF_ICON;
            data.hIcon = self.hicon;
            (self.win.Shell_NotifyIconW)(NIM_MODIFY, &mut data);
            if !old.is_null() {
                (self.win.DestroyIcon)(old);
            }
        }
    }

    unsafe fn refresh_tooltip(&mut self) {
        unsafe {
            let mut data = self.base_notify_data();
            data.uFlags = NIF_TIP | NIF_SHOWTIP;
            copy_wide(&mut data.szTip, &self.tooltip);
            (self.win.Shell_NotifyIconW)(NIM_MODIFY, &mut data);
        }
    }

    unsafe fn show_notification(&mut self, notification: &Notification) {
        unsafe {
            let mut data = self.base_notify_data();
            data.uFlags = NIF_INFO;
            copy_wide(&mut data.szInfoTitle, &notification.title);
            copy_wide(&mut data.szInfo, &notification.body);
            data.dwInfoFlags = NIIF_INFO;
            (self.win.Shell_NotifyIconW)(NIM_MODIFY, &mut data);
        }
    }
}

impl Drop for State {
    fn drop(&mut self) {
        if !self.started {
            return;
        }
        unsafe {
            let mut data = self.base_notify_data();
            (self.win.Shell_NotifyIconW)(NIM_DELETE, &mut data);
            if !self.hicon.is_null() {
                (self.win.DestroyIcon)(self.hicon);
            }
            if !self.hwnd.is_null() {
                (self.win.DestroyWindow)(self.hwnd);
            }
            if !self.class_name.is_empty() {
                (self.win.UnregisterClassW)(self.class_name.as_ptr(), self.hinstance);
            }
        }
    }
}

impl Backend for WindowsBackend {
    fn set_icon(&mut self, icon: &Icon) -> Result<()> {
        self.state.icon = (icon.width as i32, icon.height as i32, icon.rgba.clone());
        if self.state.started {
            unsafe { self.state.refresh_icon() };
        }
        Ok(())
    }

    fn set_tooltip(&mut self, text: &str) -> Result<()> {
        self.state.tooltip = text.to_string();
        if self.state.started {
            unsafe { self.state.refresh_tooltip() };
        }
        Ok(())
    }

    fn set_menu(&mut self, menu: Option<&Menu>) -> Result<()> {
        self.state.menu = menu.cloned();
        Ok(())
    }

    fn notify(&mut self, notification: &Notification) -> Result<()> {
        self.ensure_started()?;
        unsafe { self.state.show_notification(notification) };
        Ok(())
    }

    fn pump(&mut self, timeout: Duration, sink: &mut dyn FnMut(Event)) -> Result<()> {
        self.ensure_started()?;
        // Copy the fn pointers out so no borrow of `state` is held while the
        // window procedure re-enters through the shared state pointer.
        let msgwait = self.state.win.MsgWaitForMultipleObjectsEx;
        let peek = self.state.win.PeekMessageW;
        let translate = self.state.win.TranslateMessage;
        let dispatch = self.state.win.DispatchMessageW;
        let ms = timeout.as_millis().min(u32::MAX as u128) as u32;
        unsafe {
            msgwait(0, std::ptr::null(), ms, QS_ALLINPUT, MWMO_INPUTAVAILABLE);
            let mut msg: MSG = std::mem::zeroed();
            while peek(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                translate(&msg);
                dispatch(&msg);
            }
        }
        for event in self.state.pending.drain(..) {
            sink(event);
        }
        Ok(())
    }
}

/// The window procedure. Recovers the per-window [`State`] pointer and turns the
/// shell callback into queued events; everything else goes to `DefWindowProcW`.
unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let get_ptr = GET_WNDLONGPTR.get();
    let def = DEF_WNDPROC.get();
    let state_ptr = match get_ptr {
        Some(f) => (unsafe { f(hwnd, GWLP_USERDATA) }) as *mut State,
        None => std::ptr::null_mut(),
    };
    if state_ptr.is_null() {
        return match def {
            Some(f) => unsafe { f(hwnd, msg, wparam, lparam) },
            None => 0,
        };
    }
    let state = unsafe { &mut *state_ptr };
    if msg == CALLBACK_MSG {
        unsafe { state.handle_callback(wparam, lparam) };
        return 0;
    }
    if msg == WM_DESTROY {
        return 0;
    }
    unsafe { (state.win.DefWindowProcW)(hwnd, msg, wparam, lparam) }
}

/// Recursively builds a native popup menu from menu items. Button command ids
/// are the caller's [`MenuId`] values (which must be non-zero on Windows, since
/// `TrackPopupMenu` returns 0 for "nothing selected").
unsafe fn build_hmenu(win: &Win, items: &[MenuItem]) -> HMENU {
    unsafe {
        let hmenu = (win.CreatePopupMenu)();
        if hmenu.is_null() {
            return hmenu;
        }
        for item in items {
            match item {
                MenuItem::Separator => {
                    (win.AppendMenuW)(hmenu, MF_SEPARATOR, 0, std::ptr::null());
                }
                MenuItem::Button {
                    id,
                    label,
                    enabled,
                    checked,
                } => {
                    let mut flags = MF_STRING;
                    if !enabled {
                        flags |= MF_GRAYED;
                    }
                    if *checked == Some(true) {
                        flags |= MF_CHECKED;
                    }
                    let text = wide(label);
                    (win.AppendMenuW)(hmenu, flags, id.0 as usize, text.as_ptr());
                }
                MenuItem::Submenu {
                    label,
                    enabled,
                    items,
                } => {
                    let submenu = build_hmenu(win, items);
                    let mut flags = MF_POPUP;
                    if !enabled {
                        flags |= MF_GRAYED;
                    }
                    let text = wide(label);
                    (win.AppendMenuW)(hmenu, flags, submenu as usize, text.as_ptr());
                }
            }
        }
        hmenu
    }
}

/// Builds an `HICON` from RGBA pixels via a top-down 32bpp DIB section plus a
/// throwaway monochrome mask (the alpha channel is the real mask).
unsafe fn create_hicon(win: &Win, width: i32, height: i32, rgba: &[u8]) -> HICON {
    unsafe {
        let mut header: BITMAPINFOHEADER = std::mem::zeroed();
        header.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        header.biWidth = width;
        header.biHeight = -height; // negative = top-down
        header.biPlanes = 1;
        header.biBitCount = 32;
        header.biCompression = BI_RGB;

        let mut bits: *mut c_void = std::ptr::null_mut();
        let color = (win.CreateDIBSection)(
            std::ptr::null_mut(),
            &header,
            DIB_RGB_COLORS,
            &mut bits,
            std::ptr::null_mut(),
            0,
        );
        if color.is_null() || bits.is_null() {
            return std::ptr::null_mut();
        }
        // RGBA -> BGRA, the order the color bitmap expects.
        let dst = bits as *mut u8;
        let pixels = (width as usize) * (height as usize);
        for i in 0..pixels {
            let s = i * 4;
            *dst.add(s) = rgba[s + 2]; // B
            *dst.add(s + 1) = rgba[s + 1]; // G
            *dst.add(s + 2) = rgba[s]; // R
            *dst.add(s + 3) = rgba[s + 3]; // A
        }

        let mask = (win.CreateBitmap)(width, height, 1, 1, std::ptr::null());
        let mut info = ICONINFO {
            fIcon: 1,
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mask,
            hbmColor: color,
        };
        let icon = (win.CreateIconIndirect)(&mut info);
        (win.DeleteObject)(color);
        (win.DeleteObject)(mask);
        icon
    }
}

/// UTF-16, NUL-terminated — the form the wide Win32 APIs want.
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Copies `s` (UTF-16) into a fixed buffer, always leaving a NUL terminator.
fn copy_wide(dst: &mut [u16], s: &str) {
    let limit = dst.len().saturating_sub(1);
    let mut n = 0;
    for u in s.encode_utf16().take(limit) {
        dst[n] = u;
        n += 1;
    }
    dst[n] = 0;
}
