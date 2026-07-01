/// Stable identifier for a clickable menu entry.
///
/// You choose the numbers; they are echoed back verbatim in [`crate::Event::Menu`]
/// when the user activates the corresponding item.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct MenuId(pub u32);

/// A single entry in a [`Menu`].
#[derive(Clone, Debug)]
pub enum MenuItem {
    /// A clickable entry. When activated it emits [`crate::Event::Menu`] with `id`.
    Button {
        /// Identifier reported back on click.
        id: MenuId,
        /// Text shown to the user.
        label: String,
        /// Whether the entry is selectable.
        enabled: bool,
        /// `Some(true)`/`Some(false)` renders a checkbox in that state; `None`
        /// renders a plain entry.
        checked: Option<bool>,
    },
    /// A horizontal separator line.
    Separator,
    /// A nested submenu.
    Submenu {
        /// Text shown to the user.
        label: String,
        /// Whether the submenu can be opened.
        enabled: bool,
        /// Child entries.
        items: Vec<MenuItem>,
    },
}

impl MenuItem {
    /// A plain clickable button.
    pub fn button(id: u32, label: impl Into<String>) -> MenuItem {
        MenuItem::Button {
            id: MenuId(id),
            label: label.into(),
            enabled: true,
            checked: None,
        }
    }

    /// A checkbox entry in the given state.
    pub fn checkbox(id: u32, label: impl Into<String>, checked: bool) -> MenuItem {
        MenuItem::Button {
            id: MenuId(id),
            label: label.into(),
            enabled: true,
            checked: Some(checked),
        }
    }

    /// A separator line.
    pub fn separator() -> MenuItem {
        MenuItem::Separator
    }

    /// A submenu containing `items`.
    pub fn submenu(
        label: impl Into<String>,
        items: impl IntoIterator<Item = MenuItem>,
    ) -> MenuItem {
        MenuItem::Submenu {
            label: label.into(),
            enabled: true,
            items: items.into_iter().collect(),
        }
    }

    /// Builder: set whether this entry is enabled. No-op on [`MenuItem::Separator`].
    pub fn enabled(mut self, value: bool) -> MenuItem {
        match &mut self {
            MenuItem::Button { enabled, .. } | MenuItem::Submenu { enabled, .. } => {
                *enabled = value
            }
            MenuItem::Separator => {}
        }
        self
    }
}

/// An ordered list of [`MenuItem`]s shown when the tray icon is right-clicked
/// (or, on some desktops, on any click).
///
/// A `Menu` is a cheap, immutable description: rebuild and re-apply it via
/// [`crate::TrayHandle::set_menu`] whenever it changes.
#[derive(Clone, Debug, Default)]
pub struct Menu {
    pub(crate) items: Vec<MenuItem>,
}

impl Menu {
    /// An empty menu.
    pub fn new() -> Menu {
        Menu { items: Vec::new() }
    }

    /// Builder: append an item.
    pub fn item(mut self, item: MenuItem) -> Menu {
        self.items.push(item);
        self
    }

    /// The menu's entries.
    pub fn items(&self) -> &[MenuItem] {
        &self.items
    }

    /// Whether the menu has no entries.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

impl FromIterator<MenuItem> for Menu {
    fn from_iter<I: IntoIterator<Item = MenuItem>>(iter: I) -> Menu {
        Menu {
            items: iter.into_iter().collect(),
        }
    }
}
