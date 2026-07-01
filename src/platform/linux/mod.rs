//! Linux tray backend: freedesktop/KDE StatusNotifierItem (SNI) over D-Bus.
//!
//! We export an `org.kde.StatusNotifierItem` object on the session bus and
//! register it with the `org.kde.StatusNotifierWatcher` host (the panel/tray).
//! Clicks arrive as `Activate`/`SecondaryActivate`/`ContextMenu` method calls
//! and are turned into [`Event`]s. Everything goes through `libdbus`, loaded at
//! runtime by [`dbus::DBus::load`] — nothing here is linked.

mod dbus;
mod menu;
mod msg;
mod notify;

use std::collections::HashSet;
use std::ffi::{CStr, CString};
use std::os::raw::{c_int, c_void};
use std::time::Duration;

use dbus::*;

use super::{Backend, Init};
use crate::error::{Error, Result};
use crate::event::Event;
use crate::icon::Icon;
use crate::menu::{Menu, MenuId};
use crate::notification::{ActionId, Notification};

const ITEM_PATH: &CStr = c"/StatusNotifierItem";
const ITEM_IFACE: &CStr = c"org.kde.StatusNotifierItem";
const PROPERTIES_IFACE: &CStr = c"org.freedesktop.DBus.Properties";
const INTROSPECTABLE_IFACE: &CStr = c"org.freedesktop.DBus.Introspectable";
const WATCHER_NAME: &CStr = c"org.kde.StatusNotifierWatcher";
const WATCHER_PATH: &CStr = c"/StatusNotifierWatcher";
const MENU_PATH: &CStr = c"/MenuBar";
const MENU_IFACE: &CStr = c"com.canonical.dbusmenu";
const NOTIFICATIONS_IFACE: &CStr = c"org.freedesktop.Notifications";

/// Minimal, valid introspection data — enough for hosts and `d-feet`/`busctl`.
const INTROSPECT_XML: &CStr = c"<!DOCTYPE node PUBLIC \"-//freedesktop//DTD D-BUS Object Introspection 1.0//EN\" \"http://www.freedesktop.org/standards/dbus/1.0/introspect.dtd\">\n<node>\n <interface name=\"org.kde.StatusNotifierItem\">\n  <method name=\"Activate\"><arg name=\"x\" type=\"i\" direction=\"in\"/><arg name=\"y\" type=\"i\" direction=\"in\"/></method>\n  <method name=\"SecondaryActivate\"><arg name=\"x\" type=\"i\" direction=\"in\"/><arg name=\"y\" type=\"i\" direction=\"in\"/></method>\n  <method name=\"ContextMenu\"><arg name=\"x\" type=\"i\" direction=\"in\"/><arg name=\"y\" type=\"i\" direction=\"in\"/></method>\n  <method name=\"Scroll\"><arg name=\"delta\" type=\"i\" direction=\"in\"/><arg name=\"orientation\" type=\"s\" direction=\"in\"/></method>\n  <signal name=\"NewIcon\"/>\n  <signal name=\"NewToolTip\"/>\n  <signal name=\"NewTitle\"/>\n  <signal name=\"NewStatus\"><arg name=\"status\" type=\"s\"/></signal>\n  <property name=\"Category\" type=\"s\" access=\"read\"/>\n  <property name=\"Id\" type=\"s\" access=\"read\"/>\n  <property name=\"Title\" type=\"s\" access=\"read\"/>\n  <property name=\"Status\" type=\"s\" access=\"read\"/>\n  <property name=\"IconName\" type=\"s\" access=\"read\"/>\n  <property name=\"IconPixmap\" type=\"a(iiay)\" access=\"read\"/>\n  <property name=\"ToolTip\" type=\"(sa(iiay)ss)\" access=\"read\"/>\n  <property name=\"ItemIsMenu\" type=\"b\" access=\"read\"/>\n  <property name=\"Menu\" type=\"o\" access=\"read\"/>\n </interface>\n</node>\n";

/// Builds the Linux backend, or returns a graceful error if there is no session
/// bus / libdbus (e.g. a headless server).
pub(crate) fn new(init: Init) -> Result<Box<dyn Backend>> {
    let backend = LinuxBackend::new(init)?;
    Ok(Box::new(backend))
}

/// The mutable tray state, kept behind a stable heap address so it can be handed
/// to libdbus as `user_data` for the object-path handler and the message filter.
struct State {
    dbus: DBus,
    conn: *mut DBusConnection,
    /// Bus name we registered the item under.
    service: CString,
    // Cached StatusNotifierItem property values.
    id: CString,
    title: CString,
    status: CString,
    category: CString,
    menu_path: CString,
    icon_w: i32,
    icon_h: i32,
    icon_argb: Vec<u8>,
    /// The context menu, rendered as a dbusmenu node tree at `menu_path`.
    menu_model: menu::DbusMenu,
    /// Ids of our outstanding notifications that carry actions, so `ActionInvoked`
    /// signals from other apps' notifications are ignored.
    notify_ids: HashSet<u32>,
    /// Interactions collected during dispatch, drained by `pump`.
    pending: Vec<Event>,
}

// The state is only ever touched from the single event-loop thread; libdbus's
// private connection is likewise used from one thread. Moving the box between
// `Tray::new` and a `spawn`ed loop thread is safe.
unsafe impl Send for State {}

/// Public backend wrapper owning the boxed [`State`].
pub(crate) struct LinuxBackend {
    state: Box<State>,
}

impl LinuxBackend {
    fn new(init: Init) -> Result<LinuxBackend> {
        let dbus = DBus::load()?;
        let mut err = DBusError::zeroed();
        // SAFETY: err is a correctly sized DBusError; conn checked for null.
        let conn = unsafe {
            (dbus.dbus_error_init)(&mut err);
            (dbus.dbus_bus_get_private)(DBUS_BUS_SESSION, &mut err)
        };
        if conn.is_null() {
            let message = err.message();
            unsafe { (dbus.dbus_error_free)(&mut err) };
            return Err(Error::Backend(format!(
                "cannot connect to the session bus: {message}"
            )));
        }
        unsafe { (dbus.dbus_connection_set_exit_on_disconnect)(conn, FALSE) };

        // Prefer the conventional well-known name; fall back to the unique name.
        let pid = std::process::id();
        let wk = cstring(&format!("org.kde.StatusNotifierItem-{pid}-1"));
        let reply = unsafe {
            (dbus.dbus_bus_request_name)(conn, wk.as_ptr(), DBUS_NAME_FLAG_DO_NOT_QUEUE, &mut err)
        };
        let service = if reply == DBUS_REQUEST_NAME_REPLY_PRIMARY_OWNER
            || reply == DBUS_REQUEST_NAME_REPLY_ALREADY_OWNER
        {
            wk
        } else {
            let unique = unsafe { (dbus.dbus_bus_get_unique_name)(conn) };
            if unique.is_null() {
                wk
            } else {
                unsafe { CStr::from_ptr(unique) }.to_owned()
            }
        };
        unsafe { (dbus.dbus_error_free)(&mut err) };

        let id = std::env::current_exe()
            .ok()
            .and_then(|p| p.file_name().map(|s| s.to_string_lossy().into_owned()))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "ldtray".to_string());
        let title = if init.tooltip.is_empty() {
            id.clone()
        } else {
            init.tooltip.clone()
        };
        let (icon_w, icon_h, icon_argb) = rgba_to_argb(&init.icon);
        let menu_model = menu::DbusMenu::build(init.menu.as_ref(), 1);

        let mut state = Box::new(State {
            dbus,
            conn,
            service,
            id: cstring(&id),
            title: cstring(&title),
            status: cstring("Active"),
            category: cstring("ApplicationStatus"),
            menu_path: cstring("/MenuBar"),
            icon_w,
            icon_h,
            icon_argb,
            menu_model,
            notify_ids: HashSet::new(),
            pending: Vec::new(),
        });

        let state_ptr = (&mut *state as *mut State) as *mut c_void;

        // Register the item and the dbusmenu object. libdbus copies each vtable,
        // so locals are fine.
        let item_vtable = object_vtable(item_message_handler);
        let menu_vtable = object_vtable(menu_message_handler);
        let registered = unsafe {
            let item_ok = (state.dbus.dbus_connection_try_register_object_path)(
                conn,
                ITEM_PATH.as_ptr(),
                &item_vtable,
                state_ptr,
                &mut err,
            );
            if item_ok == FALSE {
                FALSE
            } else {
                (state.dbus.dbus_connection_try_register_object_path)(
                    conn,
                    MENU_PATH.as_ptr(),
                    &menu_vtable,
                    state_ptr,
                    &mut err,
                )
            }
        };
        if registered == FALSE {
            let message = err.message();
            unsafe { (state.dbus.dbus_error_free)(&mut err) };
            // `state` drops here, closing the connection.
            return Err(Error::Backend(format!(
                "failed to export tray objects: {message}"
            )));
        }

        // Notice a watcher that starts *after* us (daemon booted before the
        // panel), and receive notification action callbacks.
        let watcher_rule = c"type='signal',sender='org.freedesktop.DBus',interface='org.freedesktop.DBus',member='NameOwnerChanged',arg0='org.kde.StatusNotifierWatcher'";
        let action_rule =
            c"type='signal',interface='org.freedesktop.Notifications',member='ActionInvoked'";
        let closed_rule =
            c"type='signal',interface='org.freedesktop.Notifications',member='NotificationClosed'";
        unsafe {
            for rule in [watcher_rule, action_rule, closed_rule] {
                (state.dbus.dbus_bus_add_match)(conn, rule.as_ptr(), &mut err);
                (state.dbus.dbus_error_free)(&mut err);
            }
            (state.dbus.dbus_connection_add_filter)(conn, signal_filter, state_ptr, None);
        }

        // Best-effort registration with whatever host is already present.
        unsafe { state.register_with_watcher() };
        unsafe { (state.dbus.dbus_connection_flush)(conn) };

        Ok(LinuxBackend { state })
    }
}

impl State {
    /// Sends `RegisterStatusNotifierItem(service)` to the watcher (best effort).
    unsafe fn register_with_watcher(&self) {
        unsafe {
            let msg = (self.dbus.dbus_message_new_method_call)(
                WATCHER_NAME.as_ptr(),
                WATCHER_PATH.as_ptr(),
                WATCHER_NAME.as_ptr(),
                c"RegisterStatusNotifierItem".as_ptr(),
            );
            if msg.is_null() {
                return;
            }
            let mut it = DBusMessageIter::uninit();
            (self.dbus.dbus_message_iter_init_append)(msg, &mut it);
            msg::append_str(&self.dbus, &mut it, &self.service);
            (self.dbus.dbus_connection_send)(self.conn, msg, std::ptr::null_mut());
            (self.dbus.dbus_message_unref)(msg);
            (self.dbus.dbus_connection_flush)(self.conn);
        }
    }

    /// Dispatches a message delivered to `/StatusNotifierItem`.
    unsafe fn handle_item_message(&mut self, msg: *mut DBusMessage) -> c_int {
        let iface = unsafe { ptr_to_cstr((self.dbus.dbus_message_get_interface)(msg)) };
        let member = unsafe { ptr_to_cstr((self.dbus.dbus_message_get_member)(msg)) };
        let (Some(iface), Some(member)) = (iface, member) else {
            return DBUS_HANDLER_RESULT_NOT_YET_HANDLED;
        };

        if iface == PROPERTIES_IFACE {
            if member == c"Get" {
                return unsafe { self.reply_get(msg) };
            }
            if member == c"GetAll" {
                return unsafe { self.reply_get_all(msg) };
            }
            return DBUS_HANDLER_RESULT_NOT_YET_HANDLED;
        }

        if iface == ITEM_IFACE {
            let event = if member == c"Activate" {
                Some(Event::LeftClick)
            } else if member == c"SecondaryActivate" {
                Some(Event::MiddleClick)
            } else if member == c"ContextMenu" {
                Some(Event::RightClick)
            } else if member == c"Scroll" {
                None
            } else {
                return DBUS_HANDLER_RESULT_NOT_YET_HANDLED;
            };
            if let Some(event) = event {
                self.pending.push(event);
            }
            unsafe { self.send_empty_return(msg) };
            return DBUS_HANDLER_RESULT_HANDLED;
        }

        if iface == INTROSPECTABLE_IFACE && member == c"Introspect" {
            unsafe { self.reply_introspect(msg) };
            return DBUS_HANDLER_RESULT_HANDLED;
        }

        DBUS_HANDLER_RESULT_NOT_YET_HANDLED
    }

    /// Dispatches a message delivered to `/MenuBar` (the dbusmenu object).
    unsafe fn handle_menu_message(&mut self, msg: *mut DBusMessage) -> c_int {
        let iface = unsafe { ptr_to_cstr((self.dbus.dbus_message_get_interface)(msg)) };
        let member = unsafe { ptr_to_cstr((self.dbus.dbus_message_get_member)(msg)) };
        let (Some(iface), Some(member)) = (iface, member) else {
            return DBUS_HANDLER_RESULT_NOT_YET_HANDLED;
        };

        if iface == PROPERTIES_IFACE {
            if member == c"Get" {
                return unsafe { self.reply_menu_get(msg) };
            }
            if member == c"GetAll" {
                return unsafe { self.reply_menu_get_all(msg) };
            }
            return DBUS_HANDLER_RESULT_NOT_YET_HANDLED;
        }

        if iface == MENU_IFACE {
            if member == c"GetLayout" {
                return unsafe { self.reply_get_layout(msg) };
            }
            if member == c"GetGroupProperties" {
                return unsafe { self.reply_get_group_properties(msg) };
            }
            if member == c"Event" {
                unsafe { self.handle_menu_event(msg) };
                unsafe { self.send_empty_return(msg) };
                return DBUS_HANDLER_RESULT_HANDLED;
            }
            if member == c"AboutToShow" {
                return unsafe { self.reply_about_to_show(msg) };
            }
            // GetProperty/EventGroup/AboutToShowGroup are optional; let libdbus
            // reply UnknownMethod for anything we do not implement.
            return DBUS_HANDLER_RESULT_NOT_YET_HANDLED;
        }

        DBUS_HANDLER_RESULT_NOT_YET_HANDLED
    }

    unsafe fn reply_get_layout(&self, call: *mut DBusMessage) -> c_int {
        let (parent, depth) = unsafe {
            let mut it = msg::iter_init(&self.dbus, call);
            let parent = msg::read_i32(&self.dbus, &mut it).unwrap_or(0);
            msg::advance(&self.dbus, &mut it);
            let depth = msg::read_i32(&self.dbus, &mut it).unwrap_or(-1);
            (parent, depth)
        };
        unsafe {
            let reply = (self.dbus.dbus_message_new_method_return)(call);
            if reply.is_null() {
                return DBUS_HANDLER_RESULT_NEED_MEMORY;
            }
            let mut it = DBusMessageIter::uninit();
            (self.dbus.dbus_message_iter_init_append)(reply, &mut it);
            self.menu_model
                .append_get_layout(&self.dbus, &mut it, parent, depth);
            self.send(reply);
        }
        DBUS_HANDLER_RESULT_HANDLED
    }

    unsafe fn reply_get_group_properties(&self, call: *mut DBusMessage) -> c_int {
        let ids = unsafe {
            let mut it = msg::iter_init(&self.dbus, call);
            msg::read_i32_array(&self.dbus, &mut it)
        };
        unsafe {
            let reply = (self.dbus.dbus_message_new_method_return)(call);
            if reply.is_null() {
                return DBUS_HANDLER_RESULT_NEED_MEMORY;
            }
            let mut it = DBusMessageIter::uninit();
            (self.dbus.dbus_message_iter_init_append)(reply, &mut it);
            self.menu_model
                .append_group_properties(&self.dbus, &mut it, &ids);
            self.send(reply);
        }
        DBUS_HANDLER_RESULT_HANDLED
    }

    /// Reads `Event(id, eventId, ...)` and, on `clicked`, queues a menu event.
    unsafe fn handle_menu_event(&mut self, call: *mut DBusMessage) {
        let (id, event_id) = unsafe {
            let mut it = msg::iter_init(&self.dbus, call);
            let id = msg::read_i32(&self.dbus, &mut it);
            let event_id = if id.is_some() && msg::advance(&self.dbus, &mut it) {
                msg::read_string(&self.dbus, &mut it)
            } else {
                None
            };
            (id, event_id)
        };
        if let (Some(id), Some(event_id)) = (id, event_id) {
            if event_id == "clicked" {
                if let Some(menu_id) = self.menu_model.menu_id_for(id) {
                    self.pending.push(Event::Menu(MenuId(menu_id)));
                }
            }
        }
    }

    /// `AboutToShow(id) -> needUpdate: b`; our layout is always current.
    unsafe fn reply_about_to_show(&self, call: *mut DBusMessage) -> c_int {
        unsafe {
            let reply = (self.dbus.dbus_message_new_method_return)(call);
            if reply.is_null() {
                return DBUS_HANDLER_RESULT_NEED_MEMORY;
            }
            let mut it = DBusMessageIter::uninit();
            (self.dbus.dbus_message_iter_init_append)(reply, &mut it);
            msg::append_bool(&self.dbus, &mut it, false);
            self.send(reply);
        }
        DBUS_HANDLER_RESULT_HANDLED
    }

    /// The dbusmenu object's own properties (`Version`/`Status`/`TextDirection`).
    fn menu_property(&self, name: &str) -> Option<msg::Variant<'_>> {
        use msg::Variant;
        Some(match name {
            "Version" => Variant::UInt32(3),
            "Status" => Variant::Str(c"normal"),
            "TextDirection" => Variant::Str(c"ltr"),
            _ => return None,
        })
    }

    unsafe fn reply_menu_get(&self, call: *mut DBusMessage) -> c_int {
        let args = unsafe { msg::read_leading_strings(&self.dbus, call, 2) };
        if args.len() < 2 {
            return unsafe {
                self.send_error(
                    call,
                    c"org.freedesktop.DBus.Error.InvalidArgs",
                    c"expected interface and property",
                )
            };
        }
        match self.menu_property(&args[1]) {
            Some(value) => unsafe {
                let reply = (self.dbus.dbus_message_new_method_return)(call);
                if reply.is_null() {
                    return DBUS_HANDLER_RESULT_NEED_MEMORY;
                }
                let mut it = DBusMessageIter::uninit();
                (self.dbus.dbus_message_iter_init_append)(reply, &mut it);
                msg::append_variant(&self.dbus, &mut it, &value);
                self.send(reply);
                DBUS_HANDLER_RESULT_HANDLED
            },
            None => unsafe {
                self.send_error(
                    call,
                    c"org.freedesktop.DBus.Error.UnknownProperty",
                    c"no such property",
                )
            },
        }
    }

    unsafe fn reply_menu_get_all(&self, call: *mut DBusMessage) -> c_int {
        unsafe {
            let reply = (self.dbus.dbus_message_new_method_return)(call);
            if reply.is_null() {
                return DBUS_HANDLER_RESULT_NEED_MEMORY;
            }
            let mut it = DBusMessageIter::uninit();
            (self.dbus.dbus_message_iter_init_append)(reply, &mut it);
            let mut arr = DBusMessageIter::uninit();
            (self.dbus.dbus_message_iter_open_container)(
                &mut it,
                DBUS_TYPE_ARRAY,
                c"{sv}".as_ptr(),
                &mut arr,
            );
            for key in ["Version", "Status", "TextDirection"] {
                if let Some(value) = self.menu_property(key) {
                    let ckey = cstring(key);
                    msg::append_dict_entry(&self.dbus, &mut arr, &ckey, &value);
                }
            }
            (self.dbus.dbus_message_iter_close_container)(&mut it, &mut arr);
            self.send(reply);
        }
        DBUS_HANDLER_RESULT_HANDLED
    }

    /// Emits `LayoutUpdated(revision, 0)` so the host re-fetches the menu.
    unsafe fn emit_layout_updated(&self) {
        unsafe {
            let signal = (self.dbus.dbus_message_new_signal)(
                MENU_PATH.as_ptr(),
                MENU_IFACE.as_ptr(),
                c"LayoutUpdated".as_ptr(),
            );
            if signal.is_null() {
                return;
            }
            let mut it = DBusMessageIter::uninit();
            (self.dbus.dbus_message_iter_init_append)(signal, &mut it);
            msg::append_u32(&self.dbus, &mut it, self.menu_model.revision);
            msg::append_i32(&self.dbus, &mut it, 0);
            self.send(signal);
        }
    }

    /// Returns the variant for a single StatusNotifierItem property.
    fn item_property(&self, name: &str) -> Option<msg::Variant<'_>> {
        use msg::Variant;
        Some(match name {
            "Category" => Variant::Str(&self.category),
            "Id" => Variant::Str(&self.id),
            "Title" => Variant::Str(&self.title),
            "Status" => Variant::Str(&self.status),
            "WindowId" => Variant::Int32(0),
            "IconName" | "OverlayIconName" | "AttentionIconName" | "AttentionMovieName"
            | "IconThemePath" => Variant::Str(c""),
            "IconPixmap" => Variant::Pixmap {
                width: self.icon_w,
                height: self.icon_h,
                argb: &self.icon_argb,
            },
            "ToolTip" => Variant::ToolTip {
                title: &self.title,
                body: c"",
            },
            "ItemIsMenu" => Variant::Bool(false),
            "Menu" => Variant::ObjectPath(&self.menu_path),
            _ => return None,
        })
    }

    /// All properties, in a fixed order, for `GetAll`.
    fn item_properties(&self) -> [(&'static CStr, msg::Variant<'_>); 9] {
        use msg::Variant;
        [
            (c"Category", Variant::Str(&self.category)),
            (c"Id", Variant::Str(&self.id)),
            (c"Title", Variant::Str(&self.title)),
            (c"Status", Variant::Str(&self.status)),
            (c"IconName", Variant::Str(c"")),
            (
                c"IconPixmap",
                Variant::Pixmap {
                    width: self.icon_w,
                    height: self.icon_h,
                    argb: &self.icon_argb,
                },
            ),
            (
                c"ToolTip",
                Variant::ToolTip {
                    title: &self.title,
                    body: c"",
                },
            ),
            (c"ItemIsMenu", Variant::Bool(false)),
            (c"Menu", Variant::ObjectPath(&self.menu_path)),
        ]
    }

    unsafe fn reply_get(&self, call: *mut DBusMessage) -> c_int {
        let args = unsafe { msg::read_leading_strings(&self.dbus, call, 2) };
        if args.len() < 2 {
            return unsafe {
                self.send_error(
                    call,
                    c"org.freedesktop.DBus.Error.InvalidArgs",
                    c"expected interface and property",
                )
            };
        }
        match self.item_property(&args[1]) {
            Some(value) => unsafe {
                let reply = (self.dbus.dbus_message_new_method_return)(call);
                if reply.is_null() {
                    return DBUS_HANDLER_RESULT_NEED_MEMORY;
                }
                let mut it = DBusMessageIter::uninit();
                (self.dbus.dbus_message_iter_init_append)(reply, &mut it);
                msg::append_variant(&self.dbus, &mut it, &value);
                self.send(reply);
                DBUS_HANDLER_RESULT_HANDLED
            },
            None => unsafe {
                self.send_error(
                    call,
                    c"org.freedesktop.DBus.Error.UnknownProperty",
                    c"no such property",
                )
            },
        }
    }

    unsafe fn reply_get_all(&self, call: *mut DBusMessage) -> c_int {
        unsafe {
            let reply = (self.dbus.dbus_message_new_method_return)(call);
            if reply.is_null() {
                return DBUS_HANDLER_RESULT_NEED_MEMORY;
            }
            let mut it = DBusMessageIter::uninit();
            (self.dbus.dbus_message_iter_init_append)(reply, &mut it);
            let mut arr = DBusMessageIter::uninit();
            (self.dbus.dbus_message_iter_open_container)(
                &mut it,
                DBUS_TYPE_ARRAY,
                c"{sv}".as_ptr(),
                &mut arr,
            );
            for (key, value) in self.item_properties() {
                msg::append_dict_entry(&self.dbus, &mut arr, key, &value);
            }
            (self.dbus.dbus_message_iter_close_container)(&mut it, &mut arr);
            self.send(reply);
            DBUS_HANDLER_RESULT_HANDLED
        }
    }

    unsafe fn reply_introspect(&self, call: *mut DBusMessage) {
        unsafe {
            let reply = (self.dbus.dbus_message_new_method_return)(call);
            if reply.is_null() {
                return;
            }
            let mut it = DBusMessageIter::uninit();
            (self.dbus.dbus_message_iter_init_append)(reply, &mut it);
            msg::append_str(&self.dbus, &mut it, INTROSPECT_XML);
            self.send(reply);
        }
    }

    /// Sends an empty method-return for a fire-and-forget call.
    unsafe fn send_empty_return(&self, call: *mut DBusMessage) {
        unsafe {
            let reply = (self.dbus.dbus_message_new_method_return)(call);
            if !reply.is_null() {
                self.send(reply);
            }
        }
    }

    unsafe fn send_error(&self, call: *mut DBusMessage, name: &CStr, text: &CStr) -> c_int {
        unsafe {
            let reply = (self.dbus.dbus_message_new_error)(call, name.as_ptr(), text.as_ptr());
            if !reply.is_null() {
                self.send(reply);
            }
        }
        DBUS_HANDLER_RESULT_HANDLED
    }

    /// Queues a message for delivery, takes ownership (unrefs), and flushes so
    /// replies leave promptly even though we are inside a dispatch callback.
    unsafe fn send(&self, msg: *mut DBusMessage) {
        unsafe {
            (self.dbus.dbus_connection_send)(self.conn, msg, std::ptr::null_mut());
            (self.dbus.dbus_message_unref)(msg);
            (self.dbus.dbus_connection_flush)(self.conn);
        }
    }

    /// Emits a no-argument StatusNotifierItem signal (e.g. `NewIcon`).
    unsafe fn emit(&self, member: &CStr) {
        unsafe {
            let signal = (self.dbus.dbus_message_new_signal)(
                ITEM_PATH.as_ptr(),
                ITEM_IFACE.as_ptr(),
                member.as_ptr(),
            );
            if signal.is_null() {
                return;
            }
            (self.dbus.dbus_connection_send)(self.conn, signal, std::ptr::null_mut());
            (self.dbus.dbus_message_unref)(signal);
            (self.dbus.dbus_connection_flush)(self.conn);
        }
    }
}

impl Drop for State {
    fn drop(&mut self) {
        let state_ptr = (self as *mut State) as *mut c_void;
        unsafe {
            (self.dbus.dbus_connection_unregister_object_path)(self.conn, ITEM_PATH.as_ptr());
            (self.dbus.dbus_connection_unregister_object_path)(self.conn, MENU_PATH.as_ptr());
            (self.dbus.dbus_connection_remove_filter)(self.conn, signal_filter, state_ptr);
            (self.dbus.dbus_connection_close)(self.conn);
            (self.dbus.dbus_connection_unref)(self.conn);
        }
    }
}

impl Backend for LinuxBackend {
    fn set_icon(&mut self, icon: &Icon) -> Result<()> {
        let (w, h, argb) = rgba_to_argb(icon);
        self.state.icon_w = w;
        self.state.icon_h = h;
        self.state.icon_argb = argb;
        unsafe { self.state.emit(c"NewIcon") };
        Ok(())
    }

    fn set_tooltip(&mut self, text: &str) -> Result<()> {
        self.state.title = cstring(text);
        unsafe {
            self.state.emit(c"NewToolTip");
            self.state.emit(c"NewTitle");
        }
        Ok(())
    }

    fn set_menu(&mut self, menu: Option<&Menu>) -> Result<()> {
        let revision = self.state.menu_model.revision.wrapping_add(1);
        self.state.menu_model = menu::DbusMenu::build(menu, revision);
        unsafe { self.state.emit_layout_updated() };
        Ok(())
    }

    fn notify(&mut self, notification: &Notification) -> Result<()> {
        let id = unsafe {
            notify::send(
                &self.state.dbus,
                self.state.conn,
                &self.state.id,
                notification,
            )?
        };
        // Remember notifications that carry actions so we can route their
        // ActionInvoked callbacks (and ignore other apps' notifications).
        if !notification.actions.is_empty() && id != 0 {
            self.state.notify_ids.insert(id);
        }
        Ok(())
    }

    fn pump(&mut self, timeout: Duration, sink: &mut dyn FnMut(Event)) -> Result<()> {
        // Copy the fn pointer and connection out so no Rust borrow of `state` is
        // held while libdbus re-enters our handler (which forms its own &mut).
        let dispatch = self.state.dbus.dbus_connection_read_write_dispatch;
        let conn = self.state.conn;
        let ms = timeout.as_millis().min(i32::MAX as u128) as c_int;
        let alive = unsafe { dispatch(conn, ms) };
        for event in self.state.pending.drain(..) {
            sink(event);
        }
        if alive == FALSE {
            return Err(Error::Backend("session bus disconnected".into()));
        }
        Ok(())
    }
}

/// libdbus object-path callback for `/StatusNotifierItem`.
unsafe extern "C" fn item_message_handler(
    _conn: *mut DBusConnection,
    msg: *mut DBusMessage,
    user_data: *mut c_void,
) -> c_int {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let state = unsafe { &mut *(user_data as *mut State) };
        unsafe { state.handle_item_message(msg) }
    }));
    result.unwrap_or(DBUS_HANDLER_RESULT_NOT_YET_HANDLED)
}

/// libdbus object-path callback for `/MenuBar` (dbusmenu).
unsafe extern "C" fn menu_message_handler(
    _conn: *mut DBusConnection,
    msg: *mut DBusMessage,
    user_data: *mut c_void,
) -> c_int {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let state = unsafe { &mut *(user_data as *mut State) };
        unsafe { state.handle_menu_message(msg) }
    }));
    result.unwrap_or(DBUS_HANDLER_RESULT_NOT_YET_HANDLED)
}

/// Builds an object-path vtable with just a message handler (libdbus copies it).
fn object_vtable(handler: DBusObjectPathMessageFunction) -> DBusObjectPathVTable {
    DBusObjectPathVTable {
        unregister_function: None,
        message_function: Some(handler),
        pad1: None,
        pad2: None,
        pad3: None,
        pad4: None,
    }
}

/// libdbus connection filter: re-register when the watcher (re)appears, and
/// route notification `ActionInvoked` / `NotificationClosed` signals.
unsafe extern "C" fn signal_filter(
    _conn: *mut DBusConnection,
    msg: *mut DBusMessage,
    user_data: *mut c_void,
) -> c_int {
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let state = unsafe { &mut *(user_data as *mut State) };
        if unsafe { (state.dbus.dbus_message_get_type)(msg) } != DBUS_MESSAGE_TYPE_SIGNAL {
            return;
        }
        let iface = unsafe { ptr_to_cstr((state.dbus.dbus_message_get_interface)(msg)) };
        let member = unsafe { ptr_to_cstr((state.dbus.dbus_message_get_member)(msg)) };

        if member == Some(c"NameOwnerChanged") {
            let args = unsafe { msg::read_leading_strings(&state.dbus, msg, 3) };
            // args = [name, old_owner, new_owner]; a non-empty new owner means
            // the watcher just came up.
            if args.len() == 3 && args[0] == "org.kde.StatusNotifierWatcher" && !args[2].is_empty()
            {
                unsafe { state.register_with_watcher() };
            }
            return;
        }

        if iface != Some(NOTIFICATIONS_IFACE) {
            return;
        }
        if member == Some(c"ActionInvoked") {
            // (id: u32, action_key: s) — we set action_key to the ActionId.
            let (id, key) = unsafe {
                let mut it = msg::iter_init(&state.dbus, msg);
                let id = msg::read_u32(&state.dbus, &mut it);
                let key = if id.is_some() && msg::advance(&state.dbus, &mut it) {
                    msg::read_string(&state.dbus, &mut it)
                } else {
                    None
                };
                (id, key)
            };
            // Only react to our own notifications' actions. The id is pruned on
            // NotificationClosed (which the daemon emits after an action), so a
            // stray non-numeric key does not drop tracking prematurely.
            if let (Some(id), Some(key)) = (id, key) {
                if state.notify_ids.contains(&id) {
                    if let Ok(action) = key.parse::<u32>() {
                        state
                            .pending
                            .push(Event::NotificationAction(ActionId(action)));
                    }
                }
            }
        } else if member == Some(c"NotificationClosed") {
            // (id: u32, reason: u32) — stop tracking a dismissed notification.
            let id = unsafe {
                let mut it = msg::iter_init(&state.dbus, msg);
                msg::read_u32(&state.dbus, &mut it)
            };
            if let Some(id) = id {
                state.notify_ids.remove(&id);
            }
        }
    }));
    DBUS_HANDLER_RESULT_NOT_YET_HANDLED
}

/// Wraps a possibly-null C string pointer as an optional [`CStr`].
unsafe fn ptr_to_cstr<'a>(ptr: *const std::os::raw::c_char) -> Option<&'a CStr> {
    if ptr.is_null() {
        None
    } else {
        Some(unsafe { CStr::from_ptr(ptr) })
    }
}

/// Converts our RGBA8 icon into the ARGB32 network-byte-order layout the
/// StatusNotifierItem pixmap format expects (`[A, R, G, B]` per pixel).
fn rgba_to_argb(icon: &Icon) -> (i32, i32, Vec<u8>) {
    let mut argb = Vec::with_capacity(icon.rgba.len());
    for px in icon.rgba.chunks_exact(4) {
        argb.push(px[3]); // A
        argb.push(px[0]); // R
        argb.push(px[1]); // G
        argb.push(px[2]); // B
    }
    (icon.width as i32, icon.height as i32, argb)
}

/// Builds a [`CString`], neutralising any interior NUL bytes so it never panics.
fn cstring(s: &str) -> CString {
    match CString::new(s) {
        Ok(c) => c,
        Err(_) => CString::new(s.replace('\0', " ")).unwrap_or_default(),
    }
}
