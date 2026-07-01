//! Linux tray backend.
//!
//! Implemented via the freedesktop/KDE StatusNotifierItem (SNI) specification
//! over D-Bus, with `libdbus-1.so.3` loaded at runtime. Filled in over
//! milestones M2–M5; until then this returns [`crate::Error::Unsupported`].

use super::{Backend, Init};
use crate::error::Result;

pub(crate) fn new(_init: Init) -> Result<Box<dyn Backend>> {
    Err(crate::error::Error::Unsupported)
}
