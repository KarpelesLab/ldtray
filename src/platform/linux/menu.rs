//! `com.canonical.dbusmenu` model and layout rendering.
//!
//! The StatusNotifierItem advertises a menu object path (`/MenuBar`); the tray
//! host talks the dbusmenu protocol to that object to draw and operate the
//! context menu. We flatten the public [`Menu`] into an integer-id'd node tree
//! (dbusmenu identifies every entry — buttons, separators, submenus — by a plain
//! `i32`, with id `0` reserved for the invisible root) and render `GetLayout` /
//! `GetGroupProperties` from it. Clicks arrive as `Event(id, "clicked", ...)`
//! and map back to the caller's [`MenuId`](crate::MenuId).

use std::ffi::CString;

use super::dbus::*;
use super::msg;
use crate::menu::{Menu, MenuItem};

/// One dbusmenu entry. Index into [`DbusMenu::nodes`] is the dbusmenu id.
pub(super) struct Node {
    pub label: Option<CString>,
    pub separator: bool,
    pub enabled: bool,
    /// `Some(checked)` renders a checkmark toggle in that state.
    pub toggle: Option<bool>,
    /// dbusmenu ids of child entries (submenu contents).
    pub children: Vec<i32>,
    /// The caller's menu id, for buttons; used to map clicks back.
    pub menu_id: Option<u32>,
}

impl Node {
    fn root() -> Node {
        Node {
            label: None,
            separator: false,
            enabled: true,
            toggle: None,
            children: Vec::new(),
            menu_id: None,
        }
    }
}

/// The rendered menu: a node tree plus a monotonically increasing revision that
/// the host uses to know when to re-fetch the layout.
pub(super) struct DbusMenu {
    pub revision: u32,
    nodes: Vec<Node>,
}

impl DbusMenu {
    /// Builds a menu model. `revision` should be strictly greater than the
    /// previous model's so hosts notice the change after a `LayoutUpdated`.
    pub fn build(menu: Option<&Menu>, revision: u32) -> DbusMenu {
        let mut nodes = vec![Node::root()];
        if let Some(menu) = menu {
            let children = add_items(&mut nodes, menu.items());
            nodes[0].children = children;
        }
        DbusMenu { revision, nodes }
    }

    fn node(&self, id: i32) -> Option<&Node> {
        usize::try_from(id).ok().and_then(|i| self.nodes.get(i))
    }

    /// Maps a dbusmenu id back to the caller's [`MenuId`](crate::MenuId) value.
    pub fn menu_id_for(&self, id: i32) -> Option<u32> {
        self.node(id).and_then(|n| n.menu_id)
    }

    /// Appends a node's `a{sv}` property dict onto `it`. Used by both `GetLayout`
    /// (inside the recursive item struct) and `GetGroupProperties`.
    unsafe fn append_props(&self, d: &DBus, it: *mut DBusMessageIter, node: &Node) {
        let mut arr = DBusMessageIter::uninit();
        unsafe {
            (d.dbus_message_iter_open_container)(it, DBUS_TYPE_ARRAY, c"{sv}".as_ptr(), &mut arr);
            self.append_prop_entries(d, &mut arr, node);
            (d.dbus_message_iter_close_container)(it, &mut arr);
        }
    }

    unsafe fn append_prop_entries(&self, d: &DBus, arr: *mut DBusMessageIter, node: &Node) {
        use msg::Variant;
        unsafe {
            if node.separator {
                msg::append_dict_entry(d, arr, c"type", &Variant::Str(c"separator"));
                return;
            }
            if let Some(label) = &node.label {
                msg::append_dict_entry(d, arr, c"label", &Variant::Str(label));
            }
            msg::append_dict_entry(d, arr, c"enabled", &Variant::Bool(node.enabled));
            msg::append_dict_entry(d, arr, c"visible", &Variant::Bool(true));
            if let Some(checked) = node.toggle {
                msg::append_dict_entry(d, arr, c"toggle-type", &Variant::Str(c"checkmark"));
                msg::append_dict_entry(
                    d,
                    arr,
                    c"toggle-state",
                    &Variant::Int32(if checked { 1 } else { 0 }),
                );
            }
            if !node.children.is_empty() {
                msg::append_dict_entry(d, arr, c"children-display", &Variant::Str(c"submenu"));
            }
        }
    }

    /// Appends one `(ia{sv}av)` layout item for `id`, recursing into children
    /// while `depth != 0` (`depth < 0` means unlimited, matching dbusmenu's -1).
    unsafe fn append_layout(&self, d: &DBus, it: *mut DBusMessageIter, id: i32, depth: i32) {
        let Some(node) = self.node(id) else { return };
        unsafe {
            let mut item = DBusMessageIter::uninit();
            (d.dbus_message_iter_open_container)(it, DBUS_TYPE_STRUCT, std::ptr::null(), &mut item);
            msg::append_i32(d, &mut item, id);
            self.append_props(d, &mut item, node);

            let mut kids = DBusMessageIter::uninit();
            (d.dbus_message_iter_open_container)(
                &mut item,
                DBUS_TYPE_ARRAY,
                c"v".as_ptr(),
                &mut kids,
            );
            if depth != 0 {
                let next_depth = if depth < 0 { -1 } else { depth - 1 };
                for &child in &node.children {
                    let mut var = DBusMessageIter::uninit();
                    (d.dbus_message_iter_open_container)(
                        &mut kids,
                        DBUS_TYPE_VARIANT,
                        c"(ia{sv}av)".as_ptr(),
                        &mut var,
                    );
                    self.append_layout(d, &mut var, child, next_depth);
                    (d.dbus_message_iter_close_container)(&mut kids, &mut var);
                }
            }
            (d.dbus_message_iter_close_container)(&mut item, &mut kids);
            (d.dbus_message_iter_close_container)(it, &mut item);
        }
    }

    /// Renders the `GetLayout` reply body: `u revision` then the `(ia{sv}av)`
    /// tree rooted at `parent`.
    pub unsafe fn append_get_layout(
        &self,
        d: &DBus,
        it: *mut DBusMessageIter,
        parent: i32,
        depth: i32,
    ) {
        unsafe {
            msg::append_u32(d, it, self.revision);
            self.append_layout(d, it, parent, depth);
        }
    }

    /// Renders the `GetGroupProperties` reply body: `a(ia{sv})` for `ids`.
    pub unsafe fn append_group_properties(&self, d: &DBus, it: *mut DBusMessageIter, ids: &[i32]) {
        unsafe {
            let mut arr = DBusMessageIter::uninit();
            (d.dbus_message_iter_open_container)(
                it,
                DBUS_TYPE_ARRAY,
                c"(ia{sv})".as_ptr(),
                &mut arr,
            );
            for &id in ids {
                if let Some(node) = self.node(id) {
                    let mut entry = DBusMessageIter::uninit();
                    (d.dbus_message_iter_open_container)(
                        &mut arr,
                        DBUS_TYPE_STRUCT,
                        std::ptr::null(),
                        &mut entry,
                    );
                    msg::append_i32(d, &mut entry, id);
                    self.append_props(d, &mut entry, node);
                    (d.dbus_message_iter_close_container)(&mut arr, &mut entry);
                }
            }
            (d.dbus_message_iter_close_container)(it, &mut arr);
        }
    }
}

/// Recursively appends `items` as new nodes, returning their dbusmenu ids.
fn add_items(nodes: &mut Vec<Node>, items: &[MenuItem]) -> Vec<i32> {
    let mut ids = Vec::with_capacity(items.len());
    for item in items {
        let id = nodes.len() as i32;
        match item {
            MenuItem::Separator => {
                nodes.push(Node {
                    label: None,
                    separator: true,
                    enabled: true,
                    toggle: None,
                    children: Vec::new(),
                    menu_id: None,
                });
            }
            MenuItem::Button {
                id: menu_id,
                label,
                enabled,
                checked,
            } => {
                nodes.push(Node {
                    label: Some(super::cstring(label)),
                    separator: false,
                    enabled: *enabled,
                    toggle: *checked,
                    children: Vec::new(),
                    menu_id: Some(menu_id.0),
                });
            }
            MenuItem::Submenu {
                label,
                enabled,
                items,
            } => {
                nodes.push(Node {
                    label: Some(super::cstring(label)),
                    separator: false,
                    enabled: *enabled,
                    toggle: None,
                    children: Vec::new(),
                    menu_id: None,
                });
                let children = add_items(nodes, items);
                nodes[id as usize].children = children;
            }
        }
        ids.push(id);
    }
    ids
}
