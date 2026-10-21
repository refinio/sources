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

#[test]
fn explicit_local_tags_peel_to_commit_pins_without_fetching_or_moving_checkouts() {
    let f = Fixture::new();
    let dev = f.repo("development");
    let first = f.git(&dev, &["rev-parse", "HEAD"]);
    f.git(&dev, &["tag", "v1.2.3"]);
    std::fs::write(dev.join("tracked.txt"), "next version\n").unwrap();
    f.git(&dev, &["commit", "-qam", "next"]);
    let second = f.git(&dev, &["rev-parse", "HEAD"]);
    f.git(&dev, &["tag", "-a", "deadbeef", "-m", "annotated release"]);
    f.git(&dev, &["branch", "v1.2.3"]);
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
    let before = f.git(&dev, &["status", "--porcelain=v1"]);
    for (tag, expected) in [("v1.2.3", first), ("deadbeef", second.clone())] {
        success(
            f.cli()
                .args(["lock", &format!("a:tag={tag}")])
                .output()
                .unwrap(),
        );
        let lock: serde_json::Value =
            serde_json::from_slice(&std::fs::read(f.app.join(".sources.lock")).unwrap()).unwrap();
        assert_eq!(lock["sources"]["a"], expected);
    }
    assert_eq!(f.git(&dev, &["rev-parse", "HEAD"]), second);
    assert_eq!(f.git(&dev, &["status", "--porcelain=v1"]), before);
    assert!(!f.app.join(".sources/checkouts").exists());
    assert!(!f.app.join("sources").exists());
}

#[test]
fn checked_tag_pins_use_exact_upstream_tags_without_moving_or_fetching_into_development() {
    let f = Fixture::new();
    let upstream = f.repo("upstream");
    let first = f.git(&upstream, &["rev-parse", "HEAD"]);
    f.git(&upstream, &["tag", "v1.2.3"]);
    let module = f.repo("module");
    let module_pin = f.git(&module, &["rev-parse", "HEAD"]);
    f.git(
        &upstream,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            "../module",
            "dep",
        ],
    );
    std::fs::write(upstream.join("tracked.txt"), "second\n").unwrap();
    f.git(&upstream, &["commit", "-qam", "second"]);
    let second = f.git(&upstream, &["rev-parse", "HEAD"]);
    f.git(&upstream, &["tag", "-a", "v2.0.0", "-m", "release two"]);
    std::fs::write(upstream.join("tracked.txt"), "unrelated tip\n").unwrap();
    f.git(&upstream, &["commit", "-qam", "unrelated"]);
    let unrelated = f.git(&upstream, &["rev-parse", "HEAD"]);
    f.git(&upstream, &["tag", "unrelated-tag"]);
    let server = f.server("upstream");
    f.git(&f.root, &["clone", &server.url, "development"]);
    let dev = f.root.join("development");
    f.git(&dev, &["tag", "-f", "v1.2.3", &unrelated]);
    f.git(&dev, &["tag", "-f", "v2.0.0", &unrelated]);
    std::fs::write(dev.join("tracked.txt"), "preserve development edits\n").unwrap();
    let dev_refs = f.git(&dev, &["show-ref"]);
    let dev_status = f.git(&dev, &["status", "--porcelain=v1"]);
    assert!(!dev.join(".git/FETCH_HEAD").exists());
    f.configure(json!({"a":{"url":server.url}}), json!({}));
    let cached = f.app.join(".sources/checkouts/a");
    let mut activated = None;
    for (tag, expected) in [("v1.2.3", first), ("v2.0.0", second)] {
        success(
            f.cli()
                .args(["lock", &format!("a:tag={tag}"), "--check"])
                .output()
                .unwrap(),
        );
        let lock: serde_json::Value =
            serde_json::from_slice(&std::fs::read(f.app.join(".sources.lock")).unwrap()).unwrap();
        assert_eq!(lock["sources"]["a"], expected);
        assert_eq!(f.git(&cached, &["rev-list", "--count", &expected]), "1");
        assert_eq!(f.git(&cached, &["tag", "--list"]), "");
        if let Some(head) = &activated {
            assert_eq!(&f.git(&cached, &["rev-parse", "HEAD"]), head);
        } else {
            assert!(
                !f.command("git")
                    .current_dir(&cached)
                    .args(["rev-parse", "--verify", "HEAD"])
                    .output()
                    .unwrap()
                    .status
                    .success()
            );
            f.git(&cached, &["checkout", "--detach", &expected]);
            activated = Some(expected.clone());
        }
        f.configure(json!({"a":{"url":server.url}}), json!({"a":dev}));
    }
    assert!(
        !f.command("git")
            .current_dir(&cached)
            .args(["cat-file", "-e", &unrelated])
            .output()
            .unwrap()
            .status
            .success()
    );
    success(
        f.command("git")
            .current_dir(cached.join(".git/modules/dep"))
            .args([
                "--git-dir",
                ".",
                "--work-tree",
                ".",
                "cat-file",
                "-e",
                &module_pin,
            ])
            .output()
            .unwrap(),
    );
    assert!(!dev.join(".git/FETCH_HEAD").exists());
    assert_eq!(f.git(&dev, &["rev-parse", "HEAD"]), unrelated);
    assert_eq!(f.git(&dev, &["show-ref"]), dev_refs);
    assert_eq!(f.git(&dev, &["status", "--porcelain=v1"]), dev_status);
    assert_eq!(
        std::fs::read_to_string(dev.join("tracked.txt")).unwrap(),
        "preserve development edits\n"
    );
    assert!(!f.app.join("sources").exists());
}

#[test]
fn upstream_tag_selection_overrides_local_tags_and_cached_local_conflicts_are_explicit() {
    let f = Fixture::new();
    let upstream = f.repo("upstream");
    f.git(&upstream, &["tag", "release"]);
    let url = url::Url::from_file_path(&upstream).unwrap().to_string();
    f.git(&f.root, &["clone", &url, "development"]);
    let dev = f.root.join("development");
    f.configure(json!({"a":{"url":url}}), json!({"a":dev}));
    success(
        f.cli()
            .args(["lock", "a:tag=release", "--check"])
            .output()
            .unwrap(),
    );
    std::fs::write(upstream.join("tracked.txt"), "moved release\n").unwrap();
    f.git(&upstream, &["commit", "-qam", "new release"]);
    let moved = f.git(&upstream, &["rev-parse", "HEAD"]);
    f.git(&upstream, &["tag", "-f", "release"]);
    success(
        f.cli()
            .args(["lock", "a:tag=release", "--check"])
            .output()
            .unwrap(),
    );
    let path = f.app.join(".sources.lock");
    let before = std::fs::read(&path).unwrap();
    let lock: serde_json::Value = serde_json::from_slice(&before).unwrap();
    assert_eq!(lock["sources"]["a"], moved);
    failure(
        f.cli().args(["lock", "a:tag=release"]).output().unwrap(),
        "ambiguous local tag",
    );
    assert_eq!(std::fs::read(&path).unwrap(), before);
    std::fs::rename(&upstream, f.root.join("upstream-offline")).unwrap();
    f.configure(json!({"a":{"url":url}}), json!({}));
    success(f.cli().args(["lock", "a:tag=release"]).output().unwrap());
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

#[test]
fn invalid_or_noncommit_tags_leave_all_previous_pins_unchanged() {
    let f = Fixture::new();
    let a = f.repo("development-a");
    let b = f.repo("development-b");
    let url = "https://example.invalid/team/repo";
    for repo in [&a, &b] {
        f.git(repo, &["remote", "add", "origin", url]);
    }
    f.configure(
        json!({"a":{"url":url},"b":{"url":url}}),
        json!({"a":a,"b":b}),
    );
    success(f.cli().arg("lock").output().unwrap());
    let path = f.app.join(".sources.lock");
    let before = std::fs::read(&path).unwrap();
    std::fs::write(a.join("tracked.txt"), "new pin\n").unwrap();
    f.git(&a, &["commit", "-qam", "new"]);
    f.git(&a, &["tag", "good"]);
    let blob = f.git(&b, &["rev-parse", "HEAD:tracked.txt"]);
    f.git(&b, &["tag", "blob", &blob]);
    f.git(
        &b,
        &["tag", "-a", "annotated-blob", "-m", "not a commit", &blob],
    );
    for tag in ["blob", "annotated-blob"] {
        failure(
            f.cli()
                .args(["lock", "a:tag=good", &format!("b:tag={tag}")])
                .output()
                .unwrap(),
            "does not identify a commit",
        );
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }
    for tag in ["", "good^{commit}", "../good"] {
        failure(
            f.cli()
                .args(["lock", &format!("a:tag={tag}")])
                .output()
                .unwrap(),
            "invalid tag name",
        );
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }
    failure(
        f.cli().args(["lock", "a:tag=main"]).output().unwrap(),
        "cannot resolve tag",
    );
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert!(!f.app.join(".sources/checkouts").exists());
}

#[test]
fn checked_tags_can_replace_a_deleted_tag_with_a_nested_tag_name() {
    let f = Fixture::new();
    let upstream = f.repo("upstream");
    f.git(&upstream, &["tag", "release"]);
    let url = url::Url::from_file_path(&upstream).unwrap().to_string();
    f.configure(json!({"a":{"url":url}}), json!({}));
    success(
        f.cli()
            .args(["lock", "a:tag=release", "--check"])
            .output()
            .unwrap(),
    );
    f.git(&upstream, &["tag", "-d", "release"]);
    f.git(&upstream, &["tag", "release/v2"]);
    success(
        f.cli()
            .args(["lock", "a:tag=release/v2", "--check"])
            .output()
            .unwrap(),
    );
    std::fs::rename(&upstream, f.root.join("upstream-offline")).unwrap();
    success(f.cli().args(["lock", "a:tag=release/v2"]).output().unwrap());
}

#[test]
fn a_branch_named_like_a_tag_reference_cannot_supply_a_missing_tag_pin() {
    let f = Fixture::new();
    let dev = f.repo("development");
    let url = "https://example.invalid/team/repo";
    f.git(&dev, &["remote", "add", "origin", url]);
    f.configure(json!({"a":{"url":url}}), json!({"a":dev}));
    success(f.cli().arg("lock").output().unwrap());
    let path = f.app.join(".sources.lock");
    let before = std::fs::read(&path).unwrap();
    std::fs::write(dev.join("tracked.txt"), "branch only\n").unwrap();
    f.git(&dev, &["commit", "-qam", "branch only"]);
    f.git(&dev, &["branch", "refs/tags/release"]);
    failure(
        f.cli().args(["lock", "a:tag=release"]).output().unwrap(),
        "cannot resolve tag",
    );
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

#[test]
fn a_long_upstream_tag_can_be_cached_and_selected_again_without_network() {
    let f = Fixture::new();
    let upstream = f.repo("upstream");
    let tag = "v".repeat(130);
    let pin = f.git(&upstream, &["rev-parse", "HEAD"]);
    f.git(&upstream, &["tag", "-a", &tag, "-m", "long release tag"]);
    let url = url::Url::from_file_path(&upstream).unwrap().to_string();
    f.configure(json!({"a":{"url":url}}), json!({}));
    let selector = format!("a:tag={tag}");
    success(
        f.cli()
            .args(["lock", &selector, "--check"])
            .output()
            .unwrap(),
    );
    let path = f.app.join(".sources.lock");
    let before = std::fs::read(&path).unwrap();
    let lock: serde_json::Value = serde_json::from_slice(&before).unwrap();
    assert_eq!(lock["sources"]["a"], pin);
    std::fs::rename(&upstream, f.root.join("upstream-offline")).unwrap();
    success(f.cli().args(["lock", &selector]).output().unwrap());
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

#[test]
fn a_local_development_tag_can_be_pinned_despite_a_broken_managed_cache() {
    let f = Fixture::new();
    let dev = f.repo("development");
    let pin = f.git(&dev, &["rev-parse", "HEAD"]);
    f.git(&dev, &["tag", "release"]);
    std::fs::write(dev.join("tracked.txt"), "later development\n").unwrap();
    f.git(&dev, &["commit", "-qam", "later development"]);
    let head = f.git(&dev, &["rev-parse", "HEAD"]);
    let url = "https://example.invalid/team/repo";
    f.git(&dev, &["remote", "add", "origin", url]);
    f.configure(json!({"a":{"url":url}}), json!({"a":dev}));
    let cached = f.app.join(".sources/checkouts/a");
    std::fs::create_dir_all(&cached).unwrap();
    f.git(&cached, &["init"]);
    f.git(&cached, &["remote", "add", "origin", url]);
    let broken_head = b"broken managed HEAD\n";
    std::fs::write(cached.join(".git/HEAD"), broken_head).unwrap();
    std::fs::write(cached.join("sentinel.txt"), "leave the cache alone\n").unwrap();

    success(f.cli().args(["lock", "a:tag=release"]).output().unwrap());
    let path = f.app.join(".sources.lock");
    let before = std::fs::read(&path).unwrap();
    let lock: serde_json::Value = serde_json::from_slice(&before).unwrap();
    assert_eq!(lock["sources"]["a"], pin);
    assert_eq!(f.git(&dev, &["rev-parse", "HEAD"]), head);
    assert_eq!(
        std::fs::read(cached.join(".git/HEAD")).unwrap(),
        broken_head
    );
    assert_eq!(
        std::fs::read_to_string(cached.join("sentinel.txt")).unwrap(),
        "leave the cache alone\n"
    );
    assert!(!f.app.join("sources").exists());

    f.git(&dev, &["tag", "-d", "release"]);
    let output = f.cli().args(["lock", "a:tag=release"]).output().unwrap();
    assert!(String::from_utf8_lossy(&output.stderr).contains(cached.to_str().unwrap()));
    failure(output, "cannot resolve tag");
    assert_eq!(std::fs::read(&path).unwrap(), before);
}
