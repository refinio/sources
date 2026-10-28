mod common;

use common::{Fixture, failure, success};

#[test]
fn agents_can_discover_and_read_bundled_skills_without_installing_them() {
    let f = Fixture::new();
    let help = success(f.cli().arg("--help").output().unwrap());
    let help = String::from_utf8(help.stdout).unwrap();
    assert!(help.contains("npx skills add refinio/sources --global"));
    assert!(help.contains("sources skills"));
    assert!(help.contains("sources skills <name>"));
    let skills_help = success(f.cli().args(["skills", "--help"]).output().unwrap());
    assert!(String::from_utf8_lossy(&skills_help.stdout).contains("sources skills <name>"));

    let overview = success(f.cli().arg("skills").output().unwrap());
    let overview = String::from_utf8(overview.stdout).unwrap();
    for (name, content) in [
        (
            "sources-setup",
            include_str!("../skills/sources-setup/SKILL.md"),
        ),
        (
            "sources-release",
            include_str!("../skills/sources-release/SKILL.md"),
        ),
    ] {
        assert!(overview.contains(name), "{overview}");
        let output = success(f.cli().args(["skills", name]).output().unwrap());
        assert_eq!(output.stdout, content.as_bytes());
    }
    failure(
        f.cli().args(["skills", "unknown"]).output().unwrap(),
        "sources skills",
    );
    // Even a broken project must leave the embedded guidance accessible.
    std::fs::write(f.app.join(".sources.conf"), "invalid JSON").unwrap();
    success(f.cli().arg("skills").output().unwrap());
    assert!(!f.app.join(".sources").exists());
}
