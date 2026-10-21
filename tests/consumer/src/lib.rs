#[test]
fn renamed_facade_and_component_share_types_macros_and_runtime() {
    let path = std::env::temp_dir().join(format!("zust-renamed-{}.log", std::process::id()));
    let cfg: component::Config = zetax::log::Config {
        console: false,
        rotating_file: true,
        name: path.to_string_lossy().into_owned(),
        rotate_on_open: false,
        pattern: "%v".into(),
        flush_interval_sec: 0,
        ..Default::default()
    };
    let guard = zetax::log::init(cfg).unwrap();
    zetax::log::info!("facade {}", 1);
    component::info!("component {}", 2);
    assert!(matches!(
        component::init(component::Config::default()),
        Err(component::Error::AlreadyInitialized)
    ));
    guard.shutdown().unwrap();
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "facade 1\ncomponent 2\n"
    );
    std::fs::remove_file(path).unwrap();
}

#[test]
fn overflow_stats_are_shared_by_facade_and_component() {
    assert_eq!(zetax::log::stats().dropped, component::stats().dropped);
}
