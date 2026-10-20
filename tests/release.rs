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
    assert_eq!(workspace.canonicalize().unwrap(), dev_a);
    success(f.cli().args(["lock", "--check"]).output().unwrap());
    let locked = std::fs::read(f.app.join(".sources.lock")).unwrap();
    let dev_a_head = f.git(&dev_a, &["rev-parse", "HEAD"]);
    let dev_b_head = f.git(&dev_b, &["rev-parse", "HEAD"]);
    std::fs::write(dev_a.join("tracked.txt"), "development edit\n").unwrap();
    success(f.cli().arg("release").output().unwrap());
    assert_eq!(
        workspace.canonicalize().unwrap(),
        f.app.join(".sources/checkouts/a")
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
    assert_eq!(workspace.canonicalize().unwrap(), dev_a);
    assert_eq!(
        std::fs::read_to_string(dev_a.join("tracked.txt")).unwrap(),
        "development edit\n"
    );
    assert_eq!(f.git(&dev_a, &["rev-parse", "HEAD"]), dev_a_head);
    assert_eq!(f.git(&dev_b, &["rev-parse", "HEAD"]), dev_b_head);
    assert_eq!(std::fs::read(f.app.join(".sources.lock")).unwrap(), locked);
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

#[cfg(unix)]
#[test]
fn link_collisions_are_preflighted_and_stale_links_are_unlinked_without_following_them() {
    let (f, _, _) = project();
    std::fs::create_dir_all(f.app.join("sources/b")).unwrap();
    std::fs::write(f.app.join("sources/b/keep.txt"), "keep\n").unwrap();
    failure(
        f.cli().arg("dev").output().unwrap(),
        "ordinary file or directory",
    );
    assert!(!f.app.join("sources/a").exists());
    assert!(f.app.join("sources/b/keep.txt").exists());
    std::fs::rename(f.app.join("sources/b"), f.root.join("keep-b")).unwrap();
    let external = f.root.join("external");
    std::fs::create_dir(&external).unwrap();
    std::fs::write(external.join("keep.txt"), "keep\n").unwrap();
    std::os::unix::fs::symlink(&external, f.app.join("sources/obsolete")).unwrap();
    success(f.cli().arg("dev").output().unwrap());
    assert!(std::fs::symlink_metadata(f.app.join("sources/obsolete")).is_err());
    assert!(external.join("keep.txt").exists());
    assert!(f.root.join("keep-b/keep.txt").exists());
}
