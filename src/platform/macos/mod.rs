//! macOS tray backend.
//!
//! Implemented via `NSStatusBar`/`NSStatusItem` driven through the Objective-C
//! runtime (`libobjc` + AppKit), all loaded at runtime. Filled in at milestone
//! M7; until then this returns [`crate::Error::Unsupported`].

use super::{Backend, Init};
use crate::error::Result;

pub(crate) fn new(_init: Init) -> Result<Box<dyn Backend>> {
    Err(crate::error::Error::Unsupported)
}
