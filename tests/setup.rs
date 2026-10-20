mod common;
use common::{Fixture, success};

#[test]
fn setup_is_repeatable_preserves_configuration_and_adds_ignore_entries() {
    let fixture = Fixture::new();
    std::fs::write(fixture.app.join(".gitignore"), "existing-pattern").unwrap();
    success(fixture.cli().arg("setup").output().unwrap());
    let config = fixture.app.join(".sources.conf");
    let original = std::fs::read(&config).unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&original).unwrap()["version"],
        1
    );
    assert!(fixture.app.join(".sources.local.conf").is_file());
    assert!(fixture.app.join(".sources.lock").is_file());
    assert!(fixture.app.join(".sources").is_dir());
    assert!(!fixture.app.join("sources").exists());
    success(fixture.cli().arg("setup").output().unwrap());
    assert_eq!(std::fs::read(config).unwrap(), original);
    let ignore = std::fs::read_to_string(fixture.app.join(".gitignore")).unwrap();
    assert!(ignore.starts_with("existing-pattern\n"));
    for entry in ["/.sources/", "/.sources.local.conf", "/sources/"] {
        assert_eq!(ignore.lines().filter(|line| *line == entry).count(), 1);
    }
}

#[test]
fn setup_preserves_custom_configuration_and_normalizes_ignore_paths_with_c() {
    let fixture = Fixture::new();
    let bytes = br#"{"version":1,"directory":"./custom paths","sources":{}}"#;
    std::fs::write(fixture.app.join(".sources.conf"), bytes).unwrap();
    let elsewhere = fixture.root.join("elsewhere");
    std::fs::create_dir(&elsewhere).unwrap();
    success(
        fixture
            .cli()
            .current_dir(elsewhere)
            .arg("-C")
            .arg(&fixture.app)
            .arg("setup")
            .output()
            .unwrap(),
    );
    assert_eq!(
        std::fs::read(fixture.app.join(".sources.conf")).unwrap(),
        bytes
    );
    assert!(
        std::fs::read_to_string(fixture.app.join(".gitignore"))
            .unwrap()
            .contains("/custom\\ paths/\n")
    );
}

#[test]
fn setup_ignore_rules_match_normalized_workspace_directory_in_git() {
    let f = Fixture::new();
    f.git(&f.app, &["init"]);
    std::fs::create_dir(f.app.join("cache")).unwrap();
    std::fs::write(
        f.app.join(".sources.conf"),
        r#"{"version":1,"directory":"cache/../sources","sources":{}}"#,
    )
    .unwrap();
    success(f.cli().arg("setup").output().unwrap());
    f.git(&f.app, &["check-ignore", "sources/a"]);
}

#[cfg(unix)]
#[test]
fn setup_preserves_literal_backslashes_in_gitignore_paths() {
    let f = Fixture::new();
    f.git(&f.app, &["init"]);
    std::fs::write(
        f.app.join(".sources.conf"),
        r#"{"version":1,"directory":"folder\\name","sources":{}}"#,
    )
    .unwrap();
    success(f.cli().arg("setup").output().unwrap());
    f.git(&f.app, &["check-ignore", "folder\\name/a"]);
}

#[cfg(unix)]
#[test]
fn setup_refuses_to_write_through_a_gitignore_symlink() {
    let f = Fixture::new();
    let target = f.root.join("development-ignore");
    std::fs::write(&target, "keep this\n").unwrap();
    std::os::unix::fs::symlink(&target, f.app.join(".gitignore")).unwrap();
    let output = f.cli().arg("setup").output().unwrap();
    assert!(!output.status.success());
    assert_eq!(std::fs::read_to_string(target).unwrap(), "keep this\n");
}

#[test]
fn setup_uses_actual_filesystem_name_equivalence_for_source_collisions() {
    for (first, second) in [("Upper", "upper"), ("é", "e\u{301}")] {
        let f = Fixture::new();
        std::fs::write(f.root.join(first), "probe").unwrap();
        let collide = f.root.join(second).exists();
        f.configure(serde_json::json!({first:{"url":"https://example.invalid/a"},second:{"url":"https://example.invalid/b"}}), serde_json::json!({}));
        let output = f.cli().arg("setup").output().unwrap();
        assert_eq!(
            output.status.success(),
            !collide,
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn blank_upstream_urls_are_rejected_before_path_normalization() {
    for url in ["", "   "] {
        let f = Fixture::new();
        f.configure(serde_json::json!({"a":{"url":url}}), serde_json::json!({}));
        let out = f.cli().arg("setup").output().unwrap();
        assert_eq!(out.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&out.stderr).contains("upstream URL"));
    }
}
