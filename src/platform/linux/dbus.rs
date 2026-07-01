//! Minimal FFI bindings to `libdbus-1`, resolved at runtime via `libloading`.
//!
//! This module deliberately declares a *complete* surface of the `dbus_*`
//! functions and types the Linux backend needs across milestones M3–M5 (item
//! export, dbusmenu, notifications). Some entries are therefore not yet
//! referenced, hence the module-wide `dead_code` allow — a binding table is
//! most useful when it is complete and matches upstream exactly.
//!
//! Nothing here links against libdbus. [`DBus::load`] opens `libdbus-1.so.3` at
//! runtime; if it is absent the whole tray simply reports
//! [`crate::Error::LibraryLoad`] and the daemon carries on.
#![allow(dead_code)]

use std::os::raw::{c_char, c_int, c_uint, c_void};

use libloading::{Library, Symbol};

/// SONAME of the D-Bus library. The stable `.so.3` is what every distribution
/// ships; we never rely on the `-dev` `.so` symlink.
const LIB_NAME: &str = "libdbus-1.so.3";

// ---------------------------------------------------------------------------
// Opaque handles and POD structs (ABI-compatible with dbus/dbus.h)
// ---------------------------------------------------------------------------

/// Opaque `DBusConnection`.
pub enum DBusConnection {}
/// Opaque `DBusMessage`.
pub enum DBusMessage {}

/// D-Bus boolean (`dbus_bool_t` is a 32-bit unsigned int). Note this is *not*
/// a Rust `bool`: when appending a boolean value you must pass a `DBusBool`.
pub type DBusBool = u32;
pub const TRUE: DBusBool = 1;
pub const FALSE: DBusBool = 0;

/// Mirror of C `DBusError`. Only `name`/`message` are read; the remaining bytes
/// exist so the struct has the correct size for libdbus to write into.
#[repr(C)]
pub struct DBusError {
    pub name: *const c_char,
    pub message: *const c_char,
    dummy_bits: c_uint,
    padding1: *mut c_void,
}

impl DBusError {
    /// A zeroed error; pass to [`DBus::dbus_error_init`] before use.
    pub fn zeroed() -> DBusError {
        DBusError {
            name: std::ptr::null(),
            message: std::ptr::null(),
            dummy_bits: 0,
            padding1: std::ptr::null_mut(),
        }
    }

    /// Whether libdbus recorded an error in this slot.
    pub fn is_set(&self) -> bool {
        !self.message.is_null()
    }

    /// The error message as a lossy Rust string, or empty if unset.
    pub fn message(&self) -> String {
        if self.message.is_null() {
            String::new()
        } else {
            unsafe { std::ffi::CStr::from_ptr(self.message) }
                .to_string_lossy()
                .into_owned()
        }
    }
}

/// Mirror of C `DBusMessageIter`. The fields are opaque scratch space; we only
/// need the layout to match so libdbus can use it.
#[repr(C)]
pub struct DBusMessageIter {
    dummy1: *mut c_void,
    dummy2: *mut c_void,
    dummy3: u32,
    dummy4: c_int,
    dummy5: c_int,
    dummy6: c_int,
    dummy7: c_int,
    dummy8: c_int,
    dummy9: c_int,
    dummy10: c_int,
    dummy11: c_int,
    pad1: c_int,
    pad2: c_int,
    pad3: *mut c_void,
}

impl DBusMessageIter {
    /// An uninitialised iterator. Every libdbus iterator function fills this in
    /// before reading it, so zeroing is safe and matches idiomatic C usage.
    pub fn uninit() -> DBusMessageIter {
        unsafe { std::mem::zeroed() }
    }
}

/// Callback invoked by libdbus when a message arrives on a registered object
/// path. Returns a `DBUS_HANDLER_RESULT_*` code.
pub type DBusObjectPathMessageFunction =
    unsafe extern "C" fn(*mut DBusConnection, *mut DBusMessage, *mut c_void) -> c_int;
/// Callback invoked when an object path registration is torn down.
pub type DBusObjectPathUnregisterFunction = unsafe extern "C" fn(*mut DBusConnection, *mut c_void);

/// Mirror of C `DBusObjectPathVTable` (two real slots + four reserved).
#[repr(C)]
pub struct DBusObjectPathVTable {
    pub unregister_function: Option<DBusObjectPathUnregisterFunction>,
    pub message_function: Option<DBusObjectPathMessageFunction>,
    pub pad1: Option<unsafe extern "C" fn(*mut c_void)>,
    pub pad2: Option<unsafe extern "C" fn(*mut c_void)>,
    pub pad3: Option<unsafe extern "C" fn(*mut c_void)>,
    pub pad4: Option<unsafe extern "C" fn(*mut c_void)>,
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

// Bus types (DBusBusType).
pub const DBUS_BUS_SESSION: c_int = 0;
pub const DBUS_BUS_SYSTEM: c_int = 1;
pub const DBUS_BUS_STARTER: c_int = 2;

// Message types.
pub const DBUS_MESSAGE_TYPE_INVALID: c_int = 0;
pub const DBUS_MESSAGE_TYPE_METHOD_CALL: c_int = 1;
pub const DBUS_MESSAGE_TYPE_METHOD_RETURN: c_int = 2;
pub const DBUS_MESSAGE_TYPE_ERROR: c_int = 3;
pub const DBUS_MESSAGE_TYPE_SIGNAL: c_int = 4;

// Type codes (the ASCII values from the D-Bus type system).
pub const DBUS_TYPE_INVALID: c_int = 0;
pub const DBUS_TYPE_BYTE: c_int = b'y' as c_int;
pub const DBUS_TYPE_BOOLEAN: c_int = b'b' as c_int;
pub const DBUS_TYPE_INT16: c_int = b'n' as c_int;
pub const DBUS_TYPE_UINT16: c_int = b'q' as c_int;
pub const DBUS_TYPE_INT32: c_int = b'i' as c_int;
pub const DBUS_TYPE_UINT32: c_int = b'u' as c_int;
pub const DBUS_TYPE_INT64: c_int = b'x' as c_int;
pub const DBUS_TYPE_UINT64: c_int = b't' as c_int;
pub const DBUS_TYPE_DOUBLE: c_int = b'd' as c_int;
pub const DBUS_TYPE_STRING: c_int = b's' as c_int;
pub const DBUS_TYPE_OBJECT_PATH: c_int = b'o' as c_int;
pub const DBUS_TYPE_SIGNATURE: c_int = b'g' as c_int;
pub const DBUS_TYPE_ARRAY: c_int = b'a' as c_int;
pub const DBUS_TYPE_VARIANT: c_int = b'v' as c_int;
pub const DBUS_TYPE_STRUCT: c_int = b'r' as c_int;
pub const DBUS_TYPE_DICT_ENTRY: c_int = b'e' as c_int;

// Handler results.
pub const DBUS_HANDLER_RESULT_HANDLED: c_int = 0;
pub const DBUS_HANDLER_RESULT_NOT_YET_HANDLED: c_int = 1;
pub const DBUS_HANDLER_RESULT_NEED_MEMORY: c_int = 2;

// Name-request flags and replies.
pub const DBUS_NAME_FLAG_ALLOW_REPLACEMENT: c_uint = 1;
pub const DBUS_NAME_FLAG_REPLACE_EXISTING: c_uint = 2;
pub const DBUS_NAME_FLAG_DO_NOT_QUEUE: c_uint = 4;
pub const DBUS_REQUEST_NAME_REPLY_PRIMARY_OWNER: c_int = 1;
pub const DBUS_REQUEST_NAME_REPLY_IN_QUEUE: c_int = 2;
pub const DBUS_REQUEST_NAME_REPLY_EXISTS: c_int = 3;
pub const DBUS_REQUEST_NAME_REPLY_ALREADY_OWNER: c_int = 4;

// Timeouts.
pub const DBUS_TIMEOUT_INFINITE: c_int = 0x7fff_ffff;
pub const DBUS_TIMEOUT_USE_DEFAULT: c_int = -1;

// ---------------------------------------------------------------------------
// The loaded binding table
// ---------------------------------------------------------------------------

/// Generates the [`DBus`] struct (one field per `dbus_*` function) plus a
/// [`DBus::load`] that resolves every symbol from the runtime library.
macro_rules! dbus_bindings {
    ( $( fn $name:ident ( $($arg:ty),* $(,)? ) $( -> $ret:ty )? ; )+ ) => {
        /// Runtime-resolved libdbus entry points. Call a function through its
        /// field, e.g. `(dbus.dbus_message_unref)(msg)`.
        #[allow(non_snake_case)]
        pub struct DBus {
            $( pub $name: unsafe extern "C" fn( $($arg),* ) $( -> $ret )?, )+
            // Kept last so it is moved in after every symbol has been resolved;
            // holding the Library alive keeps the function pointers valid.
            _lib: Library,
        }

        impl DBus {
            /// Opens `libdbus-1.so.3` and resolves the full binding table.
            pub fn load() -> crate::error::Result<DBus> {
                let lib = unsafe { Library::new(LIB_NAME) }
                    .map_err(|e| crate::error::Error::LibraryLoad(e.to_string()))?;
                Ok(DBus {
                    $( $name: unsafe {
                        load_sym(&lib, concat!(stringify!($name), "\0").as_bytes())?
                    }, )+
                    _lib: lib,
                })
            }
        }
    };
}

/// Resolves a single symbol and copies its function pointer out. The pointer
/// stays valid as long as the owning [`Library`] is alive (see [`DBus`]).
unsafe fn load_sym<T: Copy>(lib: &Library, symbol: &[u8]) -> crate::error::Result<T> {
    let sym: Symbol<T> = unsafe { lib.get(symbol) }.map_err(|e| {
        let pretty = String::from_utf8_lossy(symbol.strip_suffix(b"\0").unwrap_or(symbol));
        crate::error::Error::LibraryLoad(format!("missing symbol {pretty}: {e}"))
    })?;
    Ok(*sym)
}

dbus_bindings! {
    // Errors
    fn dbus_error_init(*mut DBusError);
    fn dbus_error_free(*mut DBusError);

    // Connection lifecycle
    fn dbus_bus_get_private(c_int, *mut DBusError) -> *mut DBusConnection;
    fn dbus_connection_set_exit_on_disconnect(*mut DBusConnection, DBusBool);
    fn dbus_connection_close(*mut DBusConnection);
    fn dbus_connection_unref(*mut DBusConnection);
    fn dbus_connection_flush(*mut DBusConnection);
    fn dbus_connection_read_write_dispatch(*mut DBusConnection, c_int) -> DBusBool;

    // Sending
    fn dbus_connection_send(*mut DBusConnection, *mut DBusMessage, *mut u32) -> DBusBool;
    fn dbus_connection_send_with_reply_and_block(
        *mut DBusConnection, *mut DBusMessage, c_int, *mut DBusError,
    ) -> *mut DBusMessage;

    // Object path registration
    fn dbus_connection_try_register_object_path(
        *mut DBusConnection, *const c_char, *const DBusObjectPathVTable, *mut c_void, *mut DBusError,
    ) -> DBusBool;
    fn dbus_connection_unregister_object_path(*mut DBusConnection, *const c_char) -> DBusBool;

    // Bus / name management
    fn dbus_bus_get_unique_name(*mut DBusConnection) -> *const c_char;
    fn dbus_bus_request_name(*mut DBusConnection, *const c_char, c_uint, *mut DBusError) -> c_int;
    fn dbus_bus_add_match(*mut DBusConnection, *const c_char, *mut DBusError);

    // Message construction
    fn dbus_message_new_method_call(
        *const c_char, *const c_char, *const c_char, *const c_char,
    ) -> *mut DBusMessage;
    fn dbus_message_new_signal(*const c_char, *const c_char, *const c_char) -> *mut DBusMessage;
    fn dbus_message_new_method_return(*mut DBusMessage) -> *mut DBusMessage;
    fn dbus_message_new_error(*mut DBusMessage, *const c_char, *const c_char) -> *mut DBusMessage;
    fn dbus_message_unref(*mut DBusMessage);

    // Message inspection
    fn dbus_message_get_type(*mut DBusMessage) -> c_int;
    fn dbus_message_get_interface(*mut DBusMessage) -> *const c_char;
    fn dbus_message_get_member(*mut DBusMessage) -> *const c_char;
    fn dbus_message_get_path(*mut DBusMessage) -> *const c_char;
    fn dbus_message_get_sender(*mut DBusMessage) -> *const c_char;
    fn dbus_message_is_method_call(*mut DBusMessage, *const c_char, *const c_char) -> DBusBool;

    // Appending arguments
    fn dbus_message_iter_init_append(*mut DBusMessage, *mut DBusMessageIter);
    fn dbus_message_iter_append_basic(*mut DBusMessageIter, c_int, *const c_void) -> DBusBool;
    fn dbus_message_iter_open_container(
        *mut DBusMessageIter, c_int, *const c_char, *mut DBusMessageIter,
    ) -> DBusBool;
    fn dbus_message_iter_close_container(*mut DBusMessageIter, *mut DBusMessageIter) -> DBusBool;
    fn dbus_message_iter_append_fixed_array(
        *mut DBusMessageIter, c_int, *const c_void, c_int,
    ) -> DBusBool;

    // Reading arguments
    fn dbus_message_iter_init(*mut DBusMessage, *mut DBusMessageIter) -> DBusBool;
    fn dbus_message_iter_next(*mut DBusMessageIter) -> DBusBool;
    fn dbus_message_iter_get_arg_type(*mut DBusMessageIter) -> c_int;
    fn dbus_message_iter_get_basic(*mut DBusMessageIter, *mut c_void);
    fn dbus_message_iter_recurse(*mut DBusMessageIter, *mut DBusMessageIter);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_symbols_resolve_when_library_present() {
        // Skip gracefully where libdbus is not installed (headless/minimal CI).
        if unsafe { Library::new(LIB_NAME) }.is_err() {
            eprintln!("{LIB_NAME} not present; skipping symbol-resolution test");
            return;
        }
        DBus::load().expect("every dbus_* symbol must resolve when libdbus is present");
    }

    #[test]
    fn error_helpers() {
        let err = DBusError::zeroed();
        assert!(!err.is_set());
        assert_eq!(err.message(), "");
    }
}
