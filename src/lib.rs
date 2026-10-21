//! Shared ZetaX components, exposed through `z::xxx`.
//! Enable the `log` feature to use `log`.
//!
//! ```
//! # #[cfg(feature = "log")]
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let guard = z::log::init(z::log::Config::default())?;
//! z::log::info!("[example:startup] started");
//! guard.shutdown()?;
//! # Ok(())
//! # }
//! # #[cfg(not(feature = "log"))]
//! # fn main() {}
//! ```
#[cfg(feature = "log")]
pub use zust_log as log;
