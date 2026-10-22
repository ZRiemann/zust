use crate::{Error, Level, Result};
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};
use std::{env, fs, path::Path};

pub(crate) const DEFAULT_PATTERN: &str = "%d %H:%M:%S.%f %t %^%L%$: %v\t%g:%#";

/// Format used by the important file sink.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SinkFormat {
    /// Render records using the configured text pattern.
    Pattern,
    /// Serialize one zpp-compatible JSON object per line.
    Jsonl,
    /// Versioned structured diagnostics with UTC timestamps and tracing context.
    JsonlV2,
}

/// Independent important file configuration; sizes are MiB and backup counts.
#[derive(Debug, Clone)]
pub struct ImportantConfig {
    /// Enable the independent important file (default false).
    pub enabled: bool,
    /// File path; empty selects the loader-specific default described in Config.
    pub name: String,
    /// Maximum file size in MiB (default 32); oversized records remain whole.
    pub max_size: usize,
    /// Backup file count, excluding the active file (default 3).
    pub rotats: usize,
    /// Rotate a nonempty existing file when opening (default true).
    pub rotate_on_open: bool,
    /// Ordinary records must meet this threshold (default Warn); important! bypasses it.
    pub level: Level,
    /// Structured JSONL by default, or the text pattern.
    pub format: SinkFormat,
    /// Empty inherits the runtime pattern.
    pub pattern: String,
}
impl Default for ImportantConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            name: String::new(),
            max_size: 32,
            rotats: 3,
            rotate_on_open: true,
            level: Level::Warn,
            format: SinkFormat::Jsonl,
            pattern: String::new(),
        }
    }
}

/// zpp-compatible logging settings. Programmatic defaults enable only stdout.
/// Both JSON loaders enable the runtime rotating file by default, as zpp's
/// server configuration loader does. `flush_interval_sec = 0` disables the
/// periodic flusher programmatically; JSON requires a positive interval.
#[derive(Debug, Clone)]
pub struct Config {
    /// Use one bounded backend queue/worker when true (default false).
    pub r#async: bool,
    /// Write to stdout with terminal-aware colors (default true).
    pub console: bool,
    /// Programmatic switch; not a zpp JSON field.
    pub rotating_file: bool,
    /// Rotate a nonempty runtime file on initialization (default true).
    pub rotate_on_open: bool,
    /// Runtime path, default server.log; supports ~, $VAR and ${VAR} expansion.
    pub name: String,
    /// Runtime file size threshold in MiB (default 32).
    pub max_size: usize,
    /// Runtime backup count, excluding the active file (default 3).
    pub rotats: usize,
    /// Shared asynchronous operation queue capacity (default 8192).
    pub queue_size: usize,
    /// Periodic flush interval (default 2); programmatic zero disables it.
    pub flush_interval_sec: u64,
    /// Ordinary record minimum severity (default Trace).
    pub level: Level,
    /// Severity that requests a flush (default Warn); Off disables level flushing.
    pub flush_level: Level,
    /// Base logger name for all records (default zpp for compatibility).
    pub logger_name: String,
    /// Compiled zpp text pattern; an empty string selects the documented default.
    pub pattern: String,
    /// Logical service identity written to JSONL (default empty).
    pub service: String,
    /// Logical node identity written to JSONL (default empty).
    pub node: String,
    /// Caller-supplied run identity written to JSONL (default empty).
    pub run_id: String,
    /// Additional file target for important events and qualifying ordinary records.
    pub important: ImportantConfig,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            r#async: false,
            console: true,
            rotating_file: false,
            rotate_on_open: true,
            name: "server.log".into(),
            max_size: 32,
            rotats: 3,
            queue_size: 8192,
            flush_interval_sec: 2,
            level: Level::Trace,
            flush_level: Level::Warn,
            logger_name: "zpp".into(),
            pattern: DEFAULT_PATTERN.into(),
            service: String::new(),
            node: String::new(),
            run_id: String::new(),
            important: ImportantConfig::default(),
        }
    }
}
impl Config {
    /// Parse a standalone zpp `spd` JSON object. Unknown fields are ignored.
    pub fn from_spd_json(json: &str) -> Result<Self> {
        let value: Value = serde_json::from_str(json).map_err(|e| Error::config("spd", e))?;
        Self::parse(&value, "spd", String::new())
    }

    /// Read `<section>.spd` from a unified JSON configuration file.
    pub fn from_zpp_file(path: impl AsRef<Path>, section: &str) -> Result<Self> {
        let path = path.as_ref();
        let text =
            fs::read_to_string(path).map_err(|e| Error::config(path.display().to_string(), e))?;
        let root: Value = serde_json::from_str(&text)
            .map_err(|e| Error::config(path.display().to_string(), e))?;
        let field = format!("{section}.spd");
        let section_value = root
            .as_object()
            .and_then(|o| o.get(section))
            .and_then(Value::as_object)
            .ok_or_else(|| Error::config(section, "expected service object"))?;
        let value = section_value
            .get("spd")
            .ok_or_else(|| Error::config(&field, "missing logging object"))?;
        Self::parse(value, &field, section.to_owned())
    }

    fn parse(value: &Value, field: &str, service: String) -> Result<Self> {
        let obj = object(value, field)?;
        let mut cfg = Self {
            rotating_file: true,
            service,
            ..Self::default()
        };
        read(obj, "async", &mut cfg.r#async, field)?;
        read(obj, "console", &mut cfg.console, field)?;
        read(obj, "rotate_on_open", &mut cfg.rotate_on_open, field)?;
        read(obj, "name", &mut cfg.name, field)?;
        read(obj, "max_size", &mut cfg.max_size, field)?;
        read(obj, "rotats", &mut cfg.rotats, field)?;
        read(obj, "queue_size", &mut cfg.queue_size, field)?;
        read(
            obj,
            "flush_interval_sec",
            &mut cfg.flush_interval_sec,
            field,
        )?;
        if cfg.flush_interval_sec == 0 {
            return Err(Error::config(
                format!("{field}.flush_interval_sec"),
                "must be positive",
            ));
        }
        read(obj, "logger_name", &mut cfg.logger_name, field)?;
        read(obj, "pattern", &mut cfg.pattern, field)?;
        read(obj, "service", &mut cfg.service, field)?;
        read(obj, "node", &mut cfg.node, field)?;
        read(obj, "run_id", &mut cfg.run_id, field)?;
        read_level(obj, "level", &mut cfg.level, field)?;
        read_level(obj, "flush_level", &mut cfg.flush_level, field)?;
        if let Some(value) = obj.get("important") {
            let field = format!("{field}.important");
            let obj = object(value, &field)?;
            let imp = &mut cfg.important;
            read(obj, "enabled", &mut imp.enabled, &field)?;
            read(obj, "name", &mut imp.name, &field)?;
            read(obj, "max_size", &mut imp.max_size, &field)?;
            read(obj, "rotats", &mut imp.rotats, &field)?;
            read(obj, "rotate_on_open", &mut imp.rotate_on_open, &field)?;
            read(obj, "pattern", &mut imp.pattern, &field)?;
            read_level(obj, "level", &mut imp.level, &field)?;
            let mut format = "jsonl".to_owned();
            read(obj, "format", &mut format, &field)?;
            imp.format = match format.as_str() {
                "pattern" => SinkFormat::Pattern,
                "jsonl" => SinkFormat::Jsonl,
                "jsonl-v2" => SinkFormat::JsonlV2,
                _ => {
                    return Err(Error::config(
                        format!("{field}.format"),
                        "expected pattern, jsonl or jsonl-v2",
                    ));
                }
            };
            if imp.enabled && imp.name.is_empty() {
                imp.name = format!(
                    "{}.important.jsonl",
                    if cfg.service.is_empty() {
                        "zpp"
                    } else {
                        &cfg.service
                    }
                );
            }
        }
        cfg.validate(field)?;
        Ok(cfg)
    }

    pub(crate) fn validate(&self, field: &str) -> Result<()> {
        file_limits(self.max_size, self.rotats, field)?;
        file_limits(
            self.important.max_size,
            self.important.rotats,
            &format!("{field}.important"),
        )?;
        if self.queue_size == 0 {
            return Err(Error::config(
                format!("{field}.queue_size"),
                "must be positive",
            ));
        }
        if self.rotating_file && self.name.is_empty() {
            return Err(Error::config(
                format!("{field}.name"),
                "must not be empty for an enabled sink",
            ));
        }
        Ok(())
    }
}
fn object<'a>(value: &'a Value, field: &str) -> Result<&'a Map<String, Value>> {
    value
        .as_object()
        .ok_or_else(|| Error::config(field, "expected object"))
}
fn read<T: DeserializeOwned>(
    obj: &Map<String, Value>,
    key: &str,
    out: &mut T,
    field: &str,
) -> Result<()> {
    if let Some(value) = obj.get(key) {
        *out = serde_json::from_value(value.clone())
            .map_err(|e| Error::config(format!("{field}.{key}"), e))?;
    }
    Ok(())
}
fn read_level(obj: &Map<String, Value>, key: &str, out: &mut Level, field: &str) -> Result<()> {
    if obj.contains_key(key) {
        let mut name = String::new();
        read(obj, key, &mut name, field)?;
        *out = name
            .parse()
            .map_err(|e| Error::config(format!("{field}.{key}"), e))?;
    }
    Ok(())
}
pub(crate) fn file_limits(size: usize, backups: usize, field: &str) -> Result<(u64, usize)> {
    let bytes = u64::try_from(size)
        .ok()
        .and_then(|s| s.checked_mul(1024 * 1024))
        .filter(|s| *s > 0)
        .ok_or_else(|| {
            Error::config(
                format!("{field}.max_size"),
                "must be a positive MiB value without overflow",
            )
        })?;
    let files = backups
        .checked_add(1)
        .filter(|_| backups > 0)
        .ok_or_else(|| {
            Error::config(
                format!("{field}.rotats"),
                "must be positive and leave room for the active file",
            )
        })?;
    Ok((bytes, files))
}

pub(crate) fn expand_path(path: &str, field: &str) -> Result<String> {
    expand_with(path, field, |name| env::var(name).ok())
}
fn expand_with(path: &str, field: &str, lookup: impl Fn(&str) -> Option<String>) -> Result<String> {
    let mut path = path.to_owned();
    if path == "~" || path.starts_with("~/") {
        let home = lookup("HOME")
            .filter(|s| !s.is_empty())
            .ok_or_else(|| Error::config(field, "HOME is undefined or empty"))?;
        path.replace_range(..1, &home);
    }
    let mut expanded = String::new();
    let bytes = path.as_bytes();
    let mut pos = 0;
    while pos < path.len() {
        if bytes[pos] != b'$' {
            let ch = path[pos..].chars().next().unwrap();
            expanded.push(ch);
            pos += ch.len_utf8();
            continue;
        }
        let start = pos + 1;
        let (name, next) = if bytes.get(start) == Some(&b'{') {
            if let Some(end) = path[start + 1..].find('}') {
                let end = start + 1 + end;
                (&path[start + 1..end], end + 1)
            } else {
                expanded.push('$');
                pos += 1;
                continue;
            }
        } else {
            let mut end = start;
            while bytes
                .get(end)
                .is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_')
            {
                end += 1;
            }
            if end == start {
                expanded.push('$');
                pos += 1;
                continue;
            }
            (&path[start..end], end)
        };
        let value = if name.is_empty() { None } else { lookup(name) }.ok_or_else(|| {
            Error::config(field, format!("undefined environment variable {name:?}"))
        })?;
        expanded.push_str(&value);
        pos = next;
    }
    Ok(expanded)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn expansion_is_explicit_and_non_recursive() {
        let lookup = |key: &str| match key {
            "HOME" => Some("/tmp/家".into()),
            "ROOT" => Some("$literal".into()),
            _ => None,
        };
        assert_eq!(
            expand_with("~/${ROOT}/$HOME/日志", "name", lookup).unwrap(),
            "/tmp/家/$literal//tmp/家/日志"
        );
        for path in ["${MISSING}", "$MISSING", "${}"] {
            assert!(
                expand_with(path, "name", lookup)
                    .unwrap_err()
                    .to_string()
                    .starts_with("name:")
            );
        }
        assert_eq!(
            expand_with("$/${unfinished", "name", lookup).unwrap(),
            "$/${unfinished"
        );
    }
}
