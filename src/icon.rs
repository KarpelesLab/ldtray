use std::fmt;

use crate::error::{Error, Result};

/// A tray icon stored as raw, non-premultiplied RGBA8 pixels in row-major order
/// (`[r, g, b, a, r, g, b, a, ...]`, top row first).
///
/// RGBA is the universal representation; each backend converts it to whatever
/// the platform wants (ARGB32 pixmaps on Linux, an `HICON` on Windows, an
/// `NSImage` on macOS). Keeping the source format toolkit-neutral is what lets
/// the whole crate avoid linking against any image library.
#[derive(Clone)]
pub struct Icon {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) rgba: Vec<u8>,
}

impl Icon {
    /// Builds an icon from raw RGBA8 pixels.
    ///
    /// `rgba` must be exactly `width * height * 4` bytes and non-empty, otherwise
    /// [`Error::Backend`] is returned.
    pub fn from_rgba(width: u32, height: u32, rgba: Vec<u8>) -> Result<Icon> {
        let expected = (width as usize)
            .checked_mul(height as usize)
            .and_then(|pixels| pixels.checked_mul(4));
        match expected {
            Some(n) if n != 0 && n == rgba.len() => Ok(Icon {
                width,
                height,
                rgba,
            }),
            _ => Err(Error::Backend(format!(
                "icon data is {} bytes but {}x{} RGBA needs {}",
                rgba.len(),
                width,
                height,
                (width as u64) * (height as u64) * 4,
            ))),
        }
    }

    /// Icon width in pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Icon height in pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Raw RGBA8 pixels, `width * height * 4` bytes, row-major.
    pub fn rgba(&self) -> &[u8] {
        &self.rgba
    }
}

impl fmt::Debug for Icon {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Icon")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("bytes", &self.rgba.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_matching_length() {
        let icon = Icon::from_rgba(2, 1, vec![0; 8]).unwrap();
        assert_eq!(icon.width(), 2);
        assert_eq!(icon.height(), 1);
        assert_eq!(icon.rgba().len(), 8);
    }

    #[test]
    fn rejects_wrong_length() {
        assert!(Icon::from_rgba(2, 2, vec![0; 8]).is_err());
    }

    #[test]
    fn rejects_empty() {
        assert!(Icon::from_rgba(0, 0, vec![]).is_err());
    }
}
