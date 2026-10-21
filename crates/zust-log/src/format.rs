use crate::{Config, Error, Level, Result, config::DEFAULT_PATTERN};
use chrono::{DateTime, Datelike, Local, Timelike};
use serde::Serialize;
use spdlog::{
    Record, StringBuf,
    formatter::{Formatter, FormatterContext},
};
use std::fmt::Write;

#[derive(Clone)]
enum Token {
    Literal(String),
    Flag(char),
}

/// Compiled zpp pattern; deliberately not the backend's different pattern DSL.
#[derive(Clone)]
pub(crate) struct TextFormatter {
    tokens: Vec<Token>,
}
impl TextFormatter {
    pub(crate) fn new(pattern: &str, field: &str) -> Result<Self> {
        let pattern = if pattern.is_empty() {
            DEFAULT_PATTERN
        } else {
            pattern
        };
        let mut tokens = Vec::new();
        let mut literal = String::new();
        let mut chars = pattern.chars();
        while let Some(ch) = chars.next() {
            if ch != '%' {
                literal.push(ch);
                continue;
            }
            let flag = chars
                .next()
                .ok_or_else(|| Error::config(field, "unfinished pattern flag"))?;
            if !"%YmdHMSeftlLnvsg#!P^$".contains(flag) {
                return Err(Error::config(
                    field,
                    format!("unsupported pattern flag %{flag}; width modifiers are not supported"),
                ));
            }
            if !literal.is_empty() {
                tokens.push(Token::Literal(std::mem::take(&mut literal)));
            }
            tokens.push(Token::Flag(flag));
        }
        if !literal.is_empty() {
            tokens.push(Token::Literal(literal));
        }
        Ok(Self { tokens })
    }
    fn render(
        &self,
        meta: &Meta<'_>,
        dest: &mut StringBuf,
        ctx: &mut FormatterContext,
    ) -> std::fmt::Result {
        let mut style_start = None;
        for token in &self.tokens {
            let Token::Flag(flag) = token else {
                if let Token::Literal(text) = token {
                    dest.write_str(text)?;
                }
                continue;
            };
            match flag {
                '%' => dest.push('%'),
                'Y' => write!(dest, "{:04}", meta.time.year())?,
                'm' => write!(dest, "{:02}", meta.time.month())?,
                'd' => write!(dest, "{:02}", meta.time.day())?,
                'H' => write!(dest, "{:02}", meta.time.hour())?,
                'M' => write!(dest, "{:02}", meta.time.minute())?,
                'S' => write!(dest, "{:02}", meta.time.second())?,
                'e' => write!(dest, "{:03}", meta.time.timestamp_subsec_millis())?,
                'f' => write!(dest, "{:06}", meta.time.timestamp_subsec_micros())?,
                't' => write!(dest, "{:02}", meta.tid)?,
                'l' => dest.write_str(if meta.level == Level::Warn {
                    "warning"
                } else {
                    meta.level.name()
                })?,
                'L' => dest.push(match meta.level {
                    Level::Trace => 'T',
                    Level::Debug => 'D',
                    Level::Info => 'I',
                    Level::Warn => 'W',
                    Level::Error => 'E',
                    Level::Critical => 'C',
                    Level::Off => 'O',
                }),
                'n' => dest.write_str(meta.logger)?,
                'v' => dest.write_str(meta.message)?,
                's' => dest.write_str(meta.file.rsplit(['/', '\\']).next().unwrap_or(meta.file))?,
                'g' => dest.write_str(meta.file)?,
                '#' => write!(dest, "{}", meta.line)?,
                '!' => dest.write_str(meta.function)?,
                'P' => write!(dest, "{}", std::process::id())?,
                '^' => style_start = Some(dest.len()),
                '$' => {
                    if let Some(start) = style_start.take() {
                        ctx.set_style_range(Some(start..dest.len()));
                    }
                }
                _ => unreachable!("validated pattern"),
            }
        }
        dest.push('\n');
        Ok(())
    }
}
impl Formatter for TextFormatter {
    fn format(
        &self,
        record: &Record,
        dest: &mut StringBuf,
        ctx: &mut FormatterContext,
    ) -> spdlog::Result<()> {
        self.render(&Meta::from_record(record), dest, ctx)
            .map_err(spdlog::Error::FormatRecord)
    }
}

struct Meta<'a> {
    time: DateTime<Local>,
    level: Level,
    tid: u64,
    logger: &'a str,
    file: &'a str,
    line: u32,
    function: &'a str,
    message: &'a str,
}
impl<'a> Meta<'a> {
    fn from_record(record: &'a Record<'_>) -> Self {
        let loc = record.source_location();
        Self {
            time: record.time().into(),
            level: Level::from_backend(record.level()),
            tid: number(record, "zust_tid").unwrap_or(record.tid()),
            logger: record.logger_name().unwrap_or(""),
            file: loc.map_or("", |l| l.file()),
            line: loc.map_or(0, |l| l.line()),
            function: loc.map_or("", |l| l.module_path()),
            message: record.payload(),
        }
    }
}
pub(crate) fn number(record: &Record, key: &'static str) -> Option<u64> {
    record
        .key_values()
        .get(spdlog::kv::Key::__from_static_str(key))
        .and_then(|v| v.to_u64())
}
pub(crate) fn flag(record: &Record, key: &'static str) -> bool {
    record
        .key_values()
        .get(spdlog::kv::Key::__from_static_str(key))
        .and_then(|v| v.to_bool())
        .unwrap_or(false)
}

#[derive(Clone)]
pub(crate) struct JsonFormatter {
    service: String,
    node: String,
    run_id: String,
}
#[derive(Serialize)]
struct JsonLine<'a> {
    ts: String,
    level: &'a str,
    logger: &'a str,
    service: &'a str,
    node: &'a str,
    run_id: &'a str,
    pid: u32,
    tid: u64,
    file: &'a str,
    line: u32,
    function: &'a str,
    component: String,
    action: &'a str,
    message: &'a str,
}
impl JsonFormatter {
    pub(crate) fn new(cfg: &Config) -> Self {
        Self {
            service: cfg.service.clone(),
            node: cfg.node.clone(),
            run_id: cfg.run_id.clone(),
        }
    }
    fn render(&self, meta: &Meta<'_>, dest: &mut StringBuf) -> spdlog::Result<()> {
        let (component, action) = tags(meta.message);
        let line = JsonLine {
            ts: meta.time.format("%Y-%m-%dT%H:%M:%S%.6f").to_string(),
            level: meta.level.name(),
            logger: meta.logger,
            service: &self.service,
            node: &self.node,
            run_id: &self.run_id,
            pid: std::process::id(),
            tid: meta.tid,
            file: meta.file,
            line: meta.line,
            function: meta.function,
            component,
            action,
            message: meta.message,
        };
        let json =
            serde_json::to_string(&line).map_err(|e| spdlog::Error::Downstream(Box::new(e)))?;
        dest.push_str(&json);
        dest.push('\n');
        Ok(())
    }
}
impl Formatter for JsonFormatter {
    fn format(
        &self,
        record: &Record,
        dest: &mut StringBuf,
        _: &mut FormatterContext,
    ) -> spdlog::Result<()> {
        self.render(&Meta::from_record(record), dest)
    }
}
fn tags(message: &str) -> (String, &str) {
    let empty = (String::new(), "");
    if !message.starts_with('[') {
        return empty;
    }
    let Some(end) = message.find(']') else {
        return empty;
    };
    if end > 129 {
        return empty;
    }
    let Some((component, action)) = message[1..end].split_once(':') else {
        return empty;
    };
    if component.is_empty() || action.is_empty() {
        return empty;
    }
    if component == "nng"
        && let Some((scope, verb)) = action.split_once(':')
        && !verb.is_empty()
    {
        return (format!("nng.{scope}"), verb);
    }
    (component.to_owned(), action)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    fn meta() -> Meta<'static> {
        // A fixed offset avoids depending on the test host's local timezone.
        let time = chrono::FixedOffset::east_opt(0)
            .unwrap()
            .with_ymd_and_hms(2026, 10, 20, 12, 34, 56)
            .unwrap()
            .with_nanosecond(123456789)
            .unwrap()
            .with_timezone(&Local);
        Meta {
            time,
            level: Level::Warn,
            tid: 7,
            logger: "test",
            file: "src/main.rs",
            line: 42,
            function: "app::worker",
            message: "[nng:socket:create] 引号\"\n\t\\",
        }
    }
    #[test]
    fn default_text_and_all_supported_flags() {
        let m = meta();
        let mut output = StringBuf::new();
        TextFormatter::new("", "pattern")
            .unwrap()
            .render(&m, &mut output, &mut FormatterContext::default())
            .unwrap();
        assert_eq!(
            output,
            format!(
                "{} 07 W: {}\tsrc/main.rs:42\n",
                m.time.format("%d %H:%M:%S%.6f"),
                m.message
            )
        );
        output.clear();
        TextFormatter::new(
            "%% %Y-%m-%d %H:%M:%S %e %f %l %L %n %s %g %# %! %P",
            "pattern",
        )
        .unwrap()
        .render(&m, &mut output, &mut FormatterContext::default())
        .unwrap();
        assert!(output.contains("123 123456 warning W test main.rs src/main.rs 42 app::worker"));
        for pattern in ["%", "%Q", "%8l", "%-8l"] {
            assert!(TextFormatter::new(pattern, "pattern").is_err());
        }
    }
    #[test]
    fn json_schema_escape_and_tags() {
        let mut output = StringBuf::new();
        JsonFormatter::new(&Config {
            service: "svc".into(),
            node: "node".into(),
            run_id: "run".into(),
            ..Config::default()
        })
        .render(&meta(), &mut output)
        .unwrap();
        assert_eq!(output.lines().count(), 1);
        let json: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert_eq!(json.as_object().unwrap().len(), 14);
        assert_eq!(json["component"], "nng.socket");
        assert_eq!(json["action"], "create");
        assert_eq!(json["message"], meta().message);
        assert_eq!(json["line"], 42);
        assert_eq!(json["tid"], 7);
        assert_eq!(
            json["ts"],
            meta().time.format("%Y-%m-%dT%H:%M:%S%.6f").to_string()
        );
        assert_eq!(
            tags("[ctp-md:login-rsp] ok"),
            ("ctp-md".into(), "login-rsp")
        );
        for invalid in ["plain", "[empty]", "[:action]", "[comp:]", "[unterminated"] {
            assert_eq!(tags(invalid), (String::new(), ""));
        }
    }
}
