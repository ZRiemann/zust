//! ZetaX logging, independently usable or re-exported as `z::log`.
//!
//! Keep the returned [`LogGuard`] alive until producers have stopped. Explicit
//! [`LogGuard::shutdown`] drains accepted records and reports output failures.
//! `flush` only submits a request in asynchronous mode. Forced process exit and
//! abort do not run Rust destructors and cannot guarantee draining.
//!
//! ```
//! let guard = zust_log::init(zust_log::Config::default())?;
//! zust_log::important!("[example:ready] ready");
//! guard.shutdown()?;
//! # Ok::<(), zust_log::Error>(())
//! ```
mod config;
mod format;
mod runtime;
#[cfg(feature = "tracing")]
mod tracing_adapter;
#[cfg(feature = "tracing")]
pub use tracing_adapter::init_tracing;

pub use config::{Config, ImportantConfig, SinkFormat};
pub use runtime::{LogGuard, init, set_level};
use std::fmt;

/// Result returned by configuration and lifecycle operations.
pub type Result<T> = std::result::Result<T, Error>;

/// A configuration, initialization or output failure without backend types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// Invalid external configuration or an unavailable file/path.
    Config { field: String, message: String },
    /// This process has already successfully initialized logging.
    AlreadyInitialized,
    /// A backend initialization or output operation failed.
    Output(String),
}
impl Error {
    pub(crate) fn config(field: impl Into<String>, message: impl ToString) -> Self {
        Self::Config {
            field: field.into(),
            message: message.to_string(),
        }
    }
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config { field, message } => write!(f, "{field}: {message}"),
            Self::AlreadyInitialized => {
                f.write_str("logging has already been initialized in this process")
            }
            Self::Output(message) => write!(f, "logging output: {message}"),
        }
    }
}
impl std::error::Error for Error {}

/// Ordered severity, including the independent `critical` level used by zpp.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum Level {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
    Critical,
    Off,
}
impl Level {
    pub(crate) const fn backend(self) -> spdlog::Level {
        match self {
            Self::Trace => spdlog::Level::Trace,
            Self::Debug => spdlog::Level::Debug,
            Self::Info => spdlog::Level::Info,
            Self::Warn => spdlog::Level::Warn,
            Self::Error => spdlog::Level::Error,
            Self::Critical => spdlog::Level::Critical,
            Self::Off => spdlog::Level::Critical,
        }
    }
    pub(crate) fn filter(self) -> spdlog::LevelFilter {
        if self == Self::Off {
            spdlog::LevelFilter::Off
        } else {
            spdlog::LevelFilter::MoreSevereEqual(self.backend())
        }
    }
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Trace => "trace",
            Self::Debug => "debug",
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
            Self::Critical => "critical",
            Self::Off => "off",
        }
    }
    pub(crate) fn from_backend(level: spdlog::Level) -> Self {
        match level {
            spdlog::Level::Trace => Self::Trace,
            spdlog::Level::Debug => Self::Debug,
            spdlog::Level::Info => Self::Info,
            spdlog::Level::Warn => Self::Warn,
            spdlog::Level::Error => Self::Error,
            spdlog::Level::Critical => Self::Critical,
        }
    }
}
impl std::str::FromStr for Level {
    type Err = Error;
    fn from_str(value: &str) -> Result<Self> {
        match value {
            "trace" => Ok(Self::Trace),
            "debug" => Ok(Self::Debug),
            "info" => Ok(Self::Info),
            "warn" | "warning" => Ok(Self::Warn),
            "err" | "error" => Ok(Self::Error),
            "critical" => Ok(Self::Critical),
            "off" => Ok(Self::Off),
            _ => Err(Error::config("level", format!("unknown level {value:?}"))),
        }
    }
}

/// Implementation support for exported macros; not an application API.
#[doc(hidden)]
pub mod __private {
    pub use crate::runtime::{Ticket, acquire};
    /// Evaluate in the caller's const block so disabled invocations are pruned.
    pub const fn compiled_in(level: crate::Level) -> bool {
        !matches!(level, crate::Level::Off)
            && spdlog::STATIC_LEVEL_FILTER.__test_const(level.backend())
    }
    /// Source information captured at the macro call site.
    pub struct Source {
        pub file: &'static str,
        pub line: u32,
        pub function: &'static str,
    }
}

/// Shared implementation; acquire a submission ticket *before* evaluating args.
#[doc(hidden)]
#[macro_export]
macro_rules! __emit {
    ($level:expr, $important:expr, $($arg:tt)+) => {{
        if const { $crate::__private::compiled_in($level) } {
            if let Some(ticket) = $crate::__private::acquire($level, $important) {
                ticket.emit(format_args!($($arg)+), $crate::__private::Source {
                    file: file!(), line: line!(), function: module_path!(),
                });
            }
        }
    }};
}

/// Emit a trace record; disabled invocations do not evaluate arguments.
#[macro_export]
macro_rules! trace {
    ($($arg:tt)+) => { $crate::__emit!($crate::Level::Trace, false, $($arg)+) };
}

/// Emit a debug record; disabled invocations do not evaluate arguments.
#[macro_export]
macro_rules! debug {
    ($($arg:tt)+) => { $crate::__emit!($crate::Level::Debug, false, $($arg)+) };
}

/// Emit a info record; disabled invocations do not evaluate arguments.
#[macro_export]
macro_rules! info {
    ($($arg:tt)+) => { $crate::__emit!($crate::Level::Info, false, $($arg)+) };
}

/// Emit a warn record; disabled invocations do not evaluate arguments.
#[macro_export]
macro_rules! warn {
    ($($arg:tt)+) => { $crate::__emit!($crate::Level::Warn, false, $($arg)+) };
}

/// Emit a error record; disabled invocations do not evaluate arguments.
#[macro_export]
macro_rules! error {
    ($($arg:tt)+) => { $crate::__emit!($crate::Level::Error, false, $($arg)+) };
}

/// Emit a critical record; disabled invocations do not evaluate arguments.
#[macro_export]
macro_rules! critical {
    ($($arg:tt)+) => { $crate::__emit!($crate::Level::Critical, false, $($arg)+) };
}

/// Emit a important record; disabled invocations do not evaluate arguments.
#[macro_export]
macro_rules! important {
    ($($arg:tt)+) => { $crate::__emit!($crate::Level::Info, true, $($arg)+) };
}
