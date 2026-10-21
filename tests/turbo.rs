mod common;
#[path = "common/turbo.rs"]
mod turbo_fixture;

use common::{failure, success};
use turbo_fixture::{TurboFixture, write_json};

#[test]
fn task_selectors_preserve_exact_pairs_and_allow_turbo_prerequisites() {
    let t = TurboFixture::new();
    success(
        t.cli()
            .args(["task", "@example/protocol:build", "@example/ui:pack"])
            .output()
            .unwrap(),
    );
    assert!(t.marker("protocol", "prepare").is_file());
    assert!(t.marker("protocol", "build").is_file());
    assert!(t.marker("ui", "pack").is_file());
    assert!(!t.marker("protocol", "pack").exists());
    assert!(!t.marker("ui", "build").exists());
    assert!(!t.marker("ui", "prepare").exists());
}

#[test]
fn task_validates_every_selected_script_before_executing_any_pair() {
    let t = TurboFixture::new();
    let path = t.child.join("packages/ui/package.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    manifest["scripts"].as_object_mut().unwrap().remove("pack");
    write_json(&path, &manifest);
    failure(
        t.cli()
            .args(["task", "@example/protocol:build", "@example/ui:pack"])
            .output()
            .unwrap(),
        "no executable package script",
    );
    assert!(!t.marker("protocol", "prepare").exists());
    assert!(!t.marker("protocol", "build").exists());
    assert!(!t.marker("ui", "pack").exists());
}

#[test]
fn task_preserves_colons_in_task_names_and_deduplicates_exact_selectors() {
    let t = TurboFixture::new();
    let path = t.child.join("packages/protocol/package.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    manifest["scripts"]["pack:app"] = serde_json::json!("node ../../task.cjs pack-app");
    write_json(&path, &manifest);
    let path = t.child.join("turbo.json");
    let mut turbo: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    turbo["tasks"]["pack:app"] = serde_json::json!({"cache":false});
    write_json(&path, &turbo);
    let path = t.f.app.join(".sources.conf");
    let mut config: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    config["sources"]["shared"]["packages"]["@example/protocol"]["tasks"] =
        serde_json::json!(["build", "pack", "pack:app"]);
    write_json(&path, &config);
    success(
        t.cli()
            .args([
                "task",
                "@example/protocol:pack:app",
                "@example/protocol:pack:app",
            ])
            .output()
            .unwrap(),
    );
    assert_eq!(
        std::fs::read_to_string(t.marker("protocol", "pack-app")).unwrap(),
        "pack-app\n"
    );
    assert!(!t.marker("protocol", "pack").exists());
}

#[test]
fn task_runs_source_groups_in_stable_order_and_stops_at_first_child_failure() {
    let t = TurboFixture::new();
    let later = t.add_source("z-later", "@example/other");
    let path = t.child.join("packages/protocol/package.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    manifest["scripts"]["build"] =
        serde_json::json!("node ../../task.cjs build && node -e \"process.exit(23)\"");
    write_json(&path, &manifest);
    let output = t
        .cli()
        .args(["task", "@example/other:pack", "@example/protocol:build"])
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(23),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(t.marker("protocol", "prepare").is_file());
    assert!(t.marker("protocol", "build").is_file());
    assert!(!later.join("packages/other/dist/pack.txt").exists());
}

#[test]
fn task_preflights_later_sources_before_running_the_first_source() {
    let t = TurboFixture::new();
    let later = t.add_source("z-later", "@example/other");
    let path = later.join("packages/other/package.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    manifest["scripts"].as_object_mut().unwrap().remove("pack");
    write_json(&path, &manifest);
    failure(
        t.cli()
            .args(["task", "@example/protocol:build", "@example/other:pack"])
            .output()
            .unwrap(),
        "no executable package script",
    );
    assert!(!t.marker("protocol", "prepare").exists());
    assert!(!t.marker("protocol", "build").exists());
    assert!(!later.join("packages/other/dist/pack.txt").exists());
}

#[test]
fn task_requires_repository_local_turbo_even_when_an_inherited_turbo_is_available() {
    let t = TurboFixture::new();
    std::fs::remove_dir_all(t.child.join("node_modules/.bin")).unwrap();
    let binary = std::path::PathBuf::from(std::env::var_os("SOURCES_TEST_TURBO").unwrap());
    let external_bin = binary
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(".bin");
    let mut paths = vec![external_bin];
    paths.extend(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    ));
    failure(
        t.cli()
            .env("PATH", std::env::join_paths(paths).unwrap())
            .args(["task", "@example/protocol:build"])
            .output()
            .unwrap(),
        "locally installed Turbo",
    );
    assert!(!t.marker("protocol", "prepare").exists());
    assert!(!t.marker("protocol", "build").exists());
}

#[test]
fn task_rejects_unconfigured_selectors_and_missing_child_packages_before_execution() {
    let t = TurboFixture::new();
    for (selector, diagnostic) in [
        ("@example/ui:build", "unconfigured Turbo task"),
        ("@example/missing:pack", "unconfigured Turbo package"),
        ("shared:build", "unconfigured Turbo package"),
        ("@example/protocol", "package:task selector"),
    ] {
        failure(
            t.cli()
                .args(["task", "@example/protocol:build", selector])
                .output()
                .unwrap(),
            diagnostic,
        );
        assert!(!t.marker("protocol", "prepare").exists());
        assert!(!t.marker("protocol", "build").exists());
    }
    let path = t.child.join("packages/ui/package.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    manifest["name"] = serde_json::json!("@example/renamed-ui");
    write_json(&path, &manifest);
    failure(
        t.cli()
            .args(["task", "@example/protocol:build", "@example/ui:pack"])
            .output()
            .unwrap(),
        "Turbo metadata query failed",
    );
    assert!(!t.marker("protocol", "prepare").exists());
    assert!(!t.marker("protocol", "build").exists());
}
