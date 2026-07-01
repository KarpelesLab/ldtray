//! Low-level helpers for composing and reading D-Bus messages.
//!
//! These wrap the raw `dbus_message_iter_*` calls into a handful of typed
//! appenders (used to answer `org.freedesktop.DBus.Properties` queries and to
//! build method calls) plus a small reader for string arguments. Shared by the
//! StatusNotifierItem code here and by the menu/notification milestones, so a
//! few helpers are not yet referenced.
#![allow(dead_code)]

use std::ffi::CStr;
use std::os::raw::{c_char, c_void};

use super::dbus::*;

/// A value that can be wrapped in a D-Bus `variant`. Covers exactly the property
/// types the StatusNotifierItem interface exposes.
pub(super) enum Variant<'a> {
    /// `s`
    Str(&'a CStr),
    /// `o`
    ObjectPath(&'a CStr),
    /// `b`
    Bool(bool),
    /// `i`
    Int32(i32),
    /// `a(iiay)` with a single entry — the icon pixmap in ARGB32 (network order).
    Pixmap {
        width: i32,
        height: i32,
        argb: &'a [u8],
    },
    /// `(sa(iiay)ss)` — StatusNotifierItem tooltip: icon name, pixmaps, title, body.
    ToolTip { title: &'a CStr, body: &'a CStr },
}

/// Appends a plain `s` string.
pub(super) unsafe fn append_str(d: &DBus, it: *mut DBusMessageIter, s: &CStr) {
    let p = s.as_ptr();
    unsafe {
        (d.dbus_message_iter_append_basic)(
            it,
            DBUS_TYPE_STRING,
            &p as *const *const c_char as *const c_void,
        );
    }
}

/// Appends a plain `o` object path.
pub(super) unsafe fn append_object_path(d: &DBus, it: *mut DBusMessageIter, s: &CStr) {
    let p = s.as_ptr();
    unsafe {
        (d.dbus_message_iter_append_basic)(
            it,
            DBUS_TYPE_OBJECT_PATH,
            &p as *const *const c_char as *const c_void,
        );
    }
}

/// Appends a plain `i` int32.
pub(super) unsafe fn append_i32(d: &DBus, it: *mut DBusMessageIter, v: i32) {
    unsafe {
        (d.dbus_message_iter_append_basic)(it, DBUS_TYPE_INT32, &v as *const i32 as *const c_void);
    }
}

/// Appends a plain `u` uint32.
pub(super) unsafe fn append_u32(d: &DBus, it: *mut DBusMessageIter, v: u32) {
    unsafe {
        (d.dbus_message_iter_append_basic)(it, DBUS_TYPE_UINT32, &v as *const u32 as *const c_void);
    }
}

/// Appends a plain `b` boolean (note: the wire type is a 32-bit int).
pub(super) unsafe fn append_bool(d: &DBus, it: *mut DBusMessageIter, v: bool) {
    let b: DBusBool = if v { TRUE } else { FALSE };
    unsafe {
        (d.dbus_message_iter_append_basic)(
            it,
            DBUS_TYPE_BOOLEAN,
            &b as *const DBusBool as *const c_void,
        );
    }
}

/// Appends an `a(iiay)` array holding one `(width, height, ARGB bytes)` entry.
pub(super) unsafe fn append_pixmap(
    d: &DBus,
    it: *mut DBusMessageIter,
    width: i32,
    height: i32,
    argb: &[u8],
) {
    let mut arr = DBusMessageIter::uninit();
    unsafe {
        (d.dbus_message_iter_open_container)(it, DBUS_TYPE_ARRAY, c"(iiay)".as_ptr(), &mut arr);
        let mut st = DBusMessageIter::uninit();
        (d.dbus_message_iter_open_container)(&mut arr, DBUS_TYPE_STRUCT, std::ptr::null(), &mut st);
        append_i32(d, &mut st, width);
        append_i32(d, &mut st, height);
        let mut bytes = DBusMessageIter::uninit();
        (d.dbus_message_iter_open_container)(&mut st, DBUS_TYPE_ARRAY, c"y".as_ptr(), &mut bytes);
        let ptr = argb.as_ptr();
        (d.dbus_message_iter_append_fixed_array)(
            &mut bytes,
            DBUS_TYPE_BYTE,
            &ptr as *const *const u8 as *const c_void,
            argb.len() as std::os::raw::c_int,
        );
        (d.dbus_message_iter_close_container)(&mut st, &mut bytes);
        (d.dbus_message_iter_close_container)(&mut arr, &mut st);
        (d.dbus_message_iter_close_container)(it, &mut arr);
    }
}

/// Opens and immediately closes an empty array of the given element signature.
unsafe fn append_empty_array(d: &DBus, it: *mut DBusMessageIter, element_sig: &CStr) {
    let mut arr = DBusMessageIter::uninit();
    unsafe {
        (d.dbus_message_iter_open_container)(it, DBUS_TYPE_ARRAY, element_sig.as_ptr(), &mut arr);
        (d.dbus_message_iter_close_container)(it, &mut arr);
    }
}

/// Wraps a [`Variant`] value in a D-Bus `variant` and appends it.
pub(super) unsafe fn append_variant(d: &DBus, it: *mut DBusMessageIter, value: &Variant) {
    let sig: &CStr = match value {
        Variant::Str(_) => c"s",
        Variant::ObjectPath(_) => c"o",
        Variant::Bool(_) => c"b",
        Variant::Int32(_) => c"i",
        Variant::Pixmap { .. } => c"a(iiay)",
        Variant::ToolTip { .. } => c"(sa(iiay)ss)",
    };
    let mut var = DBusMessageIter::uninit();
    unsafe {
        (d.dbus_message_iter_open_container)(it, DBUS_TYPE_VARIANT, sig.as_ptr(), &mut var);
        match value {
            Variant::Str(s) => append_str(d, &mut var, s),
            Variant::ObjectPath(s) => append_object_path(d, &mut var, s),
            Variant::Bool(b) => append_bool(d, &mut var, *b),
            Variant::Int32(n) => append_i32(d, &mut var, *n),
            Variant::Pixmap {
                width,
                height,
                argb,
            } => append_pixmap(d, &mut var, *width, *height, argb),
            Variant::ToolTip { title, body } => {
                let mut st = DBusMessageIter::uninit();
                (d.dbus_message_iter_open_container)(
                    &mut var,
                    DBUS_TYPE_STRUCT,
                    std::ptr::null(),
                    &mut st,
                );
                append_str(d, &mut st, c""); // icon name
                append_empty_array(d, &mut st, c"(iiay)"); // icon pixmaps
                append_str(d, &mut st, title);
                append_str(d, &mut st, body);
                (d.dbus_message_iter_close_container)(&mut var, &mut st);
            }
        }
        (d.dbus_message_iter_close_container)(it, &mut var);
    }
}

/// Appends a `{sv}` dict entry (`key` → variant `value`) into an open `a{sv}`.
pub(super) unsafe fn append_dict_entry(
    d: &DBus,
    array: *mut DBusMessageIter,
    key: &CStr,
    value: &Variant,
) {
    let mut entry = DBusMessageIter::uninit();
    unsafe {
        (d.dbus_message_iter_open_container)(
            array,
            DBUS_TYPE_DICT_ENTRY,
            std::ptr::null(),
            &mut entry,
        );
        append_str(d, &mut entry, key);
        append_variant(d, &mut entry, value);
        (d.dbus_message_iter_close_container)(array, &mut entry);
    }
}

/// Reads the leading `string`/`object-path` arguments of a message (stopping at
/// the first non-string), up to `max`. Used for `Properties.Get`/`GetAll` and
/// `NameOwnerChanged`.
pub(super) unsafe fn read_leading_strings(
    d: &DBus,
    msg: *mut DBusMessage,
    max: usize,
) -> Vec<String> {
    let mut it = DBusMessageIter::uninit();
    let mut out = Vec::new();
    unsafe {
        if (d.dbus_message_iter_init)(msg, &mut it) == FALSE {
            return out;
        }
        while out.len() < max {
            let ty = (d.dbus_message_iter_get_arg_type)(&mut it);
            if ty != DBUS_TYPE_STRING && ty != DBUS_TYPE_OBJECT_PATH {
                break;
            }
            let mut p: *const c_char = std::ptr::null();
            (d.dbus_message_iter_get_basic)(&mut it, &mut p as *mut *const c_char as *mut c_void);
            if p.is_null() {
                break;
            }
            out.push(CStr::from_ptr(p).to_string_lossy().into_owned());
            if (d.dbus_message_iter_next)(&mut it) == FALSE {
                break;
            }
        }
    }
    out
}
