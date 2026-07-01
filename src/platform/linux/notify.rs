//! Desktop notifications via `org.freedesktop.Notifications.Notify`.
//!
//! A single fire-and-forget method call to the session notification daemon. An
//! attached icon is passed inline through the `image-data` hint (raw RGBA, which
//! is exactly our [`Icon`](crate::Icon) format — no conversion needed, unlike
//! the SNI pixmap).

use std::ffi::CStr;
use std::os::raw::{c_int, c_void};

use super::dbus::*;
use super::{cstring, msg};
use crate::error::{Error, Result};
use crate::notification::Notification;

const NOTIFY_NAME: &CStr = c"org.freedesktop.Notifications";
const NOTIFY_PATH: &CStr = c"/org/freedesktop/Notifications";

/// Sends `notification` to the notification daemon. Best effort: if no daemon is
/// present the call is simply dropped by the bus.
pub(super) unsafe fn send(
    d: &DBus,
    conn: *mut DBusConnection,
    app_name: &CStr,
    notification: &Notification,
) -> Result<()> {
    let summary = cstring(&notification.title);
    let body = cstring(&notification.body);

    unsafe {
        let msg = (d.dbus_message_new_method_call)(
            NOTIFY_NAME.as_ptr(),
            NOTIFY_PATH.as_ptr(),
            NOTIFY_NAME.as_ptr(),
            c"Notify".as_ptr(),
        );
        if msg.is_null() {
            return Err(Error::Backend("failed to build Notify message".into()));
        }

        let mut it = DBusMessageIter::uninit();
        (d.dbus_message_iter_init_append)(msg, &mut it);
        msg::append_str(d, &mut it, app_name); // app_name
        msg::append_u32(d, &mut it, 0); // replaces_id (0 = new)
        msg::append_str(d, &mut it, c""); // app_icon (we use image-data instead)
        msg::append_str(d, &mut it, &summary); // summary
        msg::append_str(d, &mut it, &body); // body

        // actions: empty `as`
        let mut actions = DBusMessageIter::uninit();
        (d.dbus_message_iter_open_container)(&mut it, DBUS_TYPE_ARRAY, c"s".as_ptr(), &mut actions);
        (d.dbus_message_iter_close_container)(&mut it, &mut actions);

        // hints: `a{sv}`, optionally carrying the inline image.
        let mut hints = DBusMessageIter::uninit();
        (d.dbus_message_iter_open_container)(
            &mut it,
            DBUS_TYPE_ARRAY,
            c"{sv}".as_ptr(),
            &mut hints,
        );
        if let Some(icon) = &notification.icon {
            append_image_data(
                d,
                &mut hints,
                icon.width as i32,
                icon.height as i32,
                &icon.rgba,
            );
        }
        (d.dbus_message_iter_close_container)(&mut it, &mut hints);

        msg::append_i32(d, &mut it, -1); // expire_timeout (-1 = default)

        (d.dbus_connection_send)(conn, msg, std::ptr::null_mut());
        (d.dbus_message_unref)(msg);
        (d.dbus_connection_flush)(conn);
    }
    Ok(())
}

/// Appends the `image-data` hint: `{ "image-data": (iiibiiay) }`, where the
/// struct is `(width, height, rowstride, has_alpha, bits_per_sample, channels,
/// data)` with `data` in raw RGBA byte order.
unsafe fn append_image_data(
    d: &DBus,
    hints: *mut DBusMessageIter,
    width: i32,
    height: i32,
    rgba: &[u8],
) {
    unsafe {
        let mut entry = DBusMessageIter::uninit();
        (d.dbus_message_iter_open_container)(
            hints,
            DBUS_TYPE_DICT_ENTRY,
            std::ptr::null(),
            &mut entry,
        );
        msg::append_str(d, &mut entry, c"image-data");

        let mut var = DBusMessageIter::uninit();
        (d.dbus_message_iter_open_container)(
            &mut entry,
            DBUS_TYPE_VARIANT,
            c"(iiibiiay)".as_ptr(),
            &mut var,
        );
        let mut st = DBusMessageIter::uninit();
        (d.dbus_message_iter_open_container)(&mut var, DBUS_TYPE_STRUCT, std::ptr::null(), &mut st);
        msg::append_i32(d, &mut st, width);
        msg::append_i32(d, &mut st, height);
        msg::append_i32(d, &mut st, width * 4); // rowstride
        msg::append_bool(d, &mut st, true); // has_alpha
        msg::append_i32(d, &mut st, 8); // bits_per_sample
        msg::append_i32(d, &mut st, 4); // channels (RGBA)

        let mut bytes = DBusMessageIter::uninit();
        (d.dbus_message_iter_open_container)(&mut st, DBUS_TYPE_ARRAY, c"y".as_ptr(), &mut bytes);
        let ptr = rgba.as_ptr();
        (d.dbus_message_iter_append_fixed_array)(
            &mut bytes,
            DBUS_TYPE_BYTE,
            &ptr as *const *const u8 as *const c_void,
            rgba.len() as c_int,
        );
        (d.dbus_message_iter_close_container)(&mut st, &mut bytes);

        (d.dbus_message_iter_close_container)(&mut var, &mut st);
        (d.dbus_message_iter_close_container)(&mut entry, &mut var);
        (d.dbus_message_iter_close_container)(hints, &mut entry);
    }
}
