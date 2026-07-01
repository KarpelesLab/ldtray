use crate::icon::Icon;

/// Identifier for a notification action button. You choose the number; it is
/// echoed back in [`crate::Event::NotificationAction`] when the user clicks the
/// action.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ActionId(pub u32);

/// A desktop notification ("toast"/"balloon") to be shown via
/// [`crate::TrayHandle::notify`].
#[derive(Clone, Debug)]
pub struct Notification {
    pub(crate) title: String,
    pub(crate) body: String,
    pub(crate) icon: Option<Icon>,
    pub(crate) actions: Vec<(ActionId, String)>,
}

impl Notification {
    /// A notification with a title (summary) and body text.
    pub fn new(title: impl Into<String>, body: impl Into<String>) -> Notification {
        Notification {
            title: title.into(),
            body: body.into(),
            icon: None,
            actions: Vec::new(),
        }
    }

    /// Builder: attach an icon shown alongside the message. Backends that cannot
    /// display a custom notification icon ignore it.
    pub fn with_icon(mut self, icon: Icon) -> Notification {
        self.icon = Some(icon);
        self
    }

    /// Builder: add a clickable action button. When the user activates it,
    /// [`crate::Event::NotificationAction`] is delivered with `id`.
    ///
    /// Platform behavior:
    /// - **Linux**: full support — every action is a button (freedesktop spec).
    /// - **macOS**: the first action is the notification's action button; any
    ///   further actions become "additional actions" (shown on alert-style
    ///   notifications). Requires the deprecated `NSUserNotification` path.
    /// - **Windows**: the classic `Shell_NotifyIcon` balloon has no buttons, so
    ///   clicking the balloon delivers the **first** action.
    pub fn action(mut self, id: u32, label: impl Into<String>) -> Notification {
        self.actions.push((ActionId(id), label.into()));
        self
    }

    /// The configured action buttons.
    pub fn actions(&self) -> &[(ActionId, String)] {
        &self.actions
    }

    /// The notification title.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// The notification body.
    pub fn body(&self) -> &str {
        &self.body
    }

    /// The attached icon, if any.
    pub fn icon(&self) -> Option<&Icon> {
        self.icon.as_ref()
    }
}
