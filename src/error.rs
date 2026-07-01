use std::fmt;

/// Errors produced by `ldtray`.
///
/// The most important variants for daemons are [`Error::Unsupported`] and
/// [`Error::LibraryLoad`]: both mean "no tray here, but nothing is wrong with
/// your program". Treating them as non-fatal is the intended usage.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// The current platform has no tray backend, or no display/session is
    /// available (for example a headless server, or an unknown target OS).
    Unsupported,
    /// A required platform library could not be loaded at runtime. The string
    /// carries the underlying loader message. This is the expected error on a
    /// machine where the GUI stack simply is not installed.
    LibraryLoad(String),
    /// The backend was reachable but rejected an operation. The string carries a
    /// human-readable description.
    Backend(String),
    /// The event loop is no longer running, so a [`crate::TrayHandle`] command
    /// could not be delivered.
    Disconnected,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Unsupported => f.write_str("tray is not supported in this environment"),
            Error::LibraryLoad(msg) => write!(f, "failed to load platform library: {msg}"),
            Error::Backend(msg) => write!(f, "tray backend error: {msg}"),
            Error::Disconnected => f.write_str("tray event loop is no longer running"),
        }
    }
}

impl std::error::Error for Error {}

/// Convenience alias for results returned by this crate.
pub type Result<T> = std::result::Result<T, Error>;
