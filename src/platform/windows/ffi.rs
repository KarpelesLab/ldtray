//! Runtime (`libloading`) bindings to the Win32 functions the tray needs.
//!
//! Symbols come from `user32`, `shell32`, `gdi32` and `kernel32`, all opened at
//! runtime — the crate never links against them. As on the other platforms, a
//! missing DLL or symbol surfaces as [`crate::Error::LibraryLoad`].
#![allow(dead_code)]
#![allow(non_snake_case)]
#![allow(clippy::upper_case_acronyms)]

use std::os::raw::c_void;

use libloading::{Library, Symbol};

// Win32 handle and integer aliases.
pub type HWND = *mut c_void;
pub type HINSTANCE = *mut c_void;
pub type HMODULE = *mut c_void;
pub type HICON = *mut c_void;
pub type HMENU = *mut c_void;
pub type HBITMAP = *mut c_void;
pub type HCURSOR = *mut c_void;
pub type HBRUSH = *mut c_void;
pub type HDC = *mut c_void;
pub type HANDLE = *mut c_void;
pub type WPARAM = usize;
pub type LPARAM = isize;
pub type LRESULT = isize;

/// Window procedure signature (`extern "system"` = stdcall on x86, C on x64).
pub type WndProc = unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct POINT {
    pub x: i32,
    pub y: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct RECT {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

#[repr(C)]
pub struct MSG {
    pub hwnd: HWND,
    pub message: u32,
    pub wParam: WPARAM,
    pub lParam: LPARAM,
    pub time: u32,
    pub pt: POINT,
    pub lPrivate: u32,
}

#[repr(C)]
pub struct WNDCLASSEXW {
    pub cbSize: u32,
    pub style: u32,
    pub lpfnWndProc: Option<WndProc>,
    pub cbClsExtra: i32,
    pub cbWndExtra: i32,
    pub hInstance: HINSTANCE,
    pub hIcon: HICON,
    pub hCursor: HCURSOR,
    pub hbrBackground: HBRUSH,
    pub lpszMenuName: *const u16,
    pub lpszClassName: *const u16,
    pub hIconSm: HICON,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct GUID {
    pub Data1: u32,
    pub Data2: u16,
    pub Data3: u16,
    pub Data4: [u8; 8],
}

/// `NOTIFYICONDATAW` (Vista+ layout; `cbSize` is set to this struct's size).
#[repr(C)]
pub struct NOTIFYICONDATAW {
    pub cbSize: u32,
    pub hWnd: HWND,
    pub uID: u32,
    pub uFlags: u32,
    pub uCallbackMessage: u32,
    pub hIcon: HICON,
    pub szTip: [u16; 128],
    pub dwState: u32,
    pub dwStateMask: u32,
    pub szInfo: [u16; 256],
    /// Union of `uTimeout` / `uVersion`; we use it as the version.
    pub uVersion: u32,
    pub szInfoTitle: [u16; 64],
    pub dwInfoFlags: u32,
    pub guidItem: GUID,
    pub hBalloonIcon: HICON,
}

#[repr(C)]
pub struct ICONINFO {
    pub fIcon: i32,
    pub xHotspot: u32,
    pub yHotspot: u32,
    pub hbmMask: HBITMAP,
    pub hbmColor: HBITMAP,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct BITMAPINFOHEADER {
    pub biSize: u32,
    pub biWidth: i32,
    pub biHeight: i32,
    pub biPlanes: u16,
    pub biBitCount: u16,
    pub biCompression: u32,
    pub biSizeImage: u32,
    pub biXPelsPerMeter: i32,
    pub biYPelsPerMeter: i32,
    pub biClrUsed: u32,
    pub biClrImportant: u32,
}

// --- constants -------------------------------------------------------------

pub const WM_DESTROY: u32 = 0x0002;
pub const WM_APP: u32 = 0x8000;
pub const WM_LBUTTONUP: u32 = 0x0202;
pub const WM_LBUTTONDBLCLK: u32 = 0x0203;
pub const WM_RBUTTONUP: u32 = 0x0205;
pub const WM_MBUTTONUP: u32 = 0x0208;
pub const WM_CONTEXTMENU: u32 = 0x007B;

pub const GWLP_USERDATA: i32 = -21;
pub const HWND_MESSAGE: isize = -3;

pub const PM_REMOVE: u32 = 0x0001;
pub const QS_ALLINPUT: u32 = 0x04FF;
pub const MWMO_INPUTAVAILABLE: u32 = 0x0004;

pub const MF_STRING: u32 = 0x0000;
pub const MF_SEPARATOR: u32 = 0x0800;
pub const MF_POPUP: u32 = 0x0010;
pub const MF_CHECKED: u32 = 0x0008;
pub const MF_GRAYED: u32 = 0x0001;

pub const TPM_LEFTALIGN: u32 = 0x0000;
pub const TPM_RIGHTBUTTON: u32 = 0x0002;
pub const TPM_RETURNCMD: u32 = 0x0100;

pub const NIM_ADD: u32 = 0x00000000;
pub const NIM_MODIFY: u32 = 0x00000001;
pub const NIM_DELETE: u32 = 0x00000002;
pub const NIM_SETVERSION: u32 = 0x00000004;

pub const NIF_MESSAGE: u32 = 0x00000001;
pub const NIF_ICON: u32 = 0x00000002;
pub const NIF_TIP: u32 = 0x00000004;
pub const NIF_INFO: u32 = 0x00000010;
pub const NIF_SHOWTIP: u32 = 0x00000080;

pub const NOTIFYICON_VERSION_4: u32 = 4;
pub const NIIF_INFO: u32 = 0x00000001;
pub const NIIF_USER: u32 = 0x00000004;

/// Sent (via the icon callback message) when the user clicks the balloon body.
pub const NIN_BALLOONUSERCLICK: u32 = 0x0405; // WM_USER + 5

pub const BI_RGB: u32 = 0;
pub const DIB_RGB_COLORS: u32 = 0;

/// `MAKEINTRESOURCE(32512)` — the standard arrow cursor.
pub const IDC_ARROW: u16 = 32512;

// --- the loaded binding table ----------------------------------------------

macro_rules! win_bindings {
    ( $( $lib:ident $soname:literal { $( fn $name:ident ( $($arg:ty),* $(,)? ) $(-> $ret:ty)?; )* } )+ ) => {
        pub struct Win {
            $( $( pub $name: unsafe extern "system" fn($($arg),*) $(-> $ret)?, )* )+
            $( $lib: Library, )+
        }
        impl Win {
            pub fn load() -> crate::error::Result<Win> {
                $(
                    let $lib = unsafe { Library::new($soname) }.map_err(|e| {
                        crate::error::Error::LibraryLoad(format!("{}: {}", $soname, e))
                    })?;
                )+
                Ok(Win {
                    $( $( $name: unsafe {
                        load_sym(&$lib, concat!(stringify!($name), "\0").as_bytes())?
                    }, )* )+
                    $( $lib, )+
                })
            }
        }
    };
}

unsafe fn load_sym<T: Copy>(lib: &Library, symbol: &[u8]) -> crate::error::Result<T> {
    let sym: Symbol<T> = unsafe { lib.get(symbol) }.map_err(|e| {
        let pretty = String::from_utf8_lossy(symbol.strip_suffix(b"\0").unwrap_or(symbol));
        crate::error::Error::LibraryLoad(format!("missing symbol {pretty}: {e}"))
    })?;
    Ok(*sym)
}

win_bindings! {
    kernel32 "kernel32.dll" {
        fn GetModuleHandleW(*const u16) -> HMODULE;
    }
    user32 "user32.dll" {
        fn RegisterClassExW(*const WNDCLASSEXW) -> u16;
        fn UnregisterClassW(*const u16, HINSTANCE) -> i32;
        fn CreateWindowExW(
            u32, *const u16, *const u16, u32, i32, i32, i32, i32, HWND, HMENU, HINSTANCE, *mut c_void,
        ) -> HWND;
        fn DestroyWindow(HWND) -> i32;
        fn DefWindowProcW(HWND, u32, WPARAM, LPARAM) -> LRESULT;
        fn LoadCursorW(HINSTANCE, *const u16) -> HCURSOR;
        fn SetWindowLongPtrW(HWND, i32, isize) -> isize;
        fn GetWindowLongPtrW(HWND, i32) -> isize;
        fn PeekMessageW(*mut MSG, HWND, u32, u32, u32) -> i32;
        fn TranslateMessage(*const MSG) -> i32;
        fn DispatchMessageW(*const MSG) -> LRESULT;
        fn MsgWaitForMultipleObjectsEx(u32, *const HANDLE, u32, u32, u32) -> u32;
        fn PostMessageW(HWND, u32, WPARAM, LPARAM) -> i32;
        fn CreatePopupMenu() -> HMENU;
        fn AppendMenuW(HMENU, u32, usize, *const u16) -> i32;
        fn TrackPopupMenu(HMENU, u32, i32, i32, i32, HWND, *const RECT) -> i32;
        fn DestroyMenu(HMENU) -> i32;
        fn SetForegroundWindow(HWND) -> i32;
        fn GetCursorPos(*mut POINT) -> i32;
        fn CreateIconIndirect(*mut ICONINFO) -> HICON;
        fn DestroyIcon(HICON) -> i32;
    }
    shell32 "shell32.dll" {
        fn Shell_NotifyIconW(u32, *mut NOTIFYICONDATAW) -> i32;
    }
    gdi32 "gdi32.dll" {
        fn CreateDIBSection(HDC, *const BITMAPINFOHEADER, u32, *mut *mut c_void, HANDLE, u32) -> HBITMAP;
        fn CreateBitmap(i32, i32, u32, u32, *const c_void) -> HBITMAP;
        fn DeleteObject(*mut c_void) -> i32;
    }
}
