mod common;
use common::{Fixture, success};

#[test]
fn setup_is_repeatable_preserves_configuration_and_adds_ignore_entries() {
    let fixture = Fixture::new();
    std::fs::write(fixture.app.join(".gitignore"), "existing-pattern").unwrap();
    success(fixture.cli().arg("setup").output().unwrap());
    let config = fixture.app.join(".sources.conf");
    let original = std::fs::read(&config).unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&original).unwrap()["version"],
        1
    );
    assert!(fixture.app.join(".sources.local.conf").is_file());
    assert!(fixture.app.join(".sources.lock").is_file());
    assert!(fixture.app.join(".sources").is_dir());
    assert!(!fixture.app.join("sources").exists());
    success(fixture.cli().arg("setup").output().unwrap());
    assert_eq!(std::fs::read(config).unwrap(), original);
    let ignore = std::fs::read_to_string(fixture.app.join(".gitignore")).unwrap();
    assert!(ignore.starts_with("existing-pattern\n"));
    for entry in ["/.sources/", "/.sources.local.conf", "/sources/"] {
        assert_eq!(ignore.lines().filter(|line| *line == entry).count(), 1);
    }
}
