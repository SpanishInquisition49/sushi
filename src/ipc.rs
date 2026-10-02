//! Local stream sockets: Unix domain sockets everywhere. Windows 10+ supports AF_UNIX,
//! and `uds_windows` gives it the same API as `std::os::unix::net`.

#[cfg(unix)]
pub use std::os::unix::net::{UnixListener, UnixStream};
#[cfg(windows)]
pub use uds_windows::{UnixListener, UnixStream};
