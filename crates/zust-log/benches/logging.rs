//! Bounded comparative benchmark; no network or application services.
use spdlog::{
    formatter::{Formatter, FormatterContext},
    sink::{RotatingFileSink, RotationPolicy},
};
use std::{hint::black_box, time::Instant};

#[derive(Clone)]
struct Plain;
impl Formatter for Plain {
    fn format(
        &self,
        record: &spdlog::Record,
        out: &mut spdlog::StringBuf,
        _: &mut FormatterContext,
    ) -> spdlog::Result<()> {
        out.push_str(record.payload());
        out.push('\n');
        Ok(())
    }
}
fn measure(name: &str, mut emit: impl FnMut(usize)) {
    let iterations = 50_000;
    for n in 0..2_000 {
        emit(n);
    }
    let mut samples = Vec::with_capacity(iterations);
    let started = Instant::now();
    for n in 0..iterations {
        let start = Instant::now();
        emit(black_box(n));
        samples.push(start.elapsed().as_nanos());
    }
    let elapsed = started.elapsed();
    samples.sort_unstable();
    println!(
        "{name}: iterations={iterations} throughput={:.0}/s p50={}ns p99={}ns",
        iterations as f64 / elapsed.as_secs_f64(),
        samples[iterations / 2],
        samples[iterations * 99 / 100]
    );
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    if !std::env::args().any(|arg| arg == "--run") {
        println!("Benchmark skipped; pass --run to execute the performance workflow.");
        return Ok(());
    }
    assert!(
        spdlog::STATIC_LEVEL_FILTER.test(spdlog::Level::Info),
        "benchmark requires info logging to be compiled in"
    );
    let directory = std::env::temp_dir().join(format!("zust-bench-{}", std::process::id()));
    std::fs::create_dir(&directory)?;
    let native_path = directory.join("native.log");
    let wrapped_path = directory.join("wrapped.log");
    // Both cases use identical size-rotating buffered sinks, no colors, payload
    // plus newline, no periodic/level flush, and no rotation during the run.
    let sink = RotatingFileSink::builder()
        .base_path(&native_path)
        .rotation_policy(RotationPolicy::FileSize(32 * 1024 * 1024))
        .max_files(4)
        .rotate_on_open(false)
        .formatter(Plain)
        .build_arc()?;
    let native = spdlog::Logger::builder()
        .sink(sink)
        .level_filter(spdlog::LevelFilter::All)
        .build_arc()?;
    measure(
        "native-enabled",
        |n| spdlog::info!(logger: native, "payload {n}"),
    );
    let guard = zust_log::init(zust_log::Config {
        console: false,
        rotating_file: true,
        name: wrapped_path.to_string_lossy().into_owned(),
        rotate_on_open: false,
        pattern: "%v".into(),
        flush_level: zust_log::Level::Off,
        flush_interval_sec: 0,
        ..Default::default()
    })?;
    measure("zust-enabled", |n| zust_log::info!("payload {n}"));
    native.set_level_filter(spdlog::LevelFilter::Off);
    zust_log::set_level(zust_log::Level::Off);
    measure(
        "native-disabled",
        |n| spdlog::info!(logger: native, "payload {}", black_box(n)),
    );
    measure("zust-disabled", |n| {
        zust_log::info!("payload {}", black_box(n))
    });
    guard.shutdown()?;
    native.flush();
    drop(native);
    assert_eq!(std::fs::read(&native_path)?, std::fs::read(&wrapped_path)?);
    std::fs::remove_file(native_path)?;
    std::fs::remove_file(wrapped_path)?;
    std::fs::remove_dir(directory)?;
    Ok(())
}
