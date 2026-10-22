mod common;
use common::{Fixture, success};
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
