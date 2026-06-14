use std::path::PathBuf;
use std::process::Command;

fn fixture_manifest_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("blueprint_app")
        .join("Cargo.toml")
}

fn fixture_target_dir(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(name)
}

#[test]
fn blueprint_fixture_compiles() {
    let status = Command::new("cargo")
        .arg("check")
        .arg("--quiet")
        .arg("--manifest-path")
        .arg(fixture_manifest_path())
        .env(
            "CARGO_TARGET_DIR",
            fixture_target_dir("blueprint-fixture-check"),
        )
        .status()
        .expect("blueprint fixture cargo check should run");

    assert!(status.success(), "blueprint fixture failed to compile");
}

#[test]
fn blueprint_fixture_split_bootstrap_regression() {
    let status = Command::new("cargo")
        .arg("test")
        .arg("--quiet")
        .arg("--manifest-path")
        .arg(fixture_manifest_path())
        .arg("--test")
        .arg("split_bootstrap")
        .env(
            "CARGO_TARGET_DIR",
            fixture_target_dir("blueprint-fixture-test"),
        )
        .status()
        .expect("blueprint fixture cargo test should run");

    assert!(
        status.success(),
        "blueprint fixture split-bootstrap regression failed"
    );
}
