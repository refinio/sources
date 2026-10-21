mod common;
use common::{Fixture, failure};
use serde_json::json;

#[test]
fn published_release_needs_pnpm_configuration_instead_of_a_git_commit_pin() {
    let f = Fixture::new();
    f.configure(
        json!({"library": {
            "url": "https://example.invalid/team/library",
            "pnpm": {"@example/library": {"path": ".", "version": "1.2.3"}}
        }}),
        json!({}),
    );
    failure(f.cli().arg("release").output().unwrap(), "package.json");
    assert!(!f.app.join(".sources/checkouts").exists());
    assert!(!f.app.join(".sources.lock").exists());
}

#[cfg(unix)]
#[test]
fn pnpm_rejects_unix_backslash_paths_before_creating_links() {
    use common::success;
    let f = Fixture::new();
    let dev = f.repo("dev");
    let upstream = "https://example.invalid/team/library";
    f.git(&dev, &["remote", "add", "origin", upstream]);
    std::fs::write(
        dev.join("package.json"),
        r#"{"name":"@example/library","version":"1.2.3"}"#,
    )
    .unwrap();
    f.configure(
        json!({"library": {"url": upstream, "pnpm": {
            "@example/library": {"path": ".", "version": "1.2.3"}
        }}}),
        json!({"library": dev}),
    );
    let config_path = f.app.join(".sources.conf");
    let mut config: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&config_path).unwrap()).unwrap();
    config["directory"] = json!("links\\literal");
    std::fs::write(config_path, serde_json::to_vec_pretty(&config).unwrap()).unwrap();
    std::fs::write(
        f.app.join("package.json"),
        r#"{"name":"consumer","private":true,"dependencies":{"@example/library":"1.2.3"}}"#,
    )
    .unwrap();
    std::fs::write(f.app.join("pnpm-workspace.yaml"), "packages: []\n").unwrap();
    failure(
        f.cli().env("CI", "true").arg("dev").output().unwrap(),
        "pnpm local paths cannot contain backslashes on Unix",
    );
    assert!(!f.app.join("links\\literal").exists());
    assert_eq!(
        std::fs::read_to_string(f.app.join("pnpm-workspace.yaml")).unwrap(),
        "packages: []\n"
    );
    // The restriction is specific to pnpm; the same Git layout stays supported.
    let mut config: serde_json::Value =
        serde_json::from_slice(&std::fs::read(f.app.join(".sources.conf")).unwrap()).unwrap();
    config["sources"]["library"]
        .as_object_mut()
        .unwrap()
        .remove("pnpm");
    std::fs::write(
        f.app.join(".sources.conf"),
        serde_json::to_vec_pretty(&config).unwrap(),
    )
    .unwrap();
    success(f.cli().arg("dev").output().unwrap());
    assert!(f.app.join("links\\literal/library").is_dir());
}
