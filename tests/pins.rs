mod common;
use common::{Fixture, failure, success};
use serde_json::json;

#[test]
fn lock_captures_development_heads_warns_dirty_and_validates_updates_before_writing() {
    let f = Fixture::new();
    let a = f.repo("dev-a");
    let b = f.repo("dev-b");
    f.git(
        &a,
        &["remote", "add", "origin", "https://example.invalid/team/a"],
    );
    f.git(
        &b,
        &["remote", "add", "origin", "https://example.invalid/team/b"],
    );
    f.configure(json!({"a":{"url":"https://example.invalid/team/a"},"b":{"url":"https://example.invalid/team/b"}}), json!({"a":a,"b":b}));
    let a_pin = f.git(&a, &["rev-parse", "HEAD"]);
    let b_pin = f.git(&b, &["rev-parse", "HEAD"]);
    std::fs::write(a.join("tracked.txt"), "excluded edit\n").unwrap();
    let out = success(f.cli().arg("lock").output().unwrap());
    assert!(String::from_utf8_lossy(&out.stderr).contains("uncommitted changes are excluded"));
    let lock_path = f.app.join(".sources.lock");
    let original = std::fs::read(&lock_path).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&original).unwrap();
    assert_eq!(value["sources"]["a"], a_pin);
    assert_eq!(value["sources"]["b"], b_pin);
    assert!(!f.app.join(".sources/checkouts").exists());
    assert!(!f.app.join("sources").exists());
    let abbreviated = format!("a:{}", &a_pin[..8]);
    success(f.cli().args(["lock", &abbreviated]).output().unwrap());
    assert_eq!(std::fs::read(&lock_path).unwrap(), original);
    failure(
        f.cli()
            .args(["lock", &abbreviated, &abbreviated])
            .output()
            .unwrap(),
        "duplicate",
    );
    assert_eq!(std::fs::read(&lock_path).unwrap(), original);
    failure(
        f.cli()
            .args(["lock", &abbreviated, "b:deadbeef"])
            .output()
            .unwrap(),
        "cannot resolve",
    );
    assert_eq!(std::fs::read(&lock_path).unwrap(), original);
    let blob = f.git(&a, &["rev-parse", "HEAD:tracked.txt"]);
    failure(
        f.cli()
            .args(["lock", &format!("a:{blob}")])
            .output()
            .unwrap(),
        "does not identify a commit",
    );
    assert_eq!(std::fs::read(&lock_path).unwrap(), original);
    assert_eq!(
        std::fs::read_to_string(a.join("tracked.txt")).unwrap(),
        "excluded edit\n"
    );
}

#[test]
fn check_lock_caches_remote_commit_without_development_mappings_or_activation() {
    let f = Fixture::new();
    let upstream = f.repo("upstream");
    let pin = f.git(&upstream, &["rev-parse", "HEAD"]);
    let url = url::Url::from_file_path(&upstream).unwrap().to_string();
    f.configure(json!({"a":{"url":url}}), json!({}));
    success(
        f.cli()
            .args(["lock", &format!("a:{pin}"), "--check"])
            .output()
            .unwrap(),
    );
    let value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(f.app.join(".sources.lock")).unwrap()).unwrap();
    assert_eq!(value["sources"]["a"], pin);
    assert!(!f.app.join("sources").exists());
    let cached = f.app.join(".sources/checkouts/a");
    f.git(&cached, &["cat-file", "-e", &format!("{pin}^{{commit}}")]);
    std::fs::rename(&upstream, f.root.join("upstream-offline")).unwrap();
    success(f.cli().arg("check-release").output().unwrap());
    assert!(!f.app.join("sources").exists());
    let previous = std::fs::read(f.app.join(".sources.lock")).unwrap();
    failure(
        f.cli()
            .args([
                "lock",
                "a:1111111111111111111111111111111111111111",
                "--check",
            ])
            .output()
            .unwrap(),
        "pin validation failed",
    );
    assert_eq!(
        std::fs::read(f.app.join(".sources.lock")).unwrap(),
        previous
    );
}

#[test]
fn abbreviated_pin_conflicting_between_development_and_managed_repositories_is_rejected() {
    use std::io::Write;
    use std::process::Stdio;
    let f = Fixture::new();
    let upstream = f.repo("upstream");
    let dev = f.repo("development");
    let url = url::Url::from_file_path(upstream).unwrap().to_string();
    f.git(&dev, &["remote", "add", "origin", &url]);
    f.configure(json!({"a":{"url":url}}), json!({"a":dev}));
    success(f.cli().args(["lock", "--check"]).output().unwrap());
    let managed = f.app.join(".sources/checkouts/a");
    let inject = |repo: &std::path::Path, index: u32| {
        let tree = f.git(repo, &["mktree"]);
        let contents = format!(
            "tree {tree}\nauthor Fixture <fixture@example.invalid> 1704067200 +0000\ncommitter Fixture <fixture@example.invalid> 1704067200 +0000\n\ncollision {index}\n"
        );
        let mut child = f
            .command("git")
            .current_dir(repo)
            .args(["hash-object", "-t", "commit", "-w", "--stdin"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(contents.as_bytes())
            .unwrap();
        String::from_utf8(child.wait_with_output().unwrap().stdout)
            .unwrap()
            .trim()
            .to_owned()
    };
    let managed_oid = inject(&managed, 24);
    let development_oid = inject(&dev, 306);
    assert_ne!(managed_oid, development_oid);
    assert_eq!(&managed_oid[..4], &development_oid[..4]);
    let before = std::fs::read(f.app.join(".sources.lock")).unwrap();
    failure(
        f.cli()
            .args(["lock", &format!("a:{}", &managed_oid[..4])])
            .output()
            .unwrap(),
        "ambiguous",
    );
    assert_eq!(std::fs::read(f.app.join(".sources.lock")).unwrap(), before);
}

#[test]
fn a_hexadecimal_branch_name_does_not_override_an_abbreviated_object_id() {
    let f = Fixture::new();
    let dev = f.repo("development");
    let wanted = f.git(&dev, &["rev-parse", "HEAD"]);
    std::fs::write(dev.join("tracked.txt"), "later commit\n").unwrap();
    f.git(&dev, &["commit", "-qam", "later"]);
    f.git(&dev, &["branch", &wanted[..8]]);
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
        json!({"a": {"url": "https://example.invalid/team/repo"}}),
        json!({"a": dev}),
    );
    success(
        f.cli()
            .args(["lock", &format!("a:{}", &wanted[..8])])
            .output()
            .unwrap(),
    );
    let pins: serde_json::Value =
        serde_json::from_slice(&std::fs::read(f.app.join(".sources.lock")).unwrap()).unwrap();
    assert_eq!(pins["sources"]["a"], wanted);
}

#[test]
fn a_commit_and_blob_prefix_collision_across_local_repositories_is_ambiguous() {
    use std::io::Write;
    use std::process::Stdio;
    let f = Fixture::new();
    let upstream = f.repo("upstream");
    let dev = f.repo("development");
    let url = url::Url::from_file_path(upstream).unwrap().to_string();
    f.git(&dev, &["remote", "add", "origin", &url]);
    f.configure(json!({"a": {"url": url}}), json!({"a": dev}));
    success(f.cli().args(["lock", "--check"]).output().unwrap());
    let managed = f.app.join(".sources/checkouts/a");
    let inject = |repo: &std::path::Path, kind: &str, bytes: &[u8]| {
        let mut child = f
            .command("git")
            .current_dir(repo)
            .args(["hash-object", "-t", kind, "-w", "--stdin"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(bytes).unwrap();
        String::from_utf8(child.wait_with_output().unwrap().stdout)
            .unwrap()
            .trim()
            .to_owned()
    };
    let tree = f.git(&managed, &["mktree"]);
    let commit = inject(&managed, "commit", format!("tree {tree}\nauthor Fixture <fixture@example.invalid> 1704067200 +0000\ncommitter Fixture <fixture@example.invalid> 1704067200 +0000\n\ncollision 24\n").as_bytes());
    let blob = inject(&dev, "blob", b"blob collision 83341\n");
    assert_eq!(&commit[..4], "f9fd");
    assert_eq!(blob, "f9fd5d8537ffc39d581588df077222dc771739a3");
    let lock = f.app.join(".sources.lock");
    let before = std::fs::read(&lock).unwrap();
    failure(
        f.cli().args(["lock", "a:f9fd"]).output().unwrap(),
        "ambiguous",
    );
    assert_eq!(std::fs::read(&lock).unwrap(), before);
}
