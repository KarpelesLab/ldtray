use crate::menu::MenuId;
use crate::notification::ActionId;

/// Something the user did to the tray icon, delivered to the callback passed to
/// [`crate::Tray::run`] or [`crate::Tray::spawn`].
///
/// Not every backend can produce every variant — for example some Linux desktops
/// only report an "activate" (mapped to [`Event::LeftClick`]) and a context-menu
/// request (mapped to [`Event::RightClick`]). Match non-exhaustively.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum Event {
    /// Primary button click / "activate" (typically left mouse button).
    LeftClick,
    /// Secondary button click / context-menu request (typically right button).
    RightClick,
    /// Middle button click / "secondary activate".
    MiddleClick,
    /// Primary button double-click.
    DoubleClick,
    /// A menu entry with this id was activated.
    Menu(MenuId),
    /// A notification action button with this id was clicked (Linux only for
    /// now; see [`crate::Notification::action`]).
    NotificationAction(ActionId),
}
