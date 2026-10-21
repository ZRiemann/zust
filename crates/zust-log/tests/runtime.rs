use std::{
    fs,
    path::PathBuf,
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};
use zust_log::{Config, Error, ImportantConfig, Level, init};

fn run_scenario(scenario: &str) {
    let dir = std::env::temp_dir().join(format!("zust-test-{}-{scenario}", std::process::id()));
    fs::create_dir(&dir).unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "child", "--nocapture"])
        .env("ZUST_TEST_SCENARIO", scenario)
        .env("ZUST_TEST_DIRECTORY", &dir)
        .env_remove("ZUST_LOG_DEFINITELY_UNDEFINED")
        .env("ZUST_LOG_ROOT", &dir)
        .env("HOME", &dir)
        // Config is authoritative; native backend environment variables must
        // not override it implicitly.
        .env("SPDLOG_LEVEL", "off")
        .env("SPDLOG_RS_LEVEL", "off")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        if std::time::Instant::now() >= deadline {
            child.kill().unwrap();
            panic!("{scenario}: child timed out");
        }
        thread::sleep(Duration::from_millis(10));
    }
    let result = child.wait_with_output().unwrap();
    assert!(
        result.status.success(),
        "{scenario}:\n{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(!String::from_utf8_lossy(&result.stdout).contains('\u{1b}'));
    fs::remove_dir_all(dir).unwrap();
}
#[test]
fn runtime_scenarios() {
    for scenario in [
        "sync",
        "async",
        "threshold-info",
        "runtime-off",
        "append",
        "rotate-open",
        "rotation",
        "paths",
        "init-failure",
        "inflight",
        "concurrent",
        "drop",
        "output-failure",
        "periodic",
        "filter",
        "threads",
        "important-pattern",
        "flush-level",
    ] {
        run_scenario(scenario);
    }
}
#[test]
fn static_levels() {
    run_scenario("static");
}

#[test]
fn child() {
    let Ok(scenario) = std::env::var("ZUST_TEST_SCENARIO") else {
        return;
    };
    let dir = PathBuf::from(std::env::var_os("ZUST_TEST_DIRECTORY").unwrap());
    let runtime_path = dir.join("runtime.log");
    let important_path = dir.join("important.jsonl");
    let mut cfg = Config {
        console: false,
        rotating_file: true,
        name: runtime_path.to_string_lossy().into_owned(),
        pattern: "%t %L %v\t%g:%# %!".into(),
        flush_interval_sec: 0,
        flush_level: Level::Off,
        important: ImportantConfig {
            enabled: true,
            name: important_path.to_string_lossy().into_owned(),
            ..Default::default()
        },
        ..Default::default()
    };
    let json_lines = || -> Vec<serde_json::Value> {
        fs::read_to_string(&important_path)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    };
    match scenario.as_str() {
        "sync" | "async" | "threshold-info" | "runtime-off" => {
            cfg.r#async = scenario == "async";
            cfg.console = scenario == "sync";
            cfg.queue_size = 16;
            if scenario == "threshold-info" {
                cfg.important.level = Level::Info;
            }
            if scenario == "runtime-off" {
                cfg.level = Level::Off;
            }
            let mut evaluated = 0;
            zust_log::info!("before init {}", {
                evaluated += 1;
                evaluated
            });
            assert_eq!(evaluated, 0);
            let guard = init(cfg).unwrap();
            assert!(matches!(
                init(Config::default()),
                Err(Error::AlreadyInitialized)
            ));
            let worker = thread::spawn(move || {
                zust_log::info!("[system:heartbeat] ordinary");
                let line = line!() + 1;
                zust_log::warn!("[nng:socket:create] warning");
                zust_log::important!("[system:ready] ready \"\n\\");
                line
            });
            let expected_line = worker.join().unwrap();
            guard.flush().unwrap();
            guard.shutdown().unwrap();
            guard.shutdown().unwrap();
            zust_log::important!("after shutdown {}", {
                evaluated += 1;
                evaluated
            });
            assert_eq!(evaluated, 0);
            let lines = json_lines();
            assert_eq!(
                lines.len(),
                if scenario == "threshold-info" {
                    3
                } else if scenario == "runtime-off" {
                    1
                } else {
                    2
                }
            );
            let ready = lines.iter().find(|l| l["action"] == "ready").unwrap();
            assert_eq!(ready["level"], "info");
            assert!(ready.get("important").is_none());
            assert_eq!(ready["tid"], 0);
            assert_eq!(ready["function"], module_path!());
            if scenario != "runtime-off" {
                let warn = lines.iter().find(|l| l["action"] == "create").unwrap();
                assert_eq!(warn["line"], expected_line);
                assert_eq!(warn["file"], file!());
                assert_eq!(warn["component"], "nng.socket");
                assert_eq!(lines.last().unwrap()["action"], "ready");
                let text = fs::read_to_string(&runtime_path).unwrap();
                assert!(text.contains("00 I [system:heartbeat] ordinary"));
                assert_eq!(text.matches("[system:ready]").count(), 1);
                assert!(!text.contains('\u{1b}'));
            } else {
                assert!(fs::read_to_string(runtime_path).unwrap().is_empty());
            }
        }
        "append" | "rotate-open" => {
            fs::write(&runtime_path, "old\n").unwrap();
            cfg.rotate_on_open = scenario == "rotate-open";
            cfg.important.enabled = false;
            cfg.pattern = "%v".into();
            let guard = init(cfg).unwrap();
            zust_log::info!("new");
            guard.shutdown().unwrap();
            if scenario == "append" {
                assert_eq!(fs::read_to_string(runtime_path).unwrap(), "old\nnew\n");
            } else {
                assert_eq!(fs::read_to_string(runtime_path).unwrap(), "new\n");
                assert_eq!(
                    fs::read_to_string(dir.join("runtime_1.log")).unwrap(),
                    "old\n"
                );
            }
        }
        "rotation" => {
            cfg.max_size = 1;
            cfg.rotats = 2;
            cfg.important.enabled = false;
            cfg.pattern = "%v".into();
            let guard = init(cfg).unwrap();
            let payload = "x".repeat(700_000);
            for n in 0..7 {
                zust_log::info!("{n}{payload}");
            }
            guard.shutdown().unwrap();
            for (name, digit) in [
                ("runtime.log", '6'),
                ("runtime_1.log", '5'),
                ("runtime_2.log", '4'),
            ] {
                let text = fs::read_to_string(dir.join(name)).unwrap();
                assert!(text.starts_with(digit));
                assert_eq!(text.len(), 700_002);
            }
            assert!(!dir.join("runtime_3.log").exists());
        }
        "paths" => {
            cfg.name = "$ZUST_LOG_ROOT/sub/runtime.log".into();
            cfg.important.name = "~/sub/important.jsonl".into();
            let guard = init(cfg).unwrap();
            zust_log::important!("[path:ready] expanded");
            guard.shutdown().unwrap();
            assert!(dir.join("sub/runtime.log").is_file());
            assert!(dir.join("sub/important.jsonl").is_file());
        }
        "init-failure" => {
            let mut bad = cfg.clone();
            bad.important.name = "${ZUST_LOG_DEFINITELY_UNDEFINED}/bad".into();
            assert!(
                init(bad)
                    .err()
                    .unwrap()
                    .to_string()
                    .contains("spd.important.name")
            );
            assert!(!runtime_path.exists());
            let mut bad = cfg.clone();
            bad.pattern = "%8l".into();
            assert!(init(bad).is_err());
            let mut bad = cfg.clone();
            bad.name = dir.to_string_lossy().into_owned();
            assert!(init(bad).is_err());
            let guard = init(cfg).unwrap();
            guard.shutdown().unwrap();
        }
        "inflight" => {
            struct Paused {
                entered: mpsc::Sender<()>,
                resume: MutexReceiver,
            }
            type MutexReceiver = std::sync::Mutex<mpsc::Receiver<()>>;
            impl std::fmt::Display for Paused {
                fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    self.entered.send(()).unwrap();
                    self.resume.lock().unwrap().recv().unwrap();
                    f.write_str("accepted")
                }
            }
            cfg.r#async = true;
            cfg.pattern = "%v".into();
            let guard = Arc::new(init(cfg).unwrap());
            let (entered, enter) = mpsc::channel();
            let (resume, resumed) = mpsc::channel();
            let producer = thread::spawn(move || {
                zust_log::info!(
                    "{}",
                    Paused {
                        entered,
                        resume: std::sync::Mutex::new(resumed)
                    }
                )
            });
            enter.recv_timeout(Duration::from_secs(2)).unwrap();
            let (finished, finish) = mpsc::channel();
            let closing = Arc::clone(&guard);
            let closer = thread::spawn(move || {
                closing.shutdown().unwrap();
                finished.send(()).unwrap();
            });
            assert!(finish.recv_timeout(Duration::from_millis(30)).is_err());
            resume.send(()).unwrap();
            producer.join().unwrap();
            closer.join().unwrap();
            assert_eq!(fs::read_to_string(runtime_path).unwrap(), "accepted\n");
        }
        "concurrent" => {
            cfg.r#async = true;
            cfg.queue_size = 1;
            cfg.pattern = "%v".into();
            cfg.important.enabled = false;
            let guard = Arc::new(init(cfg).unwrap());
            let accepted = Arc::new(AtomicUsize::new(0));
            let producers: Vec<_> = (0..4)
                .map(|_| {
                    let accepted = Arc::clone(&accepted);
                    thread::spawn(move || {
                        for _ in 0..1_000 {
                            zust_log::info!("{}", accepted.fetch_add(1, Ordering::SeqCst));
                        }
                    })
                })
                .collect();
            while accepted.load(Ordering::SeqCst) < 10 {
                thread::yield_now();
            }
            let closers: Vec<_> = (0..3)
                .map(|_| {
                    let guard = Arc::clone(&guard);
                    thread::spawn(move || guard.shutdown().unwrap())
                })
                .collect();
            for thread in producers.into_iter().chain(closers) {
                thread.join().unwrap();
            }
            let text = fs::read_to_string(runtime_path).unwrap();
            let mut ids: Vec<usize> = text
                .lines()
                .filter(|l| !l.contains("[logging:overflow]"))
                .map(|l| l.parse().unwrap())
                .collect();
            let total = accepted.load(Ordering::SeqCst);
            assert_eq!(ids.len() + zust_log::stats().dropped as usize, total);
            ids.sort_unstable();
            assert!(ids.windows(2).all(|pair| pair[0] < pair[1]));
            assert!(ids.iter().all(|id| *id < total));
        }
        "drop" => {
            cfg.r#async = true;
            let guard = init(cfg).unwrap();
            for n in 0..100 {
                zust_log::important!("[drop:drain] {n}");
            }
            drop(guard);
            assert_eq!(json_lines().len(), 100);
        }
        "output-failure" => {
            cfg.r#async = true;
            cfg.max_size = 1;
            cfg.rotats = 1;
            cfg.important.enabled = false;
            let guard = init(cfg).unwrap();
            // Renaming the active file into an occupied directory must fail.
            fs::create_dir(dir.join("runtime_1.log")).unwrap();
            fs::write(dir.join("runtime_1.log/keep"), "sentinel").unwrap();
            zust_log::error!("{}", "x".repeat(1_100_000));
            assert!(matches!(guard.shutdown(), Err(Error::Output(_))));
            assert!(guard.shutdown().is_err());
        }
        "periodic" => {
            cfg.r#async = true;
            cfg.flush_interval_sec = 1;
            cfg.pattern = "%v".into();
            let guard = init(cfg).unwrap();
            zust_log::info!("periodic");
            let deadline = std::time::Instant::now() + Duration::from_secs(4);
            while fs::read_to_string(&runtime_path).unwrap() != "periodic\n" {
                assert!(std::time::Instant::now() < deadline);
                thread::sleep(Duration::from_millis(20));
            }
            guard.shutdown().unwrap();
        }
        "filter" => {
            cfg.level = Level::Warn;
            let guard = init(cfg).unwrap();
            let mut evaluated = 0;
            zust_log::info!("suppressed {}", {
                evaluated += 1;
                evaluated
            });
            assert_eq!(evaluated, 0);
            zust_log::important!("[filter:first] {}", {
                evaluated += 1;
                evaluated
            });
            zust_log::set_level(Level::Off);
            zust_log::important!("[filter:second] {}", {
                evaluated += 1;
                evaluated
            });
            zust_log::set_level(Level::Trace);
            zust_log::info!("ordinary {}", {
                evaluated += 1;
                evaluated
            });
            zust_log::warn!("[filter:warn] warning");
            guard.shutdown().unwrap();
            assert_eq!(evaluated, 3);
            assert_eq!(json_lines().len(), 3);
            let text = fs::read_to_string(runtime_path).unwrap();
            assert!(!text.contains("filter:first"));
            assert!(!text.contains("filter:second"));
            assert!(text.contains("ordinary 3"));
        }
        "threads" => {
            cfg.r#async = true;
            let guard = init(cfg).unwrap();
            for _ in 0..110 {
                thread::spawn(|| zust_log::important!("[thread:unique] payload"))
                    .join()
                    .unwrap();
            }
            guard.shutdown().unwrap();
            let mut ids: Vec<u64> = json_lines()
                .iter()
                .map(|line| line["tid"].as_u64().unwrap())
                .collect();
            ids.sort_unstable();
            assert_eq!(ids, (0..110).collect::<Vec<_>>());
            let text = fs::read_to_string(runtime_path).unwrap();
            assert!(text.starts_with("00 I"));
            assert!(text.contains("109 I"));
        }
        "important-pattern" => {
            cfg.important.format = zust_log::SinkFormat::Pattern;
            cfg.important.pattern = "%L %v".into();
            cfg.important.level = Level::Off;
            let guard = init(cfg).unwrap();
            zust_log::warn!("ordinary");
            zust_log::important!("explicit");
            guard.shutdown().unwrap();
            assert_eq!(fs::read_to_string(important_path).unwrap(), "I explicit\n");
        }
        "flush-level" => {
            cfg.flush_level = Level::Warn;
            cfg.pattern = "%v".into();
            let guard = init(cfg).unwrap();
            zust_log::info!("buffered");
            assert!(fs::read_to_string(&runtime_path).unwrap().is_empty());
            zust_log::warn!("flushed");
            assert_eq!(
                fs::read_to_string(&runtime_path).unwrap(),
                "buffered\nflushed\n"
            );
            guard.shutdown().unwrap();
        }
        "static" => {
            cfg.important.enabled = false;
            cfg.pattern = "%v".into();
            let guard = init(cfg).unwrap();
            let mut count = 0;
            zust_log::trace!("{}", {
                count += 1;
                count
            });
            zust_log::debug!("{}", {
                count += 1;
                count
            });
            zust_log::info!("{}", {
                count += 1;
                count
            });
            zust_log::warn!("{}", {
                count += 1;
                count
            });
            zust_log::error!("{}", {
                count += 1;
                count
            });
            zust_log::critical!("{}", {
                count += 1;
                count
            });
            zust_log::important!("{}", {
                count += 1;
                count
            });
            let expected = [
                spdlog::Level::Trace,
                spdlog::Level::Debug,
                spdlog::Level::Info,
                spdlog::Level::Warn,
                spdlog::Level::Error,
                spdlog::Level::Critical,
                spdlog::Level::Info,
            ]
            .into_iter()
            .filter(|l| spdlog::STATIC_LEVEL_FILTER.test(*l))
            .count();
            assert_eq!(count, expected);
            guard.shutdown().unwrap();
            assert_eq!(
                fs::read_to_string(runtime_path).unwrap().lines().count(),
                expected
            );
        }
        _ => panic!("unknown scenario {scenario}"),
    }
}
