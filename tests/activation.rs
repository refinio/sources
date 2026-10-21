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
}

#[test]
fn ignored_module_collisions_survive_failed_activation_and_rerun_repairs_partial_selection() {
    let (f, parent, module, nested) = recursive_fixture();
    success(f.cli().arg("release").output().unwrap());
    let managed_nested = f.app.join(".sources/checkouts/a/dep/nested child");
    let first = f.git(&managed_nested, &["rev-parse", "HEAD"]);
    std::fs::write(
        managed_nested.join("artifact.txt"),
        "valuable ignored contents\n",
    )
    .unwrap();
    std::fs::write(nested.join("artifact.txt"), "new tracked contents\n").unwrap();
    f.git(&nested, &["add", "-f", "artifact.txt"]);
    f.git(&nested, &["commit", "-qm", "track artifact path"]);
    let next_nested = f.git(&nested, &["rev-parse", "HEAD"]);
    f.git(
        &module,
        &[
            "update-index",
            "--cacheinfo",
            &format!("160000,{next_nested},nested child"),
        ],
    );
    f.git(&module, &["commit", "-qm", "update nested input"]);
    let next_module = f.git(&module, &["rev-parse", "HEAD"]);
    f.git(
        &parent,
        &[
            "update-index",
            "--cacheinfo",
            &format!("160000,{next_module},dep"),
        ],
    );
    f.git(&parent, &["commit", "-qm", "update module input"]);
    pin(&f, &parent);
    failure(f.cli().arg("release").output().unwrap(), "artifact.txt");
    assert_eq!(
        std::fs::read_to_string(managed_nested.join("artifact.txt")).unwrap(),
        "valuable ignored contents\n"
    );
    assert_eq!(f.git(&managed_nested, &["rev-parse", "HEAD"]), first);
    std::fs::remove_file(managed_nested.join("artifact.txt")).unwrap();
    success(f.cli().arg("release").output().unwrap());
    assert_eq!(f.git(&managed_nested, &["rev-parse", "HEAD"]), next_nested);
    assert_eq!(
        std::fs::read_to_string(managed_nested.join("artifact.txt")).unwrap(),
        "new tracked contents\n"
    );
}

#[test]
fn cached_future_nested_url_changes_activate_offline_with_upstreams_unavailable() {
    let (f, parent, module, nested) = recursive_fixture();
    success(f.cli().arg("release").output().unwrap());
    let replacement = f.repo("nested-replacement");
    std::fs::write(replacement.join("tracked.txt"), "replacement input\n").unwrap();
    f.git(&replacement, &["commit", "-qam", "replacement contents"]);
    let replacement_pin = f.git(&replacement, &["rev-parse", "HEAD"]);
    f.git(
        &module,
        &[
            "config",
            "--file",
            ".gitmodules",
            "submodule.nested child.url",
            "../nested-replacement",
        ],
    );
    f.git(&module, &["add", ".gitmodules"]);
    f.git(
        &module,
        &[
            "update-index",
            "--cacheinfo",
            &format!("160000,{replacement_pin},nested child"),
        ],
    );
    f.git(&module, &["commit", "-qm", "replace nested repository"]);
    let module_pin = f.git(&module, &["rev-parse", "HEAD"]);
    f.git(
        &parent,
        &[
            "update-index",
            "--cacheinfo",
            &format!("160000,{module_pin},dep"),
        ],
    );
    f.git(&parent, &["commit", "-qm", "select replacement input"]);
    let parent_pin = pin(&f, &parent);
    success(f.cli().arg("check-release").output().unwrap());
    for upstream in [&parent, &module, &nested, &replacement] {
        std::fs::rename(upstream, upstream.with_extension("unavailable")).unwrap();
    }
    success(f.cli().args(["release", "--offline"]).output().unwrap());
    let managed = f.app.join(".sources/checkouts/a");
    let managed_nested = managed.join("dep/nested child");
    assert_eq!(f.git(&managed, &["rev-parse", "HEAD"]), parent_pin);
    assert_eq!(
        f.git(&managed_nested, &["rev-parse", "HEAD"]),
        replacement_pin
    );
    assert_eq!(
        std::fs::read_to_string(managed_nested.join("tracked.txt")).unwrap(),
        "replacement input\n"
    );
    assert!(
        f.git(&managed_nested, &["remote", "get-url", "origin"])
            .ends_with("/nested-replacement")
    );
}

#[test]
fn dirty_nested_modules_are_rejected_before_switching_managed_inputs() {
    let (f, _, _, _) = recursive_fixture();
    success(f.cli().arg("release").output().unwrap());
    let managed = f.app.join(".sources/checkouts/a");
    let nested = managed.join("dep/nested child");
    let parent_head = f.git(&managed, &["rev-parse", "HEAD"]);
    std::fs::write(nested.join("tracked.txt"), "source edit\n").unwrap();
    std::fs::write(nested.join("untracked.txt"), "source addition\n").unwrap();
    let output = f.cli().arg("release").output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    let changed = Path::new("dep").join("nested child");
    assert!(
        stderr.contains(changed.join("tracked.txt").to_string_lossy().as_ref())
            && stderr.contains(changed.join("untracked.txt").to_string_lossy().as_ref()),
        "{stderr}"
    );
    assert_eq!(f.git(&managed, &["rev-parse", "HEAD"]), parent_head);
    assert_eq!(
        std::fs::read_to_string(nested.join("tracked.txt")).unwrap(),
        "source edit\n"
    );
}

#[test]
fn removing_an_initialized_module_requires_clean_recreation_and_preserves_files_on_failure() {
    let (f, parent, _, _) = recursive_fixture();
    success(f.cli().arg("release").output().unwrap());
    let managed = f.app.join(".sources/checkouts/a");
    let old_pin = f.git(&managed, &["rev-parse", "HEAD"]);
    let artifact = managed.join("dep/artifact.txt");
    std::fs::write(&artifact, "old generated output\n").unwrap();
    f.git(&parent, &["rm", "--cached", "dep"]);
    std::fs::write(parent.join(".gitmodules"), "").unwrap();
    f.git(&parent, &["add", ".gitmodules"]);
    f.git(&parent, &["commit", "-qm", "remove module"]);
    let next = pin(&f, &parent);
    failure(
        f.cli().arg("release").output().unwrap(),
        "sources release --clean",
    );
    assert_eq!(f.git(&managed, &["rev-parse", "HEAD"]), old_pin);
    assert_eq!(
        std::fs::read_to_string(&artifact).unwrap(),
        "old generated output\n"
    );
    success(f.cli().args(["release", "--clean"]).output().unwrap());
    assert_eq!(f.git(&managed, &["rev-parse", "HEAD"]), next);
    assert!(!managed.join("dep").exists());
}

#[test]
fn offline_release_repairs_a_missing_nested_worktree_using_the_existing_cache() {
    let (f, _, _, nested) = recursive_fixture();
    success(f.cli().arg("release").output().unwrap());
    let expected = f.git(&nested, &["rev-parse", "HEAD"]);
    let worktree = f.app.join(".sources/checkouts/a/dep/nested child");
    std::fs::remove_dir_all(&worktree).unwrap();
    success(f.cli().args(["release", "--offline"]).output().unwrap());
    assert_eq!(f.git(&worktree, &["rev-parse", "HEAD"]), expected);
    assert_eq!(
        std::fs::read_to_string(worktree.join("tracked.txt")).unwrap(),
        "initial\n"
    );
}

#[test]
fn missing_worktree_repair_preserves_staged_changes_in_its_cached_git_directory() {
    let (f, _, _, _) = recursive_fixture();
    success(f.cli().arg("release").output().unwrap());
    let managed = f.app.join(".sources/checkouts/a");
    let worktree = managed.join("dep/nested child");
    std::fs::write(worktree.join("tracked.txt"), "staged source edit\n").unwrap();
    f.git(&worktree, &["add", "tracked.txt"]);
    std::fs::remove_dir_all(&worktree).unwrap();
    failure(
        f.cli().args(["release", "--offline"]).output().unwrap(),
        "staged source changes",
    );
    let output = f.git(
        &managed,
        &[
            "--git-dir=.git/modules/dep/modules/nested child",
            "--work-tree=.",
            "show",
            ":tracked.txt",
        ],
    );
    assert_eq!(output, "staged source edit");
}

#[cfg(unix)]
#[test]
fn recursive_module_names_and_paths_preserve_native_non_utf8_bytes() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let f = Fixture::new();
    let raw_probe = f.root.join(OsString::from_vec(b"raw-\xff-\xfe".to_vec()));
    if let Err(error) = std::fs::create_dir(&raw_probe) {
        eprintln!("Skipping raw-byte paths: fixture filesystem rejected the name: {error}");
        return;
    }
    std::fs::remove_dir(&raw_probe).unwrap();
    let _nested = f.repo("nested");
    let module = f.repo("module");
    let nested_name = OsString::from_vec(b"nested-\xfe".to_vec());
    success(
        f.command("git")
            .current_dir(&module)
            .args([
                "-c",
                "protocol.file.allow=always",
                "submodule",
                "add",
                "../nested",
            ])
            .arg(&nested_name)
            .output()
            .unwrap(),
    );
    f.git(&module, &["commit", "-qam", "nested input"]);
    let parent = f.repo("upstream");
    let module_name = OsString::from_vec(b"dep-\xff".to_vec());
    success(
        f.command("git")
            .current_dir(&parent)
            .args([
                "-c",
                "protocol.file.allow=always",
                "submodule",
                "add",
                "../module",
            ])
            .arg(&module_name)
            .output()
            .unwrap(),
    );
    f.git(&parent, &["commit", "-qam", "module input"]);
    f.configure(json!({"a": {"url": parent}}), json!({}));
    pin(&f, &parent);
    success(f.cli().arg("fetch").output().unwrap());
    success(f.cli().args(["release", "--offline"]).output().unwrap());
    let worktree = f
        .app
        .join(".sources/checkouts/a")
        .join(&module_name)
        .join(&nested_name);
    assert_eq!(
        std::fs::read_to_string(worktree.join("tracked.txt")).unwrap(),
        "initial\n"
    );
    success(f.cli().arg("check-release").output().unwrap());
    std::fs::remove_dir_all(&worktree).unwrap();
    success(f.cli().args(["release", "--offline"]).output().unwrap());
    assert_eq!(
        std::fs::read_to_string(worktree.join("tracked.txt")).unwrap(),
        "initial\n"
    );
}
