use crate::{
    __private::Source,
    Config, Error, Level, Result, SinkFormat,
    config::{expand_path, file_limits},
    format::{JsonFormatter, TextFormatter, flag},
};
use spdlog::{
    Record,
    sink::{
        AsyncPoolSink, GetSinkProp, OverflowPolicy, RotatingFileSink, RotationPolicy, Sink,
        SinkProp, StdStreamSink,
    },
};
use std::{
    cell::Cell,
    fmt,
    io::Write,
    sync::{
        Arc, Mutex, OnceLock, RwLock, RwLockReadGuard,
        atomic::{AtomicBool, AtomicU8, AtomicU64, AtomicUsize, Ordering},
    },
    time::Duration,
};

static GLOBAL: OnceLock<Runtime> = OnceLock::new();
static INIT_LOCK: Mutex<()> = Mutex::new(());
static NEXT_TID: AtomicU64 = AtomicU64::new(0);
thread_local! { static SHORT_TID: Cell<Option<u64>> = const { Cell::new(None) }; }
fn short_tid() -> u64 {
    SHORT_TID.with(|slot| match slot.get() {
        Some(tid) => tid,
        None => {
            let tid = NEXT_TID.fetch_add(1, Ordering::Relaxed);
            slot.set(Some(tid));
            tid
        }
    })
}

#[derive(Default)]
struct Errors {
    count: AtomicUsize,
    first: Mutex<Option<String>>,
}
impl Errors {
    fn report(&self, error: impl ToString) {
        let mut first = self.first.lock().unwrap_or_else(|e| e.into_inner());
        self.count.fetch_add(1, Ordering::Relaxed);
        if first.is_none() {
            let message = error.to_string();
            // Do not recurse into logging, or panic if stderr itself is broken.
            let _ = writeln!(
                std::io::stderr().lock(),
                "zust-log output failure: {message}"
            );
            *first = Some(message);
        }
    }
    fn result(&self) -> Result<()> {
        let first = self.first.lock().unwrap_or_else(|e| e.into_inner());
        match first.as_ref() {
            Some(first) => Err(Error::Output(format!(
                "{} failure(s); first: {first}",
                self.count.load(Ordering::Relaxed)
            ))),
            None => Ok(()),
        }
    }
}

struct Router {
    prop: SinkProp,
    runtime: Vec<Arc<dyn Sink>>,
    important: Option<Arc<dyn Sink>>,
}
impl GetSinkProp for Router {
    fn prop(&self) -> &SinkProp {
        &self.prop
    }
}
fn combine(errors: Vec<spdlog::Error>) -> spdlog::Result<()> {
    if errors.is_empty() {
        Ok(())
    } else {
        Err(spdlog::Error::Multiple(errors))
    }
}
impl Sink for Router {
    fn log(&self, record: &Record) -> spdlog::Result<()> {
        let mut errors = Vec::new();
        if flag(record, "zust_runtime") {
            for sink in &self.runtime {
                if let Err(error) = sink.log(record) {
                    errors.push(error);
                }
            }
        }
        if flag(record, "zust_important")
            && let Some(sink) = &self.important
            && let Err(error) = sink.log(record)
        {
            errors.push(error);
        }
        combine(errors)
    }
    fn flush(&self) -> spdlog::Result<()> {
        let mut errors = Vec::new();
        for sink in self.runtime.iter().chain(self.important.iter()) {
            if let Err(error) = sink.flush() {
                errors.push(error);
            }
        }
        combine(errors)
    }
}
struct Active {
    logger: Arc<spdlog::Logger>,
    root: Arc<dyn Sink>,
}
struct Runtime {
    enabled: AtomicBool,
    level: AtomicU8,
    important_level: Level,
    has_runtime: bool,
    has_important: bool,
    active: RwLock<Option<Active>>,
    errors: Arc<Errors>,
}
impl Runtime {
    fn accepts_runtime(&self, level: Level) -> bool {
        self.has_runtime && (level as u8) >= self.level.load(Ordering::Relaxed)
    }
    fn accepts(&self, level: Level, important: bool) -> bool {
        self.accepts_runtime(level)
            || (important && self.has_important)
            || (self.has_important
                && (level as u8) >= self.level.load(Ordering::Relaxed)
                && level >= self.important_level)
    }
    fn shutdown(&self) -> Result<()> {
        self.enabled.store(false, Ordering::Release);
        // Tickets hold a read lock through argument evaluation and submission.
        // The write lock also serializes competing shutdown calls to completion.
        let mut state = self.active.write().unwrap_or_else(|e| e.into_inner());
        if let Some(active) = state.take() {
            active.logger.set_flush_period(None);
            // Unlike async flush(), this backend operation drains and joins the
            // pool before flushing the underlying destinations synchronously.
            if let Err(error) = active.root.flush_on_exit() {
                self.errors.report(error);
            }
        }
        self.errors.result()
    }
}

/// Owns the process logging lifetime. Stop/join producers before shutting down.
/// Calling shutdown from a log argument's Display implementation is unsupported:
/// that invocation still holds its own submission ticket.
#[must_use = "dropping the guard closes logging"]
pub struct LogGuard {
    runtime: &'static Runtime,
}
impl LogGuard {
    /// Submit a flush. In async mode, success is not a completion barrier.
    /// Previously observed output failures are returned when available.
    pub fn flush(&self) -> Result<()> {
        let state = self
            .runtime
            .active
            .read()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(active) = state.as_ref() {
            active.logger.flush();
        }
        self.runtime.errors.result()
    }
    /// Stop accepting records, drain the queue, flush and join workers.
    /// Repeated calls are safe and return any previously recorded failure.
    pub fn shutdown(&self) -> Result<()> {
        self.runtime.shutdown()
    }
}
impl Drop for LogGuard {
    fn drop(&mut self) {
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.runtime.shutdown()))
            .is_err()
        {
            self.runtime
                .errors
                .report("backend panicked during shutdown");
        }
    }
}

/// Initialize once per process. Failed initialization can be retried; after a
/// successful init, even a closed runtime cannot be initialized again.
pub fn init(config: Config) -> Result<LogGuard> {
    let _init = INIT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    if GLOBAL.get().is_some() {
        return Err(Error::AlreadyInitialized);
    }
    let runtime = prepare(config)?;
    // INIT_LOCK excludes another successful initializer.
    let _ = GLOBAL.set(runtime);
    Ok(LogGuard {
        runtime: GLOBAL.get().expect("runtime installed under init lock"),
    })
}

fn prepare(config: Config) -> Result<Runtime> {
    config.validate("spd")?;
    let text = TextFormatter::new(&config.pattern, "spd.pattern")?;
    let imp_text = TextFormatter::new(
        if config.important.pattern.is_empty() {
            &config.pattern
        } else {
            &config.important.pattern
        },
        "spd.important.pattern",
    )?;
    // Validate names and resolve every enabled path before opening any file.
    let mut builder = spdlog::Logger::builder();
    builder
        .name(&config.logger_name)
        .level_filter(spdlog::LevelFilter::All)
        .flush_level_filter(config.flush_level.filter());
    builder
        .clone()
        .build()
        .map_err(|e| Error::config("spd.logger_name", e))?;
    let runtime_path = if config.rotating_file {
        Some(expand_path(&config.name, "spd.name")?)
    } else {
        None
    };
    let important_path = if config.important.enabled {
        let name = if config.important.name.is_empty() {
            format!("{}.important.jsonl", config.logger_name)
        } else {
            config.important.name.clone()
        };
        Some(expand_path(&name, "spd.important.name")?)
    } else {
        None
    };
    if runtime_path.is_some() && runtime_path == important_path {
        return Err(Error::config(
            "spd.important.name",
            "runtime and important sinks must use different files",
        ));
    }
    let errors = Arc::new(Errors::default());
    let mut sinks: Vec<Arc<dyn Sink>> = Vec::new();
    if config.console {
        sinks.push(
            StdStreamSink::builder()
                .stdout()
                .formatter(text.clone())
                .build_arc()
                .map_err(|e| Error::config("spd.console", e))?,
        );
    }
    if let Some(path) = runtime_path {
        let (size, files) = file_limits(config.max_size, config.rotats, "spd")?;
        sinks.push(
            RotatingFileSink::builder()
                .base_path(path)
                .rotation_policy(RotationPolicy::FileSize(size))
                .max_files(files)
                .rotate_on_open(config.rotate_on_open)
                .formatter(text)
                .build_arc()
                .map_err(|e| Error::config("spd.name", e))?,
        );
    }
    let has_runtime = !sinks.is_empty();
    let important: Option<Arc<dyn Sink>> = if let Some(path) = important_path {
        let (size, files) = file_limits(
            config.important.max_size,
            config.important.rotats,
            "spd.important",
        )?;
        let sink = RotatingFileSink::builder()
            .base_path(path)
            .rotation_policy(RotationPolicy::FileSize(size))
            .max_files(files)
            .rotate_on_open(config.important.rotate_on_open)
            .build_arc()
            .map_err(|e| Error::config("spd.important.name", e))?;
        use spdlog::sink::SinkPropAccess;
        match config.important.format {
            SinkFormat::Jsonl => sink.set_formatter(Box::new(JsonFormatter::new(&config))),
            SinkFormat::Pattern => sink.set_formatter(Box::new(imp_text)),
        }
        Some(sink)
    } else {
        None
    };
    let root: Arc<dyn Sink> = Arc::new(Router {
        prop: SinkProp::default(),
        runtime: sinks,
        important,
    });
    let root = if config.r#async {
        asynchronous(root, config.queue_size, &errors)?
    } else {
        root
    };
    let handler = Arc::clone(&errors);
    let logger = builder
        .sink(Arc::clone(&root))
        .error_handler(move |e| handler.report(e))
        .build_arc()
        .map_err(|e| Error::Output(e.to_string()))?;
    if config.flush_interval_sec > 0 {
        logger.set_flush_period(Some(Duration::from_secs(config.flush_interval_sec)));
    }
    Ok(Runtime {
        enabled: AtomicBool::new(true),
        level: AtomicU8::new(config.level as u8),
        has_runtime,
        has_important: config.important.enabled,
        important_level: config.important.level,
        active: RwLock::new(Some(Active { logger, root })),
        errors,
    })
}

fn asynchronous(
    root: Arc<dyn Sink>,
    capacity: usize,
    errors: &Arc<Errors>,
) -> Result<Arc<dyn Sink>> {
    let pool = spdlog::ThreadPool::builder()
        .capacity(capacity.try_into().expect("validated queue size"))
        .build_arc()
        .map_err(|e| Error::Output(e.to_string()))?;
    let handler = Arc::clone(errors);
    let sink = AsyncPoolSink::builder()
        .sink(root)
        .thread_pool(pool)
        .overflow_policy(OverflowPolicy::Block)
        .error_handler(move |e| handler.report(e))
        .build_arc()
        .map_err(|e| Error::Output(e.to_string()))?;
    Ok(sink as Arc<dyn Sink>)
}

/// Set the ordinary channel's level. The explicit important channel is independent.
/// Before initialization or after shutdown this has no observable effect.
pub fn set_level(level: Level) {
    if let Some(runtime) = GLOBAL.get() {
        runtime.level.store(level as u8, Ordering::Relaxed);
    }
}

/// Implementation-only submission ticket used by the exported logging macros.
#[doc(hidden)]
pub struct Ticket {
    state: RwLockReadGuard<'static, Option<Active>>,
    level: Level,
    runtime: bool,
    important: bool,
}
/// Acquire before evaluating formatting arguments. Not an application API.
#[doc(hidden)]
pub fn acquire(level: Level, important: bool) -> Option<Ticket> {
    if level == Level::Off || !spdlog::STATIC_LEVEL_FILTER.test(level.backend()) {
        return None;
    }
    let runtime = GLOBAL.get()?;
    if !runtime.enabled.load(Ordering::Acquire) || !runtime.accepts(level, important) {
        return None;
    }
    let state = runtime.active.read().unwrap_or_else(|e| e.into_inner());
    if state.is_none() || !runtime.enabled.load(Ordering::Acquire) {
        return None;
    }
    let ordinary = (level as u8) >= runtime.level.load(Ordering::Relaxed);
    let runtime_allowed = ordinary && runtime.has_runtime;
    let important_allowed =
        runtime.has_important && (important || (ordinary && level >= runtime.important_level));
    if !runtime_allowed && !important_allowed {
        return None;
    }
    Some(Ticket {
        state,
        level,
        runtime: runtime_allowed,
        important: important_allowed,
    })
}
impl Ticket {
    /// Submit to the backend with original caller metadata.
    pub fn emit(self, args: fmt::Arguments<'_>, source: Source) {
        let active = self.state.as_ref().expect("ticket holds an active runtime");
        // spdlog-rs 0.5.3's exported macro support entry point allows preserving
        // source location without a second formatting pass or message prefixes.
        // These pinned backend macro-support APIs are covered by consumer tests.
        spdlog::__log(
            &active.logger,
            self.level.backend(),
            Some(spdlog::SourceLocation::__new(
                source.function,
                source.file,
                source.line,
                0,
            )),
            &[
                (
                    spdlog::kv::Key::__from_static_str("zust_tid"),
                    short_tid().into(),
                ),
                (
                    spdlog::kv::Key::__from_static_str("zust_runtime"),
                    self.runtime.into(),
                ),
                (
                    spdlog::kv::Key::__from_static_str("zust_important"),
                    self.important.into(),
                ),
            ],
            args,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{sync::mpsc, thread};

    struct PausedSink {
        prop: SinkProp,
        entered: mpsc::Sender<()>,
        resume: Mutex<mpsc::Receiver<()>>,
        count: AtomicUsize,
    }
    impl GetSinkProp for PausedSink {
        fn prop(&self) -> &SinkProp {
            &self.prop
        }
    }
    impl Sink for PausedSink {
        fn log(&self, _: &Record) -> spdlog::Result<()> {
            if self.count.fetch_add(1, Ordering::SeqCst) == 0 {
                self.entered.send(()).unwrap();
                self.resume.lock().unwrap().recv().unwrap();
            }
            Ok(())
        }
        fn flush(&self) -> spdlog::Result<()> {
            Ok(())
        }
    }
    #[test]
    fn bounded_backend_queue_blocks_and_drains_without_loss() {
        let (entered, enter) = mpsc::channel();
        let (resume, resumed) = mpsc::channel();
        let sink = Arc::new(PausedSink {
            prop: SinkProp::default(),
            entered,
            resume: Mutex::new(resumed),
            count: AtomicUsize::new(0),
        });
        let errors = Arc::new(Errors::default());
        let root = asynchronous(sink.clone(), 1, &errors).unwrap();
        let logger = spdlog::Logger::builder()
            .sink(root.clone())
            .build_arc()
            .unwrap();
        spdlog::info!(logger: logger, "worker blocks here");
        enter.recv_timeout(Duration::from_secs(2)).unwrap();
        spdlog::info!(logger: logger, "queue now full");
        let (finished, finish) = mpsc::channel();
        let producer = thread::spawn(move || {
            spdlog::info!(logger: logger, "must block until capacity is available");
            finished.send(()).unwrap();
        });
        let blocked = finish.recv_timeout(Duration::from_millis(30)).is_err();
        resume.send(()).unwrap();
        producer.join().unwrap();
        root.flush_on_exit().unwrap();
        assert!(blocked);
        assert_eq!(sink.count.load(Ordering::SeqCst), 3);
        errors.result().unwrap();
    }
}
