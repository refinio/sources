//! Critical public API contracts; no CLI, Git, pnpm or Turbo subprocesses.
use serde_json::{Value, json};
use sources::config::{Project, turbo_env_name};
use sources::git::normalized_identity;
use std::path::Path;

#[test]
fn repository_identity_preserves_repository_and_transport_boundaries() {
    for url in [
        "https://GitHub.com/Owner/Repo.git",
        "ssh://alice:secret@GitHub.COM/Owner/Repo",
        "git@github.com:Owner/Repo.git",
    ] {
        assert_eq!(normalized_identity(url).unwrap(), "github.com/Owner/Repo");
    }
    let standard = normalized_identity("https://example.com/Owner/Repo").unwrap();
    assert_eq!(
        standard,
        normalized_identity("ssh://git@example.com:22/Owner/Repo.git").unwrap()
    );
    for different in [
        "https://example.com/owner/repo",
        "ssh://git@example.com:443/Owner/Repo",
    ] {
        assert_ne!(standard, normalized_identity(different).unwrap());
    }
    assert_ne!(
        normalized_identity("https://example.com:8443/Owner/Repo").unwrap(),
        normalized_identity("ssh://git@example.com:8443/Owner/Repo").unwrap()
    );
}

#[test]
fn turbo_environment_identity_is_stable_and_unambiguous() {
    for (package, task, expected) in [
        (
            "@example/protocol",
            "build",
            "SOURCES_EXAMPLE_PROTOCOL__BUILD",
        ),
        (
            "@example/protocol",
            "pack:app",
            "SOURCES_EXAMPLE_PROTOCOL__PACK_APP",
        ),
        (
            "Pkg...-ä--Name",
            "build :app",
            "SOURCES_PKG_NAME__BUILD_APP",
        ),
    ] {
        assert_eq!(turbo_env_name(package, task).unwrap(), expected);
    }
    for (package, task) in [
        ("bad__package", "build"),
        ("bad:package", "build"),
        ("共同", "build"),
        ("pkg", ""),
        ("pkg", "bad#task"),
        ("pkg", "bad\ntask"),
    ] {
        assert!(turbo_env_name(package, task).is_err());
    }
}

fn project(root: &Path, directory: &str, sources: Value, local: Value) -> Project {
    std::fs::create_dir_all(root.join(".sources")).unwrap();
    std::fs::write(
        root.join(".sources.conf"),
        json!({
            "version":1,"directory":directory,"sources":sources
        })
        .to_string(),
    )
    .unwrap();
    std::fs::write(
        root.join(".sources.local.conf"),
        json!({"sources":local}).to_string(),
    )
    .unwrap();
    Project::load(root.to_owned()).unwrap()
}

#[test]
fn configuration_keeps_paths_separate_and_package_ownership_unique() {
    let root = tempfile::tempdir().unwrap();
    let source = json!({"url":"https://example.invalid/sdk", "pnpm":{
        "@example/core":{"path":"packages/core"}
    },"packages":{"@example/core":{"tasks":["build"]}}});
    project(root.path(), "sources", json!({"sdk":source}), json!({}))
        .validate()
        .unwrap();
    for (directory, sources, local, message) in [
        (
            "sources",
            json!({"../outside":source}),
            json!({}),
            "invalid source",
        ),
        (
            ".sources/links",
            json!({"sdk":source}),
            json!({}),
            "overlaps .sources",
        ),
        (
            "sources",
            json!({"sdk":source}),
            json!({"sdk":".sources/checkouts/sdk"}),
            "overlaps managed",
        ),
        (
            "sources",
            json!({"a":source,"b":source}),
            json!({}),
            "duplicate Turbo package",
        ),
        (
            "sources",
            json!({"sdk":{"url":"https://example.invalid/sdk","pnpm":{
                "@example/core":{"path":"../outside"}
            }}}),
            json!({}),
            "package path",
        ),
        (
            "sources",
            json!({"sdk":{"url":"https://example.invalid/sdk","packages":{
                "core-ui":{"tasks":["build"]},"core.ui":{"tasks":["build"]}
            }}}),
            json!({}),
            "environment name collision",
        ),
    ] {
        let error = project(root.path(), directory, sources, local)
            .validate()
            .unwrap_err();
        assert!(format!("{error:#}").contains(message), "{error:#}");
    }
}

#[test]
fn pins_require_exact_git_and_registry_identities_without_losing_either_kind() {
    let root = tempfile::tempdir().unwrap();
    let project = project(
        root.path(),
        "sources",
        json!({"sdk":{
            "url":"https://example.invalid/sdk","pnpm":{"@example/core":{"path":"."}}
        }}),
        json!({}),
    );
    let hash = "a".repeat(40);
    for (git, version, valid) in [
        (hash.as_str(), "1.2.3", true),
        ("abcdef", "1.2.3", false),
        (hash.as_str(), "^1.2.3", false),
    ] {
        std::fs::write(
            root.path().join(".sources.lock"),
            json!({"version":2,"sources":{
                "sdk":{"git":git,"pnpm":{"@example/core":version}}
            }})
            .to_string(),
        )
        .unwrap();
        let pins = project.pins(true);
        if valid {
            let pins = pins.unwrap();
            assert_eq!(pins.sources["sdk"].git.as_deref(), Some(hash.as_str()));
            assert_eq!(pins.sources["sdk"].pnpm["@example/core"], "1.2.3");
        } else {
            assert!(pins.is_err());
        }
    }
}
