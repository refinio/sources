mod common;
use common::{Fixture, success};
use serde_json::json;

#[cfg(unix)]
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
    assert_eq!(value["version"], 1);
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

#[test]
fn status_reports_incomplete_recursive_cache_after_failed_fetch_without_activating_it() {
    let f = Fixture::new();
    let _nested = f.repo("nested");
    let module = f.repo("module");
    f.git(
        &module,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            "../nested",
            "nested",
        ],
    );
    f.git(
        &module,
        &[
            "config",
            "--file",
            ".gitmodules",
            "submodule.nested.url",
            "../unavailable-nested",
        ],
    );
    f.git(&module, &["add", "."]);
    f.git(&module, &["commit", "-qm", "recursive input unavailable"]);
    let parent = f.repo("upstream");
    f.git(
        &parent,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            "../module",
            "dep",
        ],
    );
    f.git(&parent, &["commit", "-qam", "parent input"]);
    let pin = f.git(&parent, &["rev-parse", "HEAD"]);
    f.configure(json!({"a": {"url": parent}}), json!({}));
    std::fs::write(
        f.app.join(".sources.lock"),
        json!({"version": 1, "sources": {"a": pin}}).to_string(),
    )
    .unwrap();
    common::failure(f.cli().arg("fetch").output().unwrap(), "submodule");
    let managed = f.app.join(".sources/checkouts/a");
    assert_eq!(f.git(&managed, &["cat-file", "-t", &pin]), "commit");
    let head = std::fs::read(managed.join(".git/HEAD")).unwrap();
    let refs = f.git(&managed, &["for-each-ref"]);
    let lock = std::fs::read(f.app.join(".sources.lock")).unwrap();
    let output = f.cli().args(["status", "--json"]).output().unwrap();
    let status: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(status["sources"]["a"]["cached_commit_available"], false);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        status["sources"]["a"]["errors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|error| error.as_str().unwrap().contains("sources fetch"))
    );
    assert_eq!(std::fs::read(managed.join(".git/HEAD")).unwrap(), head);
    assert_eq!(f.git(&managed, &["for-each-ref"]), refs);
    assert!(!managed.join("tracked.txt").exists());
    assert!(!managed.join("dep").exists());
    assert!(!f.app.join("sources").exists());
    assert_eq!(std::fs::read(f.app.join(".sources.lock")).unwrap(), lock);

    std::fs::rename(f.root.join("nested"), f.root.join("unavailable-nested")).unwrap();
    success(f.cli().arg("fetch").output().unwrap());
    let output = success(f.cli().args(["status", "--json"]).output().unwrap());
    let status: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(status["sources"]["a"]["cached_commit_available"], true);
    assert_eq!(std::fs::read(managed.join(".git/HEAD")).unwrap(), head);
    assert!(!managed.join("tracked.txt").exists());
    assert!(!f.app.join("sources").exists());
}
