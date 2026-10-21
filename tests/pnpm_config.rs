mod common;
use common::{Fixture, failure, success};
use serde_json::json;

#[test]
fn pnpm_package_mappings_validate_paths_names_and_exact_release_versions() {
    let f = Fixture::new();
    let dev = f.repo("library");
    let upstream = "https://example.invalid/team/library";
    f.git(&dev, &["remote", "add", "origin", upstream]);
    let configure = |packages: serde_json::Value| {
        f.configure(
            json!({"library": {"url": upstream, "pnpm": packages}}),
            json!({"library": dev}),
        );
    };
    configure(json!({"@example/library": {"path": "packages/library", "version": "1.2.3-beta.1"}}));
    success(f.cli().arg("check").output().unwrap());
    for path in ["../outside", "/outside", "packages/../../outside", ""] {
        configure(json!({"@example/library": {"path": path, "version": "1.2.3"}}));
        failure(f.cli().arg("check").output().unwrap(), "package path");
    }
    for version in ["^1.2.3", "latest", "1.2", "file:../library", ""] {
        configure(json!({"@example/library": {"path": ".", "version": version}}));
        failure(
            f.cli().arg("check").output().unwrap(),
            "exact package version",
        );
    }
    for name in [
        "",
        "../library",
        "@example",
        "@example/library/extra",
        "bad name",
    ] {
        configure(json!({name: {"path": ".", "version": "1.2.3"}}));
        failure(f.cli().arg("check").output().unwrap(), "package name");
    }
}

#[test]
fn two_sources_cannot_control_the_same_pnpm_package() {
    let f = Fixture::new();
    let mapping = json!({"url": "https://example.invalid/team/library", "pnpm": {
        "@example/library": {"path": ".", "version": "1.2.3"}
    }});
    f.configure(json!({"one": mapping, "two": mapping}), json!({}));
    failure(
        f.cli().arg("check").output().unwrap(),
        "duplicate pnpm package",
    );
}
