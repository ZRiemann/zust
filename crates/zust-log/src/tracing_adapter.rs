use crate::{__private::Source, Error, Level, Result, format::Diagnostics, runtime};
use serde_json::{Map, Value, json};
use std::fmt;
use tracing::{
    Event, Metadata, Subscriber,
    field::{Field, Visit},
    span::{Attributes, Id, Record},
};
use tracing_subscriber::{
    EnvFilter, Layer, Registry,
    layer::{Context, SubscriberExt},
    registry::LookupSpan,
};

/// Install one global tracing subscriber after `init`, without another output or queue.
/// The explicit filter is authoritative; RUST_LOG is not read. Span data remains
/// available even when its own level/target is excluded from event output.
pub fn init_tracing(filter: &str) -> Result<()> {
    let filter =
        EnvFilter::try_new(filter).map_err(|error| Error::config("tracing.filter", error))?;
    if !runtime::initialized() {
        return Err(Error::config(
            "tracing",
            "initialize logging before tracing",
        ));
    }
    tracing::subscriber::set_global_default(Registry::default().with(Adapter { filter }))
        .map_err(|error| Error::config("tracing", error))
}
fn level(level: &tracing::Level) -> Level {
    match *level {
        tracing::Level::TRACE => Level::Trace,
        tracing::Level::DEBUG => Level::Debug,
        tracing::Level::INFO => Level::Info,
        tracing::Level::WARN => Level::Warn,
        tracing::Level::ERROR => Level::Error,
    }
}
#[derive(Default)]
struct Fields(Map<String, Value>);
impl Visit for Fields {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.0
            .insert(field.name().into(), format!("{value:?}").into());
    }
    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.insert(field.name().into(), value.into());
    }
    fn record_bool(&mut self, field: &Field, value: bool) {
        self.0.insert(field.name().into(), value.into());
    }
    fn record_i64(&mut self, field: &Field, value: i64) {
        self.0.insert(field.name().into(), value.into());
    }
    fn record_u64(&mut self, field: &Field, value: u64) {
        self.0.insert(field.name().into(), value.into());
    }
    fn record_i128(&mut self, field: &Field, value: i128) {
        let number = i64::try_from(value)
            .map(Value::from)
            .or_else(|_| u64::try_from(value).map(Value::from));
        self.0.insert(
            field.name().into(),
            number.unwrap_or_else(|_| value.to_string().into()),
        );
    }
    fn record_u128(&mut self, field: &Field, value: u128) {
        self.0.insert(
            field.name().into(),
            u64::try_from(value).map_or_else(|_| value.to_string().into(), Value::from),
        );
    }
    fn record_f64(&mut self, field: &Field, value: f64) {
        self.0.insert(
            field.name().into(),
            serde_json::Number::from_f64(value)
                .map_or_else(|| value.to_string().into(), Value::Number),
        );
    }
}
struct Adapter {
    filter: EnvFilter,
}
impl<S> Layer<S> for Adapter
where
    S: Subscriber + for<'lookup> LookupSpan<'lookup>,
{
    fn register_callsite(
        &self,
        metadata: &'static Metadata<'static>,
    ) -> tracing::subscriber::Interest {
        let _ = <EnvFilter as Layer<S>>::register_callsite(&self.filter, metadata);
        tracing::subscriber::Interest::sometimes()
    }
    fn enabled(&self, metadata: &Metadata<'_>, ctx: Context<'_, S>) -> bool {
        metadata.is_span()
            || (runtime::accepts(level(metadata.level())) && self.filter.enabled(metadata, ctx))
    }
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        self.filter.on_new_span(attrs, id, ctx.clone());
        if let Some(span) = ctx.span(id) {
            let mut fields = Fields::default();
            attrs.record(&mut fields);
            span.extensions_mut().insert(fields);
        }
    }
    fn on_record(&self, id: &Id, record: &Record<'_>, ctx: Context<'_, S>) {
        self.filter.on_record(id, record, ctx.clone());
        if let Some(span) = ctx.span(id) {
            let mut extensions = span.extensions_mut();
            if let Some(fields) = extensions.get_mut::<Fields>() {
                record.record(fields);
            }
        }
    }
    fn on_enter(&self, id: &Id, ctx: Context<'_, S>) {
        self.filter.on_enter(id, ctx);
    }
    fn on_exit(&self, id: &Id, ctx: Context<'_, S>) {
        self.filter.on_exit(id, ctx);
    }
    fn on_close(&self, id: Id, ctx: Context<'_, S>) {
        self.filter.on_close(id, ctx);
    }
    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        if !self.filter.enabled(event.metadata(), ctx.clone())
            || !self.filter.event_enabled(event, ctx.clone())
        {
            return;
        }
        let metadata = event.metadata();
        let Some(ticket) = runtime::acquire(level(metadata.level()), false) else {
            return;
        };
        let mut fields = Fields::default();
        event.record(&mut fields);
        let message = fields
            .0
            .remove("message")
            .map_or_else(String::new, |value| {
                value
                    .as_str()
                    .map_or_else(|| value.to_string(), str::to_owned)
            });
        let spans = ctx.event_scope(event).map_or_else(Vec::new, |scope| scope.from_root().map(|span| {
            let extensions = span.extensions();
            let fields = extensions.get::<Fields>().map(|fields| &fields.0);
            json!({"name": span.name(), "target": span.metadata().target(), "fields": fields.cloned().unwrap_or_default()})
        }).collect());
        let diagnostic = Diagnostics {
            target: metadata.target().into(),
            fields: fields.0,
            spans,
        };
        // Only JSON scalar values are stored, so serialization cannot fail.
        let diagnostic = serde_json::to_string(&diagnostic).expect("JSON diagnostic values");
        ticket.emit_record(
            format_args!("{message}"),
            Source {
                file: metadata.file().unwrap_or(""),
                line: metadata.line().unwrap_or(0),
                function: metadata.module_path().unwrap_or(""),
            },
            &diagnostic,
        );
    }
}
