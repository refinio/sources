mod common;
use common::{Fixture, failure};
use serde_json::json;

#[test]
fn configuration_rejects_source_names_and_development_paths_that_overlap_managed_state() {
    let f = Fixture::new();
    f.configure(
        json!({"../outside": {"url": "https://example.invalid/team/repo"}}),
        json!({}),
    );
    failure(f.cli().arg("check").output().unwrap(), "invalid source");
    f.configure(
        json!({"a": {"url": "https://example.invalid/team/repo"}}),
        json!({"a": ".sources/checkouts/a"}),
    );
    failure(f.cli().arg("check").output().unwrap(), "overlaps managed");
    let value = json!({"version":1,"directory":".sources/links","sources":{}});
    std::fs::write(f.app.join(".sources.conf"), value.to_string()).unwrap();
    std::fs::remove_file(f.app.join(".sources.local.conf")).unwrap();
    failure(f.cli().arg("check").output().unwrap(), "overlaps .sources");
}

#[cfg(unix)]
#[test]
fn setup_refuses_internal_directory_alias_without_touching_its_target() {
    let f = Fixture::new();
    let dev = f.repo("development");
    std::os::unix::fs::symlink(&dev, f.app.join(".sources")).unwrap();
    failure(
        f.cli().arg("setup").output().unwrap(),
        "must be a real directory",
    );
    assert!(dev.join("tracked.txt").is_file());
    assert!(!dev.join("project.lock").exists());
}

#[cfg(unix)]
#[test]
fn path_overlap_checks_resolve_symlinks_before_parent_components() {
    let f = Fixture::new();
    let dev = f.repo("external/managed");
    std::fs::create_dir_all(f.root.join("external/container")).unwrap();
    std::os::unix::fs::symlink(f.root.join("external/container"), f.app.join("alias")).unwrap();
    f.git(
        &dev,
        &[
            "remote",
            "add",
            "origin",
            "https://example.invalid/team/repo",
        ],
    );
    let config = json!({"version":1,"directory":dev,"sources":{"a":{"url":"https://example.invalid/team/repo"}}});
    std::fs::write(f.app.join(".sources.conf"), config.to_string()).unwrap();
    std::fs::write(
        f.app.join(".sources.local.conf"),
        json!({"sources":{"a":"alias/../managed"}}).to_string(),
    )
    .unwrap();
    failure(f.cli().arg("check").output().unwrap(), "overlaps managed");
}
