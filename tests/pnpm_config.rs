mod common;
use common::{Fixture, failure, success};
use serde_json::json;

#[test]
fn pnpm_package_mappings_validate_paths_and_names() {
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
    configure(json!({"@example/library": {"path": "packages/library"}}));
    success(f.cli().arg("check").output().unwrap());
    for path in ["../outside", "/outside", "packages/../../outside", ""] {
        configure(json!({"@example/library": {"path": path}}));
        failure(f.cli().arg("check").output().unwrap(), "package path");
    }
    for name in [
        "",
        "../library",
        "@example",
        "@example/library/extra",
        "bad name",
    ] {
        configure(json!({name: {"path": "."}}));
        failure(f.cli().arg("check").output().unwrap(), "package name");
    }
}

#[test]
fn two_sources_cannot_control_the_same_pnpm_package() {
    let f = Fixture::new();
    let mapping = json!({"url": "https://example.invalid/team/library", "pnpm": {
        "@example/library": {"path": "."}
    }});
    f.configure(json!({"one": mapping, "two": mapping}), json!({}));
    failure(
        f.cli().arg("check").output().unwrap(),
        "duplicate pnpm package",
    );
}
