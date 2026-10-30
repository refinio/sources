mod common;
use common::{Fixture, failure, success};
use serde_json::json;
use std::path::PathBuf;

fn project() -> (Fixture, PathBuf, PathBuf) {
    let f = Fixture::new();
    let a = f.repo("upstream-a");
    let b = f.repo("upstream-b");
    let dev_a = f.root.join("dev-a");
    let dev_b = f.root.join("dev-b");
    let url_a = url::Url::from_file_path(a).unwrap().to_string();
    let url_b = url::Url::from_file_path(b).unwrap().to_string();
    f.git(&f.root, &["clone", &url_a, dev_a.to_str().unwrap()]);
    f.git(&f.root, &["clone", &url_b, dev_b.to_str().unwrap()]);
    f.configure(
        json!({"a":{"url":url_a},"b":{"url":url_b}}),
        json!({"a":dev_a,"b":dev_b}),
    );
    (f, dev_a, dev_b)
}

#[test]
fn development_and_release_switch_stable_paths_without_changing_pins_or_development_checkouts() {
    let (f, dev_a, dev_b) = project();
    success(f.cli().arg("setup").output().unwrap());
    success(f.cli().arg("dev").output().unwrap());
    let workspace = f.app.join("sources/a");
    assert_eq!(
        workspace.canonicalize().unwrap(),
        dev_a.canonicalize().unwrap()
    );
    success(f.cli().args(["lock", "--check"]).output().unwrap());
    let locked = std::fs::read(f.app.join(".sources.lock")).unwrap();
    let dev_a_head = f.git(&dev_a, &["rev-parse", "HEAD"]);
    let dev_b_head = f.git(&dev_b, &["rev-parse", "HEAD"]);
    std::fs::write(dev_a.join("tracked.txt"), "development edit\n").unwrap();
    success(f.cli().arg("release").output().unwrap());
    assert_eq!(
        workspace.canonicalize().unwrap(),
        f.app.join(".sources/checkouts/a").canonicalize().unwrap()
    );
    assert_eq!(
        std::fs::read_to_string(workspace.join("tracked.txt")).unwrap(),
        "initial\n"
    );
    std::fs::write(workspace.join("artifact.txt"), "retain generated output\n").unwrap();
    success(f.cli().arg("release").output().unwrap());
    assert_eq!(
        std::fs::read_to_string(workspace.join("artifact.txt")).unwrap(),
        "retain generated output\n"
    );
    success(f.cli().arg("dev").output().unwrap());
    assert_eq!(
        workspace.canonicalize().unwrap(),
        dev_a.canonicalize().unwrap()
    );
    assert_eq!(
        std::fs::read_to_string(dev_a.join("tracked.txt")).unwrap(),
        "development edit\n"
    );
    assert_eq!(f.git(&dev_a, &["rev-parse", "HEAD"]), dev_a_head);
    assert_eq!(f.git(&dev_b, &["rev-parse", "HEAD"]), dev_b_head);
    assert_eq!(std::fs::read(f.app.join(".sources.lock")).unwrap(), locked);

    // A committed diamond must select one revision of a shared repository,
    // even when its source aliases and URL spellings differ.
    let shared = f.repo("shared");
    let shared_module = f.repo("shared-module");
    f.git(
        &shared,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            shared_module.to_str().unwrap(),
            "module",
        ],
    );
    f.git(&shared, &["commit", "-qam", "add shared module"]);
    let old = f.git(&shared, &["rev-parse", "HEAD"]);
    std::fs::write(shared.join("tracked.txt"), "new shared revision\n").unwrap();
    f.git(&shared, &["commit", "-qam", "advance shared"]);
    let new = f.git(&shared, &["rev-parse", "HEAD"]);
    let shared_url = url::Url::from_file_path(&shared).unwrap().to_string();
    for (repo, alias, url, commit) in [
        ("upstream-a", "shared-old", shared_url, old.clone()),
        (
            "upstream-b",
            "shared-new",
            shared.to_str().unwrap().to_owned(),
            new.clone(),
        ),
    ] {
        let upstream = f.root.join(repo);
        std::fs::write(
            upstream.join(".sources.conf"),
            json!({
                "version": 1, "directory": "sources", "sources": {alias: {"url": url}}
            })
            .to_string(),
        )
        .unwrap();
        std::fs::write(
            upstream.join(".sources.lock"),
            json!({
                "version": 2, "sources": {alias: {"git": commit}}
            })
            .to_string(),
        )
        .unwrap();
        f.git(&upstream, &["add", "."]);
        f.git(&upstream, &["commit", "-qm", "pin shared source"]);
    }
    let a = f.git(&f.root.join("upstream-a"), &["rev-parse", "HEAD"]);
    let b = f.git(&f.root.join("upstream-b"), &["rev-parse", "HEAD"]);
    success(
        f.cli()
            .args(["lock", &format!("a:{a}"), &format!("b:{b}"), "--check"])
            .output()
            .unwrap(),
    );
    let pins = std::fs::read(f.app.join(".sources.lock")).unwrap();
    let managed_a = f.app.join(".sources/checkouts/a");
    let before_head = f.git(&managed_a, &["rev-parse", "HEAD"]);
    for command in ["check-release", "release"] {
        let output = f.cli().arg(command).output().unwrap();
        let diagnostic = String::from_utf8_lossy(&output.stderr).to_string();
        failure(output, "conflicting pinned versions");
        for detail in [&old, &new, "a -> shared-old", "b -> shared-new"] {
            assert!(diagnostic.contains(detail), "{diagnostic}");
        }
        assert_eq!(f.git(&managed_a, &["rev-parse", "HEAD"]), before_head);
        assert_eq!(
            workspace.canonicalize().unwrap(),
            dev_a.canonicalize().unwrap()
        );
        assert_eq!(std::fs::read(f.app.join(".sources.lock")).unwrap(), pins);
    }
    // Align the child's committed pin, then validate and activate without
    // changing development checkouts or recursively activating child projects.
    let upstream_b = f.root.join("upstream-b");
    std::fs::write(
        upstream_b.join(".sources.lock"),
        json!({
            "version": 2, "sources": {"shared-new": {"git": old}}
        })
        .to_string(),
    )
    .unwrap();
    f.git(&upstream_b, &["commit", "-qam", "align shared source"]);
    let b = f.git(&upstream_b, &["rev-parse", "HEAD"]);
    success(
        f.cli()
            .args(["lock", &format!("b:{b}"), "--check"])
            .output()
            .unwrap(),
    );
    success(f.cli().arg("check-release").output().unwrap());
    success(f.cli().args(["release", "--offline"]).output().unwrap());
    assert!(!managed_a.join("sources").exists());
    assert_eq!(f.git(&dev_a, &["rev-parse", "HEAD"]), dev_a_head);
    assert_eq!(f.git(&dev_b, &["rev-parse", "HEAD"]), dev_b_head);
    // The application itself also cannot be reached at an older revision.
    f.git(&f.app, &["init"]);
    f.git(
        &f.app,
        &["remote", "add", "origin", shared.to_str().unwrap()],
    );
    f.git(&f.app, &["add", "."]);
    f.git(&f.app, &["commit", "-qm", "application release"]);
    let app_commit = f.git(&f.app, &["rev-parse", "HEAD"]);
    let output = f.cli().arg("check-release").output().unwrap();
    let diagnostic = String::from_utf8_lossy(&output.stderr).to_string();
    failure(output, "conflicting pinned versions");
    assert!(
        diagnostic.contains(&format!("root: {app_commit}")),
        "{diagnostic}"
    );
}

#[test]
fn clean_recreation_discards_all_managed_edits_and_artifacts_but_preserves_external_targets() {
    let (f, dev_a, _) = project();
    success(f.cli().arg("lock").output().unwrap());
    success(f.cli().arg("release").output().unwrap());
    let managed = f.app.join(".sources/checkouts/a");
    std::fs::write(managed.join("tracked.txt"), "discard managed edit\n").unwrap();
    std::fs::write(managed.join("untracked.txt"), "discard untracked\n").unwrap();
    std::fs::write(managed.join("artifact.txt"), "discard ignored\n").unwrap();
    std::fs::write(dev_a.join("tracked.txt"), "preserve development edit\n").unwrap();
    let external = f.root.join("external");
    std::fs::create_dir(&external).unwrap();
    std::fs::write(external.join("valuable.txt"), "keep\n").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&external, managed.join("external-link")).unwrap();
    failure(f.cli().arg("release").output().unwrap(), "source changes");
    success(f.cli().args(["release", "--clean"]).output().unwrap());
    assert_eq!(
        std::fs::read_to_string(managed.join("tracked.txt")).unwrap(),
        "initial\n"
    );
    assert!(!managed.join("untracked.txt").exists());
    assert!(!managed.join("artifact.txt").exists());
    assert_eq!(
        std::fs::read_to_string(external.join("valuable.txt")).unwrap(),
        "keep\n"
    );
    assert_eq!(
        std::fs::read_to_string(dev_a.join("tracked.txt")).unwrap(),
        "preserve development edit\n"
    );
}

#[test]
fn offline_activation_uses_caches_and_failed_clean_recreation_is_repaired_by_rerunning() {
    let (f, _, _) = project();
    success(f.cli().arg("dev").output().unwrap());
    success(f.cli().arg("lock").output().unwrap());
    let before_target = std::fs::read_link(f.app.join("sources/a")).unwrap();
    failure(
        f.cli().args(["release", "--offline"]).output().unwrap(),
        "missing managed cache",
    );
    assert_eq!(
        std::fs::read_link(f.app.join("sources/a")).unwrap(),
        before_target
    );
    success(f.cli().arg("release").output().unwrap());
    let pins = std::fs::read(f.app.join(".sources.lock")).unwrap();
    std::fs::rename(f.root.join("upstream-a"), f.root.join("upstream-a-away")).unwrap();
    std::fs::rename(f.root.join("upstream-b"), f.root.join("upstream-b-away")).unwrap();
    success(f.cli().arg("dev").output().unwrap());
    let offline = success(f.cli().args(["release", "--offline"]).output().unwrap());
    assert!(String::from_utf8_lossy(&offline.stdout).contains("without network access"));
    failure(
        f.cli().args(["release", "--clean"]).output().unwrap(),
        "release preparation failed",
    );
    let broken = f.cli().args(["status", "--json"]).output().unwrap();
    assert_eq!(broken.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&broken.stderr).contains("sources release"));
    success(f.cli().arg("dev").output().unwrap());
    std::fs::rename(f.root.join("upstream-a-away"), f.root.join("upstream-a")).unwrap();
    std::fs::rename(f.root.join("upstream-b-away"), f.root.join("upstream-b")).unwrap();
    success(f.cli().args(["release", "--clean"]).output().unwrap());
    success(f.cli().args(["status", "--json"]).output().unwrap());
    assert_eq!(std::fs::read(f.app.join(".sources.lock")).unwrap(), pins);
}
