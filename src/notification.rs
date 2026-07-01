use crate::icon::Icon;

/// A desktop notification ("toast"/"balloon") to be shown via
/// [`crate::TrayHandle::notify`].
#[derive(Clone, Debug)]
pub struct Notification {
    pub(crate) title: String,
    pub(crate) body: String,
    pub(crate) icon: Option<Icon>,
}

impl Notification {
    /// A notification with a title (summary) and body text.
    pub fn new(title: impl Into<String>, body: impl Into<String>) -> Notification {
        Notification {
            title: title.into(),
            body: body.into(),
            icon: None,
        }
    }

    /// Builder: attach an icon shown alongside the message. Backends that cannot
    /// display a custom notification icon ignore it.
    pub fn with_icon(mut self, icon: Icon) -> Notification {
        self.icon = Some(icon);
        self
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
