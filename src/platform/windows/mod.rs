//! Windows tray backend.
//!
//! Implemented via `Shell_NotifyIconW` (shell32) plus a hidden message window
//! (user32), all loaded at runtime. Filled in at milestone M6; until then this
//! returns [`crate::Error::Unsupported`].

use super::{Backend, Init};
use crate::error::Result;

pub(crate) fn new(_init: Init) -> Result<Box<dyn Backend>> {
    Err(crate::error::Error::Unsupported)
}
