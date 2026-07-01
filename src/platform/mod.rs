//! Platform backend selection.
//!
//! The public API talks to a single [`Backend`] trait object. Which concrete
//! backend is built is decided at runtime by [`new_backend`], based on the
//! target OS. Unknown targets — and any platform whose libraries fail to load —
//! resolve to a graceful [`crate::Error`], never a link-time dependency.

use std::time::Duration;

use crate::error::Result;
use crate::event::Event;
use crate::icon::Icon;
use crate::menu::Menu;
use crate::notification::Notification;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

/// Everything needed to bring a tray up, moved into the backend at creation.
///
/// Fields are consumed by the concrete platform backends (M3+). On the current
/// stub targets they are not yet read, hence the allow.
#[allow(dead_code)]
pub(crate) struct Init {
    pub icon: Icon,
    pub tooltip: String,
    pub menu: Option<Menu>,
}

/// The platform-agnostic contract every backend implements.
///
/// A backend is created on one thread and then owned by exactly one event loop;
/// it is only ever touched from that loop, so it need not be `Sync`, but it must
/// be `Send` so [`crate::Tray::spawn`] can move it to a worker thread.
pub(crate) trait Backend: Send {
    /// Replace the tray icon.
    fn set_icon(&mut self, icon: &Icon) -> Result<()>;
    /// Replace the hover tooltip.
    fn set_tooltip(&mut self, text: &str) -> Result<()>;
    /// Replace (or clear, with `None`) the context menu.
    fn set_menu(&mut self, menu: Option<&Menu>) -> Result<()>;
    /// Show a desktop notification.
    fn notify(&mut self, notification: &Notification) -> Result<()>;
    /// Service the platform event source for up to `timeout`, forwarding any
    /// user interactions to `sink`.
    fn pump(&mut self, timeout: Duration, sink: &mut dyn FnMut(Event)) -> Result<()>;
    /// Whether this backend may run its loop on a background thread.
    /// macOS overrides this to `false`.
    fn can_spawn(&self) -> bool {
        true
    }
}

/// Builds the backend appropriate for the current platform, or returns a
/// graceful error if none is available.
pub(crate) fn new_backend(init: Init) -> Result<Box<dyn Backend>> {
    #[cfg(target_os = "linux")]
    let backend = linux::new(init);
    #[cfg(target_os = "windows")]
    let backend = windows::new(init);
    #[cfg(target_os = "macos")]
    let backend = macos::new(init);
    #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
    let backend = {
        let _ = init;
        Err(crate::error::Error::Unsupported)
    };
    backend
}
