use zust_log::{Config, Level, SinkFormat};
#[test]
fn existing_zpp_and_trader_samples_load_without_conversion() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/zpp.json");
    let cfg = Config::from_zpp_file(path, "server").unwrap();
    assert!(cfg.r#async && cfg.console && cfg.rotating_file);
    assert_eq!(cfg.service, "server");
    assert_eq!(cfg.logger_name, "zpp");
    assert_eq!(cfg.max_size, 32);
    assert_eq!(cfg.rotats, 3);
    assert_eq!(cfg.queue_size, 8192);
    assert_eq!(cfg.flush_interval_sec, 2);
    assert_eq!(cfg.level, Level::Trace);
    assert_eq!(cfg.flush_level, Level::Warn);
    let cfg = Config::from_zpp_file(path, "zeta_ctp_md").unwrap();
    assert!(cfg.important.enabled);
    assert_eq!(cfg.important.format, SinkFormat::Jsonl);
    assert_eq!(cfg.important.max_size, 16);
    assert_eq!(cfg.important.rotats, 5);
    assert_eq!(cfg.important.level, Level::Warn);
    assert!(
        Config::from_zpp_file(path, "missing")
            .unwrap_err()
            .to_string()
            .contains("missing")
    );
}
#[test]
fn defaults_aliases_and_unknown_fields() {
    let cfg = Config::from_spd_json(
        r#"{"level":"warning","flush_level":"err","future":{"x":1},"important":{"enabled":true}}"#,
    )
    .unwrap();
    assert_eq!(cfg.level, Level::Warn);
    assert_eq!(cfg.flush_level, Level::Error);
    assert_eq!(cfg.important.name, "zpp.important.jsonl");
    assert!(!Config::default().rotating_file);
    for level in [
        "trace", "debug", "info", "warn", "warning", "err", "error", "critical", "off",
    ] {
        assert!(Config::from_spd_json(&format!(r#"{{"level":"{level}"}}"#)).is_ok());
    }
}
#[test]
fn malformed_fields_have_a_precise_location() {
    for (json, field) in [
        (r#"{"async":1}"#, "spd.async"),
        (r#"{"level":"fatal"}"#, "spd.level"),
        (r#"{"queue_size":0}"#, "spd.queue_size"),
        (r#"{"max_size":0}"#, "spd.max_size"),
        (r#"{"rotats":0}"#, "spd.rotats"),
        (r#"{"flush_interval_sec":0}"#, "spd.flush_interval_sec"),
        (r#"{"important":null}"#, "spd.important"),
        (r#"{"important":{"level":true}}"#, "spd.important.level"),
        (r#"{"important":{"format":"xml"}}"#, "spd.important.format"),
        (r#"{"important":{"max_size":-1}}"#, "spd.important.max_size"),
        (r#"{"name":""}"#, "spd.name"),
        (r#"{"max_size":18446744073709551615}"#, "spd.max_size"),
    ] {
        assert!(
            Config::from_spd_json(json)
                .unwrap_err()
                .to_string()
                .starts_with(field),
            "{json}"
        );
    }
    assert!(Config::from_spd_json("[]").is_err());
    assert!(Config::from_spd_json("{").is_err());
}
