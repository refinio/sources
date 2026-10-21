mod common;
use common::{Fixture, failure, success};
use serde_json::{Value, json};

#[test]
fn git_and_registry_pins_coexist_and_updates_preserve_the_other_selection() {
    let f = Fixture::new();
    let upstream = f.repo("upstream");
    let dev = f.root.join("dev");
    f.git(
        &f.root,
        &["clone", upstream.to_str().unwrap(), dev.to_str().unwrap()],
    );
    f.configure(
        json!({"sdk":{"url":upstream,"pnpm":{
            "@example/core":{"path":"packages/core"},
            "@example/ui":{"path":"packages/ui"}
        }}}),
        json!({"sdk":dev}),
    );
    success(f.cli().args(["lock", "--git"]).output().unwrap());
    success(
        f.cli()
            .args(["lock", "--registry", "@example/core:1.2.3"])
            .output()
            .unwrap(),
    );
    failure(
        f.cli()
            .args(["release", "--registry", "--clean"])
            .output()
            .unwrap(),
        "missing registry pin for @example/ui",
    );
    assert!(!f.app.join("sources/sdk").exists());
    success(
        f.cli()
            .args([
                "lock",
                "--registry",
                "@example/core:1.2.3",
                "@example/ui:2.0.0-beta.1",
            ])
            .output()
            .unwrap(),
    );
    let path = f.app.join(".sources.lock");
    let read = || serde_json::from_slice::<Value>(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(
        read()["sources"]["sdk"]["git"],
        f.git(&dev, &["rev-parse", "HEAD"])
    );
    assert_eq!(
        read()["sources"]["sdk"]["pnpm"]["@example/ui"],
        "2.0.0-beta.1"
    );
    let before = std::fs::read(&path).unwrap();
    for updates in [
        vec!["@example/core:2.0.0", "@example/ui:latest"],
        vec!["missing:1.0.0"],
        vec!["@example/core:1.0.0", "@example/core:2.0.0"],
    ] {
        assert!(
            !f.cli()
                .args(["lock", "--registry"])
                .args(updates)
                .output()
                .unwrap()
                .status
                .success()
        );
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }
    std::fs::write(dev.join("new.txt"), "new revision\n").unwrap();
    f.git(&dev, &["add", "."]);
    f.git(&dev, &["commit", "-qm", "next revision"]);
    success(f.cli().arg("lock").output().unwrap());
    assert_eq!(
        read()["sources"]["sdk"]["git"],
        f.git(&dev, &["rev-parse", "HEAD"])
    );
    assert_eq!(read()["sources"]["sdk"]["pnpm"]["@example/core"], "1.2.3");
    failure(
        f.cli().args(["release", "--registry"]).output().unwrap(),
        "package.json",
    );
    for args in [
        vec!["lock", "--git", "--registry"],
        vec!["release", "--git", "--registry"],
        vec!["lock", "--registry"],
    ] {
        assert_eq!(f.cli().args(args).output().unwrap().status.code(), Some(2));
    }
}
