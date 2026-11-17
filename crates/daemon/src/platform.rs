#[cfg(unix)]
pub use crate::linux::{controller, desktop, games, hotkeys, notify, tray};

#[cfg(windows)]
pub use crate::windows::{controller, desktop, games, hotkeys, notify, tray};
