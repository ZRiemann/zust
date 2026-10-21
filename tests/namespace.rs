#![cfg(feature = "log")]
#[test]
fn z_log_facade_preserves_call_site_and_macro_hygiene() {
    let path = std::env::temp_dir().join(format!("zust-facade-{}.log", std::process::id()));
    let guard = z::log::init(z::log::Config {
        console: false,
        rotating_file: true,
        name: path.to_string_lossy().into_owned(),
        rotate_on_open: false,
        pattern: "%v\t%g:%#".into(),
        flush_interval_sec: 0,
        ..Default::default()
    })
    .unwrap();
    let line = line!() + 1;
    z::log::info!("{}", "facade");
    guard.shutdown().unwrap();
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        format!("facade\t{}:{line}\n", file!())
    );
    std::fs::remove_file(path).unwrap();
}
