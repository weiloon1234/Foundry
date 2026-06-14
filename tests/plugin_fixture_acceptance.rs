use std::path::PathBuf;
use std::process::Command;

fn fixture_manifest_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("plugin_consumer_app")
        .join("Cargo.toml")
}

fn fixture_target_dir(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(name)
}

#[test]
fn plugin_fixture_compiles() {
    let status = Command::new("cargo")
        .arg("check")
        .arg("--quiet")
        .arg("--manifest-path")
        .arg(fixture_manifest_path())
        .env(
            "CARGO_TARGET_DIR",
            fixture_target_dir("plugin-fixture-check"),
        )
        .status()
        .expect("plugin fixture cargo check should run");

    assert!(status.success(), "plugin fixture failed to compile");
}

#[test]
fn plugin_fixture_smoke_test_passes() {
    let status = Command::new("cargo")
        .arg("test")
        .arg("--quiet")
        .arg("--manifest-path")
        .arg(fixture_manifest_path())
        .arg("--test")
        .arg("plugin_bootstrap")
        .env(
            "CARGO_TARGET_DIR",
            fixture_target_dir("plugin-fixture-test"),
        )
        .status()
        .expect("plugin fixture cargo test should run");

    assert!(status.success(), "plugin fixture smoke test failed");
}
