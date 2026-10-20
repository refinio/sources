mod common;
use common::{Fixture, failure, success};
use serde_json::json;

#[test]
fn check_discovers_project_accepts_fork_upstream_and_warns_without_changing_checkout() {
    let f = Fixture::new();
    let dev = f.repo("dev checkout");
    f.git(
        &dev,
        &[
            "remote",
            "add",
            "origin",
            "https://example.invalid/fork/repo.git",
        ],
    );
    f.git(
        &dev,
        &[
            "remote",
            "add",
            "upstream",
            "git@example.invalid:team/repo.git",
        ],
    );
    f.configure(
        json!({"a": {"url": "https://example.invalid/team/repo"}}),
        json!({"a": dev}),
    );
    std::fs::write(dev.join("tracked.txt"), "source edit\n").unwrap();
    std::fs::write(dev.join("untracked.txt"), "untracked\n").unwrap();
    std::fs::write(dev.join("artifact.txt"), "ignored\n").unwrap();
    let before = f.git(&dev, &["status", "--porcelain=v1"]);
    let nested = f.app.join("packages/nested");
    std::fs::create_dir_all(&nested).unwrap();
    let out = success(f.cli().current_dir(nested).arg("check").output().unwrap());
    let warning = String::from_utf8_lossy(&out.stderr);
    assert!(
        warning.contains("a")
            && warning.contains("tracked.txt")
            && warning.contains("untracked.txt"),
        "{warning}"
    );
    assert!(!warning.contains("artifact.txt"), "{warning}");
    assert_eq!(f.git(&dev, &["status", "--porcelain=v1"]), before);
    assert!(!f.app.join(".sources.lock").exists());
    assert!(!f.app.join("sources").exists());
}

#[test]
fn check_rejects_wrong_repository_and_unknown_local_mapping() {
    let f = Fixture::new();
    let dev = f.repo("dev");
    f.git(
        &dev,
        &[
            "remote",
            "add",
            "origin",
            "https://example.invalid/other/repo.git",
        ],
    );
    f.configure(
        json!({"a": {"url": "https://example.invalid/team/repo"}}),
        json!({"a": dev}),
    );
    failure(f.cli().arg("check").output().unwrap(), "a");
    f.configure(json!({}), json!({"unknown": dev}));
    failure(f.cli().arg("check").output().unwrap(), "unknown");
}

#[test]
fn mismatch_errors_redact_credentials_with_quotes_in_url_userinfo() {
    let f = Fixture::new();
    let dev = f.repo("dev");
    f.git(
        &dev,
        &[
            "remote",
            "add",
            "origin",
            "https://example.invalid/other/repo",
        ],
    );
    for password in ["pa'ss", "pa\"ss", "pa ss"] {
        f.configure(
            json!({"a":{"url":format!("https://alice:{password}@example.invalid/team/repo")}}),
            json!({"a":dev}),
        );
        let output = f.cli().arg("check").output().unwrap();
        assert!(!output.status.success());
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("mismatch"), "{stderr}");
        assert!(
            !stderr.contains("alice") && !stderr.contains(password),
            "{stderr}"
        );
    }
}

#[test]
fn relative_local_upstreams_use_project_and_checkout_roots_from_nested_invocations() {
    let f = Fixture::new();
    f.repo("upstream");
    let dev = f.repo("dev");
    f.git(&dev, &["remote", "add", "origin", "../upstream"]);
    f.configure(json!({"a":{"url":"../upstream"}}), json!({"a":dev}));
    let nested = f.app.join("packages/nested");
    std::fs::create_dir_all(&nested).unwrap();
    success(f.cli().current_dir(nested).arg("check").output().unwrap());
}

#[test]
fn local_inspection_accepts_matching_relative_urls_when_upstream_is_unavailable() {
    let f = Fixture::new();
    let dev = f.repo("dev");
    f.git(&dev, &["remote", "add", "origin", "../missing-upstream"]);
    f.configure(json!({"a":{"url":"../missing-upstream"}}), json!({"a":dev}));
    success(f.cli().arg("check").output().unwrap());
}

#[cfg(unix)]
#[test]
fn cached_inputs_and_development_identity_survive_a_dangling_upstream_alias() {
    let f = Fixture::new();
    let upstream = f.repo("upstream");
    let alias = f.root.join("upstream-alias");
    std::os::unix::fs::symlink(&upstream, &alias).unwrap();
    let dev = f.repo("dev");
    f.git(&dev, &["remote", "add", "origin", alias.to_str().unwrap()]);
    f.configure(json!({"a":{"url":alias}}), json!({"a":dev}));
    success(f.cli().args(["lock", "--check"]).output().unwrap());
    std::fs::rename(upstream, f.root.join("upstream-away")).unwrap();
    success(f.cli().arg("check").output().unwrap());
    success(f.cli().arg("check-release").output().unwrap());
}

#[cfg(unix)]
#[test]
fn cyclic_dangling_upstream_alias_returns_an_error() {
    let f = Fixture::new();
    let alias = f.root.join("alias");
    std::os::unix::fs::symlink("missing/../alias", &alias).unwrap();
    let dev = f.repo("dev");
    f.git(&dev, &["remote", "add", "origin", alias.to_str().unwrap()]);
    f.configure(json!({"a":{"url":alias}}), json!({"a":dev}));
    failure(
        f.cli().arg("check").output().unwrap(),
        "too many symbolic links",
    );
}
