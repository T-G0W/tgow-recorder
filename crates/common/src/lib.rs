pub mod config;
pub mod i18n;
pub mod ipc;
pub mod paths;
pub mod platform;
pub mod system;

pub use config::Config;
pub use i18n::{tr, trf};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
