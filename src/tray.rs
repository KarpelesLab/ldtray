use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::time::Duration;

use crate::error::{Error, Result};
use crate::event::Event;
use crate::icon::Icon;
use crate::menu::Menu;
use crate::notification::Notification;
use crate::platform::{self, Backend, Init};

/// How long the event loop blocks in the platform pump between checks of the
/// command channel. Bounds the latency of [`TrayHandle`] updates.
const PUMP_INTERVAL: Duration = Duration::from_millis(100);

/// Initial configuration for a [`Tray`].
#[derive(Clone, Debug)]
pub struct TrayConfig {
    icon: Icon,
    tooltip: String,
    menu: Option<Menu>,
}

impl TrayConfig {
    /// Start a configuration with the required tray icon.
    pub fn new(icon: Icon) -> TrayConfig {
        TrayConfig {
            icon,
            tooltip: String::new(),
            menu: None,
        }
    }

    /// Builder: set the hover tooltip text.
    pub fn tooltip(mut self, tooltip: impl Into<String>) -> TrayConfig {
        self.tooltip = tooltip.into();
        self
    }

    /// Builder: set the context menu.
    pub fn menu(mut self, menu: Menu) -> TrayConfig {
        self.menu = Some(menu);
        self
    }
}

/// Commands sent from a [`TrayHandle`] to the running event loop.
enum Command {
    SetIcon(Icon),
    SetTooltip(String),
    SetMenu(Option<Menu>),
    Notify(Notification),
    Quit,
}

/// A live tray icon.
///
/// Construct it with [`Tray::new`] (which loads the platform libraries and may
/// fail gracefully), then hand control to the event loop with either
/// [`Tray::run`] (blocking, current thread) or [`Tray::spawn`] (background
/// thread). Obtain [`TrayHandle`]s with [`Tray::handle`] to mutate the tray from
/// any thread.
pub struct Tray {
    backend: Box<dyn Backend>,
    tx: Sender<Command>,
    rx: Receiver<Command>,
}

impl Tray {
    /// Creates the tray, loading the platform GUI libraries at runtime.
    ///
    /// Returns [`Error::Unsupported`] or [`Error::LibraryLoad`] when no tray is
    /// available (headless server, missing libraries, unknown OS). Daemons
    /// should treat these as non-fatal and continue without a tray.
    pub fn new(config: TrayConfig) -> Result<Tray> {
        let backend = platform::new_backend(Init {
            icon: config.icon,
            tooltip: config.tooltip,
            menu: config.menu,
        })?;
        let (tx, rx) = mpsc::channel();
        Ok(Tray { backend, tx, rx })
    }

    /// Returns a cloneable, `Send + Sync` handle for updating the tray from any
    /// thread while the event loop runs.
    pub fn handle(&self) -> TrayHandle {
        TrayHandle {
            tx: self.tx.clone(),
        }
    }

    /// Runs the event loop on the calling thread, blocking until
    /// [`TrayHandle::quit`] is called (or the backend closes).
    ///
    /// This is the portable path: it is correct on every platform, including
    /// macOS, where tray work must happen on the main thread.
    pub fn run(self, mut callback: impl FnMut(Event)) -> Result<()> {
        let Tray {
            mut backend,
            tx: _keep_alive,
            rx,
        } = self;
        event_loop(backend.as_mut(), &rx, &mut callback)
    }

    /// Runs the event loop on a newly spawned background thread and returns a
    /// handle to control it.
    ///
    /// Not available on macOS (tray work must run on the main thread there); on
    /// that platform this returns [`Error::Unsupported`] — use [`Tray::run`]
    /// from the main thread instead.
    pub fn spawn(self, mut callback: impl FnMut(Event) + Send + 'static) -> Result<TrayHandle> {
        if !self.backend.can_spawn() {
            return Err(Error::Unsupported);
        }
        let handle = self.handle();
        std::thread::Builder::new()
            .name("ldtray".into())
            .spawn(move || {
                let Tray {
                    mut backend,
                    tx: _keep_alive,
                    rx,
                } = self;
                let _ = event_loop(backend.as_mut(), &rx, &mut callback);
            })
            .map_err(|e| Error::Backend(format!("failed to spawn tray thread: {e}")))?;
        Ok(handle)
    }
}

/// The core loop: drain any queued commands, then pump platform events for a
/// bounded interval, forwarding them to `callback`. Returns when a
/// [`Command::Quit`] is received.
fn event_loop(
    backend: &mut dyn Backend,
    rx: &Receiver<Command>,
    callback: &mut dyn FnMut(Event),
) -> Result<()> {
    loop {
        loop {
            match rx.try_recv() {
                Ok(Command::SetIcon(icon)) => backend.set_icon(&icon)?,
                Ok(Command::SetTooltip(text)) => backend.set_tooltip(&text)?,
                Ok(Command::SetMenu(menu)) => backend.set_menu(menu.as_ref())?,
                Ok(Command::Notify(notification)) => backend.notify(&notification)?,
                Ok(Command::Quit) => return Ok(()),
                // The event loop keeps its own sender alive, so `Disconnected`
                // only happens if every sender is gone — nothing left to do.
                Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => break,
            }
        }
        backend.pump(PUMP_INTERVAL, callback)?;
    }
}

/// A cloneable, thread-safe handle used to update a running tray.
///
/// Every method sends a message to the event loop; if the loop has stopped they
/// return [`Error::Disconnected`].
#[derive(Clone)]
pub struct TrayHandle {
    tx: Sender<Command>,
}

impl TrayHandle {
    fn send(&self, command: Command) -> Result<()> {
        self.tx.send(command).map_err(|_| Error::Disconnected)
    }

    /// Replaces the tray icon.
    pub fn set_icon(&self, icon: Icon) -> Result<()> {
        self.send(Command::SetIcon(icon))
    }

    /// Replaces the hover tooltip text.
    pub fn set_tooltip(&self, text: impl Into<String>) -> Result<()> {
        self.send(Command::SetTooltip(text.into()))
    }

    /// Replaces the context menu.
    pub fn set_menu(&self, menu: Menu) -> Result<()> {
        self.send(Command::SetMenu(Some(menu)))
    }

    /// Removes the context menu.
    pub fn clear_menu(&self) -> Result<()> {
        self.send(Command::SetMenu(None))
    }

    /// Shows a desktop notification.
    pub fn notify(&self, notification: Notification) -> Result<()> {
        self.send(Command::Notify(notification))
    }

    /// Asks the event loop to stop. After this, [`Tray::run`] returns and the
    /// tray icon is removed.
    pub fn quit(&self) -> Result<()> {
        self.send(Command::Quit)
    }
}
