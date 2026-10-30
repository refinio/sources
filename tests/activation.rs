mod common;

use common::{Fixture, failure, success};
use serde_json::json;
use std::path::{Path, PathBuf};

fn add_module(f: &Fixture, parent: &Path, url: &str, path: &str) {
    f.git(
        parent,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            url,
            path,
        ],
    );
    f.git(parent, &["commit", "-qam", "add module"]);
}

fn pin(f: &Fixture, upstream: &Path) -> String {
    let commit = f.git(upstream, &["rev-parse", "HEAD"]);
    std::fs::write(
        f.app.join(".sources.lock"),
        json!({"version": 2, "sources": {"a": {"git": commit}}}).to_string(),
    )
    .unwrap();
    commit
}

fn recursive_fixture() -> (Fixture, PathBuf, PathBuf, PathBuf) {
    let f = Fixture::new();
    let nested = f.repo("nested");
    let module = f.repo("module");
    add_module(&f, &module, "../nested", "nested child");
    let parent = f.repo("upstream");
    add_module(&f, &parent, "../module", "dep");
    f.configure(json!({"a": {"url": parent}}), json!({}));
    pin(&f, &parent);
    (f, parent, module, nested)
}

#[test]
fn release_activates_recursive_recorded_pins_and_retains_ignored_module_artifacts() {
    let (f, parent, module, nested) = recursive_fixture();
    success(f.cli().arg("release").output().unwrap());
    let managed = f.app.join(".sources/checkouts/a");
    let module_worktree = managed.join("dep");
    let nested_worktree = module_worktree.join("nested child");
    assert_eq!(
        f.git(&module_worktree, &["rev-parse", "HEAD"]),
        f.git(&module, &["rev-parse", "HEAD"])
    );
    assert_eq!(
        f.git(&nested_worktree, &["rev-parse", "HEAD"]),
        f.git(&nested, &["rev-parse", "HEAD"])
    );
    std::fs::write(
        module_worktree.join("artifact.txt"),
        "retained module output\n",
    )
    .unwrap();
    std::fs::write(
        nested_worktree.join("artifact.txt"),
        "retained nested output\n",
    )
    .unwrap();
    std::fs::write(nested.join("tracked.txt"), "future nested source\n").unwrap();
    f.git(&nested, &["commit", "-qam", "update nested"]);
    let next_nested = f.git(&nested, &["rev-parse", "HEAD"]);
    f.git(
        &module,
        &[
            "update-index",
            "--cacheinfo",
            &format!("160000,{next_nested},nested child"),
        ],
    );
    f.git(&module, &["commit", "-qm", "update module input"]);
    let next_module = f.git(&module, &["rev-parse", "HEAD"]);
    f.git(
        &parent,
        &[
            "update-index",
            "--cacheinfo",
            &format!("160000,{next_module},dep"),
        ],
    );
    f.git(&parent, &["commit", "-qm", "update parent input"]);
    pin(&f, &parent);
    success(f.cli().arg("release").output().unwrap());
    assert_eq!(f.git(&nested_worktree, &["rev-parse", "HEAD"]), next_nested);
    assert_eq!(
        std::fs::read_to_string(module_worktree.join("artifact.txt")).unwrap(),
        "retained module output\n"
    );
    assert_eq!(
        std::fs::read_to_string(nested_worktree.join("artifact.txt")).unwrap(),
        "retained nested output\n"
    );
    assert_eq!(
        std::fs::read_to_string(nested_worktree.join("tracked.txt")).unwrap(),
        "future nested source\n"
    );

    // A direct source cannot select another version of a recursive submodule.
    let previous_nested = f.git(&nested, &["rev-parse", "HEAD^"]);
    f.configure(
        json!({"a": {"url": parent}, "nested-direct": {"url": nested}}),
        json!({}),
    );
    let parent_commit = f.git(&parent, &["rev-parse", "HEAD"]);
    std::fs::write(
        f.app.join(".sources.lock"),
        json!({
            "version": 2, "sources": {
                "a": {"git": parent_commit}, "nested-direct": {"git": previous_nested}
            }
        })
        .to_string(),
    )
    .unwrap();
    let output = f.cli().arg("check-release").output().unwrap();
    let diagnostic = String::from_utf8_lossy(&output.stderr).to_string();
    failure(output, "conflicting pinned versions");
    assert!(
        diagnostic.contains("a -> submodule dep -> submodule nested child"),
        "{diagnostic}"
    );
    assert!(diagnostic.contains("nested-direct"), "{diagnostic}");
    assert!(diagnostic.contains(&next_nested), "{diagnostic}");
    assert!(diagnostic.contains(&previous_nested), "{diagnostic}");
    assert_eq!(f.git(&nested_worktree, &["rev-parse", "HEAD"]), next_nested);
    std::fs::write(
        f.app.join(".sources.lock"),
        json!({
            "version": 2, "sources": {
                "a": {"git": parent_commit}, "nested-direct": {"git": next_nested}
            }
        })
        .to_string(),
    )
    .unwrap();
    success(f.cli().arg("check-release").output().unwrap());
    success(f.cli().args(["release", "--offline"]).output().unwrap());
}
