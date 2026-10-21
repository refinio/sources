mod common;

use common::{Fixture, failure, success};
use serde_json::json;

#[test]
fn turbo_packages_are_optional_and_coexist_with_pnpm_release_mappings() {
    let f = Fixture::new();
    let dev = f.repo("child");
    let upstream = "https://example.invalid/team/child";
    f.git(&dev, &["remote", "add", "origin", upstream]);
    f.configure(
        json!({"child": {
            "url": upstream,
            "pnpm": {"@example/protocol": {"path": "."}},
            "packages": {
                "@Example/protocol": {"tasks": ["build", "pack:app", "test with spaces", "-legacy"]},
                "@example/ui": {"tasks": ["pack"]}
            }
        }}),
        json!({"child": dev}),
    );
    success(f.cli().arg("check").output().unwrap());
    f.configure(json!({"child": {"url": upstream}}), json!({"child": dev}));
    success(f.cli().arg("check").output().unwrap());
}

#[test]
fn turbo_hash_names_follow_the_ascii_component_normalization_contract() {
    use sources::config::turbo_env_name;
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
        ("foo_9", "__custom__", "SOURCES_FOO_9__CUSTOM"),
        (
            "éxample/共同_package",
            "build",
            "SOURCES_XAMPLE_PACKAGE__BUILD",
        ),
    ] {
        assert_eq!(turbo_env_name(package, task).unwrap(), expected);
    }
}

#[test]
fn malformed_turbo_package_task_lists_fail_with_actionable_errors() {
    let f = Fixture::new();
    for (package, message) in [
        (json!({}), "tasks"),
        (json!({"tasks": []}), "must declare at least one Turbo task"),
        (json!({"tasks": ["build", "build"]}), "duplicate Turbo task"),
        (
            json!({"tasks": ["build"], "command": "echo bypass"}),
            "unknown field `command`",
        ),
        (
            json!({"tasks": ["build"], "script": "build"}),
            "unknown field `script`",
        ),
    ] {
        f.configure(json!({"child": {"url": "https://example.invalid/child", "packages": {"@example/protocol": package}}}), json!({}));
        failure(f.cli().arg("check").output().unwrap(), message);
    }
}

#[test]
fn turbo_identity_rejects_ambiguous_delimiters_controls_and_empty_components() {
    use sources::config::turbo_env_name;
    for package in [
        "",
        "bad__package",
        "bad:package",
        "bad#package",
        "bad\npackage",
        "-package",
        "共同",
        "...",
    ] {
        assert!(
            turbo_env_name(package, "build").is_err(),
            "accepted {package:?}"
        );
    }
    for task in ["", "bad#task", "bad\0task", "bad\ntask", "共同", "---"] {
        assert!(
            turbo_env_name("@example/protocol", task).is_err(),
            "accepted {task:?}"
        );
    }
}

#[test]
fn turbo_packages_have_one_owner_and_globally_unique_environment_names() {
    let f = Fixture::new();
    for (one, two, message) in [
        (
            json!({"@example/protocol": {"tasks": ["build"]}}),
            json!({"@example/protocol": {"tasks": ["pack"]}}),
            "duplicate Turbo package",
        ),
        (
            json!({"@Example/protocol": {"tasks": ["build"]}}),
            json!({"@example/protocol": {"tasks": ["build"]}}),
            "Turbo environment name collision",
        ),
        (
            json!({"protocol-ui": {"tasks": ["build"]}}),
            json!({"protocol.ui": {"tasks": ["build"]}}),
            "Turbo environment name collision",
        ),
        (
            json!({"@example/protocol": {"tasks": ["pack-app", "pack:app"]}}),
            json!({}),
            "Turbo environment name collision",
        ),
    ] {
        f.configure(
            json!({
                "one": {"url": "https://example.invalid/one", "packages": one},
                "two": {"url": "https://example.invalid/two", "packages": two}
            }),
            json!({}),
        );
        failure(f.cli().arg("check").output().unwrap(), message);
    }
    f.configure(json!({
        "one": {"url": "https://example.invalid/one", "packages": {"@example/protocol": {"tasks": ["build", "pack"]}}},
        "two": {"url": "https://example.invalid/two", "packages": {"@example/ui": {"tasks": ["pack"]}}}
    }), json!({}));
    success(f.cli().args(["status", "--json"]).output().unwrap());
}
