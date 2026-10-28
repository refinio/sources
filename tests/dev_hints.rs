mod common;
use common::{Fixture, success};
use serde_json::{Value, json};

#[test]
fn capture_dev_hints_from_linked_checkouts_and_preserve_config_on_failure() {
    let f = Fixture::new();
    let a = f.repo("a");
    let b = f.repo("b");
    let upstream = f.repo("upstream");
    for repo in [&a, &b] {
        f.git(
            repo,
            &["remote", "add", "origin", "https://example.invalid/sdk"],
        );
    }
    f.git(
        &a,
        &["remote", "set-url", "origin", upstream.to_str().unwrap()],
    );
    f.git(&a, &["switch", "-c", "feature/dev"]);
    f.configure(
        json!({
            "a":{"url":"../upstream","dev_hint":"v0.1.0",
                "packages":{"@example/core":{"tasks":["build"]}}},
            "b":{"url":"https://example.invalid/sdk"}
        }),
        json!({"a":a,"b":b}),
    );
    success(f.cli().arg("setup").output().unwrap());
    success(f.cli().arg("dev").output().unwrap());
    let config = f.app.join(".sources.conf");
    let local = std::fs::read(f.app.join(".sources.local.conf")).unwrap();
    let lock = std::fs::read(f.app.join(".sources.lock")).unwrap();
    success(
        f.cli()
            .args(["dev-hints", "--ref", "branch"])
            .output()
            .unwrap(),
    );
    let mut expected: Value = serde_json::from_slice(&std::fs::read(&config).unwrap()).unwrap();
    assert_eq!(expected["sources"]["a"]["dev_hint"], "feature/dev");
    assert_eq!(expected["sources"]["b"]["dev_hint"], "main");
    assert_eq!(expected["sources"]["a"]["url"], "../upstream");
    assert_eq!(
        expected["sources"]["a"]["packages"]["@example/core"]["tasks"],
        json!(["build"])
    );

    f.git(&a, &["tag", "-am", "release", "v1.0.0"]);
    success(
        f.cli()
            .args(["dev-hints", "a", "--ref", "tag"])
            .output()
            .unwrap(),
    );
    expected["sources"]["a"]["dev_hint"] = json!("v1.0.0");
    assert_eq!(
        serde_json::from_slice::<Value>(&std::fs::read(&config).unwrap()).unwrap(),
        expected
    );

    // A failure in the second source must not save the first source's new hint.
    f.git(&b, &["checkout", "--detach"]);
    let before = std::fs::read(&config).unwrap();
    let out = f
        .cli()
        .args(["dev-hints", "--ref", "branch"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let error = String::from_utf8_lossy(&out.stderr);
    assert!(error.contains("b") && error.contains("detached"), "{error}");
    assert_eq!(std::fs::read(&config).unwrap(), before);

    success(
        f.cli()
            .args(["dev-hints", "b", "--ref", "commit"])
            .output()
            .unwrap(),
    );
    expected["sources"]["b"]["dev_hint"] = json!(f.git(&b, &["rev-parse", "HEAD"]));
    assert_eq!(
        serde_json::from_slice::<Value>(&std::fs::read(&config).unwrap()).unwrap(),
        expected
    );
    assert_eq!(f.git(&a, &["branch", "--show-current"]), "feature/dev");
    assert_eq!(f.git(&b, &["branch", "--show-current"]), "");
    assert_eq!(
        std::fs::read(f.app.join(".sources.local.conf")).unwrap(),
        local
    );
    assert_eq!(std::fs::read(f.app.join(".sources.lock")).unwrap(), lock);
    assert_eq!(
        std::fs::canonicalize(f.app.join("sources/a")).unwrap(),
        std::fs::canonicalize(&a).unwrap()
    );
    assert_eq!(
        std::fs::canonicalize(f.app.join("sources/b")).unwrap(),
        std::fs::canonicalize(&b).unwrap()
    );

    // Literal hints also work before checkouts have been linked locally.
    std::fs::rename(f.app.join("sources"), f.app.join("saved-links")).unwrap();
    success(
        f.cli()
            .args(["dev-hints", "a", "--hint", "next/development"])
            .output()
            .unwrap(),
    );
    expected["sources"]["a"]["dev_hint"] = json!("next/development");
    assert_eq!(
        serde_json::from_slice::<Value>(&std::fs::read(&config).unwrap()).unwrap(),
        expected
    );
    success(
        f.cli()
            .args(["dev-hints", "--hint", "refs/tags/future-release"])
            .output()
            .unwrap(),
    );
    for source in ["a", "b"] {
        expected["sources"][source]["dev_hint"] = json!("refs/tags/future-release");
    }
    assert_eq!(
        serde_json::from_slice::<Value>(&std::fs::read(&config).unwrap()).unwrap(),
        expected
    );
    success(f.cli().arg("dev").output().unwrap());
    assert_eq!(f.git(&a, &["branch", "--show-current"]), "feature/dev");
    assert_eq!(f.git(&b, &["branch", "--show-current"]), "");
}
