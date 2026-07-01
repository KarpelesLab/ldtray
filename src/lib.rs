//! `ldtray` — cross-platform tray icons that are **never linked** against any
//! GUI or platform library at compile time.
//!
//! Every platform toolkit (libdbus on Linux, `shell32`/`user32` on Windows,
//! AppKit/`objc` on macOS) is resolved at *runtime* through
//! [`libloading`](https://docs.rs/libloading). The practical consequence is that
//! a single daemon binary runs everywhere: on a headless server the only failure
//! is a clean [`Error`] returned from [`Tray::new`] ("the tray library could not
//! be loaded"), never a link error and never a crash. Callers can simply ignore
//! the error and keep running without a tray.
//!
//! # Example
//!
//! ```no_run
//! use ldtray::{Tray, TrayConfig, Icon, Menu, MenuItem, Event, Notification};
//!
//! # fn main() -> ldtray::Result<()> {
//! let icon = Icon::from_rgba(1, 1, vec![255, 0, 0, 255])?;
//! let menu = Menu::new()
//!     .item(MenuItem::button(1, "Say hi"))
//!     .item(MenuItem::separator())
//!     .item(MenuItem::button(2, "Quit"));
//!
//! let tray = match Tray::new(TrayConfig::new(icon).tooltip("demo").menu(menu)) {
//!     Ok(tray) => tray,
//!     Err(err) => {
//!         eprintln!("no tray available ({err}); continuing headless");
//!         return Ok(());
//!     }
//! };
//!
//! let handle = tray.handle();
//! tray.run(move |event| match event {
//!     Event::Menu(id) if id.0 == 1 => {
//!         let _ = handle.notify(Notification::new("demo", "hi there"));
//!     }
//!     Event::Menu(id) if id.0 == 2 => {
//!         let _ = handle.quit();
//!     }
//!     other => println!("event: {other:?}"),
//! })
//! # }
//! ```

#![deny(unsafe_op_in_unsafe_fn)]

mod error;
mod event;
mod icon;
mod menu;
mod notification;
mod platform;
mod tray;

pub use error::{Error, Result};
pub use event::Event;
pub use icon::Icon;
pub use menu::{Menu, MenuId, MenuItem};
pub use notification::{ActionId, Notification};
pub use tray::{Tray, TrayConfig, TrayHandle};
