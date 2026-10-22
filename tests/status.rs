#![cfg(unix)]

mod common;
use common::{Fixture, success};
use serde_json::json;

#[test]
fn status_derives_mode_from_links_and_reports_broken_targets_with_repair_commands() {
    let f = Fixture::new();
    let dev = f.repo("dev");
    f.git(
        &dev,
        &[
            "remote",
            "add",
            "origin",
            "https://example.invalid/team/repo",
        ],
    );
    f.configure(
        json!({"a":{"url":"https://example.invalid/team/repo"}}),
        json!({"a":dev}),
    );
    let out = success(f.cli().args(["status", "--json"]).output().unwrap());
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(value["version"], 2);
    assert_eq!(value["mode"], "unconfigured");
    success(f.cli().arg("dev").output().unwrap());
    std::fs::write(dev.join("tracked.txt"), "dirty\n").unwrap();
    let out = success(f.cli().args(["status", "--json"]).output().unwrap());
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(value["mode"], "dev");
    assert_eq!(value["sources"]["a"]["dirty"], true);
    assert_eq!(
        value["sources"]["a"]["active_commit"],
        f.git(&dev, &["rev-parse", "HEAD"])
    );
    std::fs::rename(&dev, f.root.join("moved-dev")).unwrap();
    let out = f.cli().args(["status", "--json"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(
        value["sources"]["a"]["errors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e.as_str().unwrap().contains("sources dev"))
    );
    std::fs::remove_file(f.app.join("sources/a")).unwrap();
    std::os::unix::fs::symlink(f.root.join("moved-dev"), f.app.join("sources/a")).unwrap();
    let out = f.cli().args(["status", "--json"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(value["mode"], "mixed");
}
