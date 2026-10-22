mod common;
use common::{Fixture, failure, success};
use serde_json::json;

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
        assert_eq!(lock["sources"]["a"]["git"], expected);
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
    let before = std::fs::read(f.app.join(".sources.lock")).unwrap();
    failure(
        f.cli()
            .args(["lock", "a:tag=missing", "--check"])
            .output()
            .unwrap(),
        "tag",
    );
    assert_eq!(std::fs::read(f.app.join(".sources.lock")).unwrap(), before);
    assert!(!f.app.join("sources").exists());
}
