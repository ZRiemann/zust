#![cfg(feature = "tracing")]
use serde_json::Value;
use std::{fs, process::Command};
use zust_log::{Config, ImportantConfig, Level, SinkFormat};

#[test]
fn isolated_tracing_scenarios() {
    for scenario in ["v2", "identities", "legacy", "conflict", "uninitialized"] {
        let path = std::env::temp_dir().join(format!(
            "zust-tracing-{}-{scenario}.jsonl",
            std::process::id()
        ));
        let child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "child", "--nocapture"])
            .env("ZUST_TRACING_SCENARIO", scenario)
            .env("ZUST_TRACING_PATH", &path)
            .env("RUST_LOG", "off")
            .output()
            .unwrap();
        assert!(
            child.status.success(),
            "{scenario}: {} {}",
            String::from_utf8_lossy(&child.stdout),
            String::from_utf8_lossy(&child.stderr)
        );
        let _ = fs::remove_file(path);
    }
}

#[test]
fn child() {
    let Ok(scenario) = std::env::var("ZUST_TRACING_SCENARIO") else {
        return;
    };
    if scenario == "uninitialized" {
        assert!(zust_log::init_tracing("warn").is_err());
        return;
    }
    let path = std::env::var("ZUST_TRACING_PATH").unwrap();
    let cfg = Config {
        r#async: true,
        console: false,
        service: "test".into(),
        node: if scenario == "identities" {
            "node".into()
        } else {
            String::new()
        },
        run_id: if scenario == "identities" {
            "caller-run".into()
        } else {
            String::new()
        },
        level: Level::Trace,
        important: ImportantConfig {
            enabled: true,
            name: path.clone(),
            level: Level::Trace,
            format: if scenario == "legacy" {
                SinkFormat::Jsonl
            } else {
                SinkFormat::JsonlV2
            },
            ..ImportantConfig::default()
        },
        ..Config::default()
    };
    let guard = zust_log::init(cfg).unwrap();
    assert!(zust_log::init_tracing("[broken").is_err());
    if scenario == "conflict" {
        tracing::subscriber::set_global_default(tracing::subscriber::NoSubscriber::default())
            .unwrap();
        assert!(zust_log::init_tracing("warn").is_err());
        guard.shutdown().unwrap();
        return;
    }
    zust_log::init_tracing("warn,accepted=trace").unwrap();
    assert!(zust_log::init_tracing("warn").is_err());
    let native_line = line!() + 1;
    zust_log::warn!("[owl:ready] native\nmessage");
    let root = tracing::trace_span!(target: "excluded", "root", value = 1_u64);
    root.record("value", 2_u64);
    let child = tracing::debug_span!(target: "excluded", parent: &root, "child", flag = true);
    let event_line = line!() + 1;
    tracing::warn!(target: "accepted", parent: &child, count = 42_u64, level = "spoof", service = "spoof", ratio = 1.5_f64, flag = true, label = "text", huge = u128::MAX, nan = f64::NAN, debug = ?vec![1, 2], "framework");
    let _entered = root.enter();
    std::thread::scope(|scope| {
        let span = child.clone();
        scope.spawn(move || {
            let _entered = span.enter();
            tracing::warn!(target: "accepted", worker = true);
        });
    });
    tracing::info!(target: "excluded", "must not appear");
    tracing::info!(target: "accepted", "allowed info");
    fn dynamic() {
        tracing::warn!(target: "accepted", "dynamic level");
    }
    zust_log::set_level(Level::Off);
    dynamic();
    zust_log::set_level(Level::Trace);
    dynamic();
    guard.shutdown().unwrap();
    guard.shutdown().unwrap();
    tracing::error!("after shutdown");
    let rows: Vec<Value> = fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(rows.len(), 5, "{rows:?}");
    if scenario == "legacy" {
        assert!(rows[0].get("schema_version").is_none());
        assert_eq!(rows[0]["message"], "[owl:ready] native\nmessage");
        assert_eq!(rows[1]["line"], event_line);
        assert!(
            rows[1]["message"]
                .as_str()
                .unwrap()
                .contains("\"count\":42")
        );
        return;
    }
    assert_eq!(rows[0]["schema_version"], 2);
    assert_eq!(rows[0]["message"], "native\nmessage");
    assert_eq!(rows[0]["component"], "owl");
    assert_eq!(rows[0]["action"], "ready");
    assert_eq!(rows[0]["target"], module_path!());
    assert_eq!(rows[0]["source"]["line"], native_line);
    assert_eq!(rows[0]["source"]["module_path"], module_path!());
    for key in ["logger", "function", "tid", "fields", "spans"] {
        assert!(rows[0].get(key).is_none(), "{key}");
    }
    if scenario == "identities" {
        assert_eq!(rows[0]["node"], "node");
        assert_eq!(rows[0]["run_id"], "caller-run");
    } else {
        assert!(rows[0].get("node").is_none());
        assert!(rows[0].get("run_id").is_none());
    }
    let timestamp = rows[0]["ts"].as_str().unwrap();
    assert!(timestamp.ends_with('Z'));
    assert_eq!(timestamp.len(), 27);
    chrono::DateTime::parse_from_rfc3339(timestamp).unwrap();
    let framework = &rows[1];
    assert_eq!(framework["message"], "framework");
    assert_eq!(framework["target"], "accepted");
    assert_eq!(framework["level"], "warn");
    assert_eq!(framework["service"], "test");
    assert_eq!(framework["fields"]["level"], "spoof");
    assert_eq!(framework["source"]["line"], event_line);
    assert!(
        framework["source"]["file"]
            .as_str()
            .unwrap()
            .ends_with("tests/tracing.rs")
    );
    assert_eq!(framework["fields"]["count"], 42);
    assert_eq!(framework["fields"]["ratio"], 1.5);
    assert_eq!(framework["fields"]["flag"], true);
    assert_eq!(framework["fields"]["label"], "text");
    assert_eq!(framework["fields"]["huge"], u128::MAX.to_string());
    assert_eq!(framework["fields"]["nan"], "NaN");
    assert_eq!(framework["fields"]["debug"], "[1, 2]");
    assert!(framework["fields"].get("message").is_none());
    assert_eq!(framework["spans"][0]["name"], "root");
    assert_eq!(framework["spans"][0]["fields"]["value"], 2);
    assert_eq!(framework["spans"][1]["name"], "child");
    assert_eq!(rows[2]["message"], "");
    assert_eq!(rows[2]["spans"][0]["name"], "root");
    assert_ne!(rows[0]["thread_index"], rows[2]["thread_index"]);
    assert_eq!(rows[4]["message"], "dynamic level");
}
