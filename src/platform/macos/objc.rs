//! Minimal Objective-C runtime + AppKit access, resolved at runtime.
//!
//! We `dlopen` `libobjc` and the AppKit/Foundation frameworks, then drive Cocoa
//! purely through `objc_getClass` / `sel_registerName` / `objc_msgSend`. Nothing
//! links against AppKit, so a headless/SSH process just gets
//! [`crate::Error::LibraryLoad`] and no tray.
//!
//! `objc_msgSend` is variadic in C; we resolve it once and `transmute` it to the
//! exact signature at each call site (sound on both x86-64 and arm64 for the
//! integer/pointer/double/small-struct returns we use — we never message-send a
//! large struct return, which would need `objc_msgSend_stret` on x86-64).
#![allow(dead_code)]
#![allow(non_snake_case)]
// `id`/`SEL` mirror the Objective-C runtime's own type names.
#![allow(non_camel_case_types)]
#![allow(clippy::upper_case_acronyms)]
#![allow(clippy::too_many_arguments)]

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_void};

use libloading::{Library, Symbol};

pub type id = *mut c_void;
pub type SEL = *mut c_void;
pub type Class = *mut c_void;
/// Bare method-implementation pointer, as taken by `class_addMethod`.
pub type Imp = unsafe extern "C" fn();

/// A Cocoa size (`CGFloat` = `f64` on 64-bit).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct NSSize {
    pub width: f64,
    pub height: f64,
}

const LIBOBJC: &str = "/usr/lib/libobjc.A.dylib";
const APPKIT: &str = "/System/Library/Frameworks/AppKit.framework/AppKit";
const FOUNDATION: &str = "/System/Library/Frameworks/Foundation.framework/Foundation";

/// Resolved Objective-C runtime entry points (plus the loaded frameworks, kept
/// alive so their classes stay registered).
pub struct ObjC {
    objc_getClass: unsafe extern "C" fn(*const c_char) -> Class,
    sel_registerName: unsafe extern "C" fn(*const c_char) -> SEL,
    objc_allocateClassPair: unsafe extern "C" fn(Class, *const c_char, usize) -> Class,
    objc_registerClassPair: unsafe extern "C" fn(Class),
    class_addMethod: unsafe extern "C" fn(Class, SEL, Imp, *const c_char) -> i8,
    class_addIvar: unsafe extern "C" fn(Class, *const c_char, usize, u8, *const c_char) -> i8,
    object_getInstanceVariable:
        unsafe extern "C" fn(id, *const c_char, *mut *mut c_void) -> *mut c_void,
    object_setInstanceVariable: unsafe extern "C" fn(id, *const c_char, *mut c_void) -> *mut c_void,
    msg_send: unsafe extern "C" fn(),
    _libs: Vec<Library>,
}

/// Expands to a typed wrapper around `objc_msgSend` with the given trailing
/// argument list and (optional) return type.
macro_rules! msg {
    ($name:ident ( $($an:ident : $at:ty),* ) $(-> $rt:ty)? ) => {
        pub unsafe fn $name(&self, obj: id, sel: SEL $(, $an: $at)*) $(-> $rt)? {
            let f: unsafe extern "C" fn(id, SEL $(, $at)*) $(-> $rt)? =
                unsafe { std::mem::transmute(self.msg_send) };
            unsafe { f(obj, sel $(, $an)*) }
        }
    };
}

impl ObjC {
    pub fn load() -> crate::error::Result<ObjC> {
        let objc = open(LIBOBJC)?;
        let appkit = open(APPKIT)?;
        let foundation = open(FOUNDATION)?;
        Ok(ObjC {
            objc_getClass: unsafe { sym(&objc, b"objc_getClass\0")? },
            sel_registerName: unsafe { sym(&objc, b"sel_registerName\0")? },
            objc_allocateClassPair: unsafe { sym(&objc, b"objc_allocateClassPair\0")? },
            objc_registerClassPair: unsafe { sym(&objc, b"objc_registerClassPair\0")? },
            class_addMethod: unsafe { sym(&objc, b"class_addMethod\0")? },
            class_addIvar: unsafe { sym(&objc, b"class_addIvar\0")? },
            object_getInstanceVariable: unsafe { sym(&objc, b"object_getInstanceVariable\0")? },
            object_setInstanceVariable: unsafe { sym(&objc, b"object_setInstanceVariable\0")? },
            msg_send: unsafe { sym(&objc, b"objc_msgSend\0")? },
            _libs: vec![objc, appkit, foundation],
        })
    }

    pub unsafe fn class(&self, name: &CStr) -> Class {
        unsafe { (self.objc_getClass)(name.as_ptr()) }
    }

    pub unsafe fn sel(&self, name: &CStr) -> SEL {
        unsafe { (self.sel_registerName)(name.as_ptr()) }
    }

    pub unsafe fn allocate_class(&self, superclass: Class, name: &CStr) -> Class {
        unsafe { (self.objc_allocateClassPair)(superclass, name.as_ptr(), 0) }
    }

    pub unsafe fn register_class(&self, cls: Class) {
        unsafe { (self.objc_registerClassPair)(cls) }
    }

    pub unsafe fn add_method(&self, cls: Class, sel: SEL, imp: Imp, types: &CStr) {
        unsafe { (self.class_addMethod)(cls, sel, imp, types.as_ptr()) };
    }

    pub unsafe fn add_ivar(&self, cls: Class, name: &CStr, size: usize, align: u8, types: &CStr) {
        unsafe { (self.class_addIvar)(cls, name.as_ptr(), size, align, types.as_ptr()) };
    }

    pub unsafe fn set_ivar(&self, obj: id, name: &CStr, value: *mut c_void) {
        unsafe { (self.object_setInstanceVariable)(obj, name.as_ptr(), value) };
    }

    pub unsafe fn get_ivar(&self, obj: id, name: &CStr) -> *mut c_void {
        let mut out: *mut c_void = std::ptr::null_mut();
        unsafe { (self.object_getInstanceVariable)(obj, name.as_ptr(), &mut out) };
        out
    }

    /// `[[NSString alloc] ...]`-free helper: an autoreleased `NSString` from a
    /// Rust string. Retain it if you need it past the current autorelease pool.
    pub unsafe fn nsstring(&self, s: &str) -> id {
        let c = CString::new(s.replace('\0', " ")).unwrap_or_default();
        unsafe {
            let cls = self.class(c"NSString");
            self.send_str(cls, self.sel(c"stringWithUTF8String:"), c.as_ptr())
        }
    }

    // Typed objc_msgSend wrappers. Naming: send_<args>[_ret<type>].
    msg!(send0() -> id);
    msg!(send_id(a: id) -> id);
    msg!(send_void_id(a: id));
    msg!(send_void_sel(a: SEL));
    msg!(send_void_i64(a: i64));
    msg!(send_void_u64(a: u64));
    msg!(send_void_i8(a: i8));
    msg!(send_i64(a: i64) -> id);
    msg!(send_f64(a: f64) -> id);
    msg!(send_ret_u64() -> u64);
    msg!(send_ret_i64() -> i64);
    msg!(send_str(a: *const c_char) -> id);
    msg!(send_size(a: NSSize) -> id);
    msg!(send_next_event(mask: u64, date: id, mode: id, dequeue: i8) -> id);
    msg!(send_init_item(title: id, action: SEL, key: id) -> id);
    msg!(send_init_bitmap(
        planes: *mut *mut u8,
        w: i64, h: i64, bps: i64, spp: i64,
        alpha: i8, planar: i8, colorspace: id, bpr: i64, bpp: i64
    ) -> id);
}

fn open(path: &str) -> crate::error::Result<Library> {
    unsafe { Library::new(path) }
        .map_err(|e| crate::error::Error::LibraryLoad(format!("{path}: {e}")))
}

unsafe fn sym<T: Copy>(lib: &Library, name: &[u8]) -> crate::error::Result<T> {
    let s: Symbol<T> = unsafe { lib.get(name) }.map_err(|e| {
        let pretty = String::from_utf8_lossy(name.strip_suffix(b"\0").unwrap_or(name));
        crate::error::Error::LibraryLoad(format!("missing symbol {pretty}: {e}"))
    })?;
    Ok(*s)
}
