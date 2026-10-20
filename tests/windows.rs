#![cfg(windows)]

mod common;

use common::{Fixture, success};
use serde_json::json;

#[test]
fn native_windows_paths_support_recursive_release_and_child_execution() {
    let mut f = Fixture::new();
    let app = f.root.join("app with spaces é");
    std::fs::rename(&f.app, &app).unwrap();
    f.app = app;
    let module = f.repo("module with spaces ü");
    let module_pin = f.git(&module, &["rev-parse", "HEAD"]);
    let upstream = f.repo("upstream with spaces é");
    f.git(
        &upstream,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            "../module with spaces ü",
            "nested module",
        ],
    );
    f.git(&upstream, &["commit", "-qam", "add module"]);
    let pin = f.git(&upstream, &["rev-parse", "HEAD"]);
    let upstream_url = url::Url::from_file_path(&upstream).unwrap().to_string();
    f.git(
        &f.root,
        &[
            "-c",
            "protocol.file.allow=always",
            "clone",
            "--recurse-submodules",
            &upstream_url,
            "dev with spaces",
        ],
    );
    let dev = f.root.join("dev with spaces");
    f.configure(json!({"a":{"url":upstream_url}}), json!({"a":dev}));

    success(f.cli().arg("dev").output().unwrap());
    success(f.cli().args(["lock", "--check"]).output().unwrap());
    let before_lock = std::fs::read(f.app.join(".sources.lock")).unwrap();
    success(f.cli().args(["release", "--offline"]).output().unwrap());
    let active = f.app.join("sources/a");
    assert_eq!(f.git(&active, &["rev-parse", "HEAD"]), pin);
    assert_eq!(
        f.git(&active.join("nested module"), &["rev-parse", "HEAD"]),
        module_pin
    );
    assert_eq!(
        std::fs::read(active.join("nested module/tracked.txt")).unwrap(),
        b"initial\n"
    );
    success(f.cli().args(["status", "--json"]).output().unwrap());
    let child = f
        .cli()
        .args(["run", "a", "--", "cmd.exe", "/D", "/C", "exit /b 23"])
        .output()
        .unwrap();
    assert_eq!(child.status.code(), Some(23), "{child:?}");
    success(f.cli().args(["release", "--clean"]).output().unwrap());
    success(f.cli().arg("dev").output().unwrap());
    assert_eq!(
        std::fs::read(f.app.join(".sources.lock")).unwrap(),
        before_lock
    );
    assert_eq!(f.git(&dev, &["rev-parse", "HEAD"]), pin);
}

#[test]
fn removing_windows_directory_links_preserves_targets_and_repairs_dangling_links() {
    use std::os::windows::fs::{symlink_dir, symlink_file};

    let f = Fixture::new();
    let upstream = f.repo("upstream");
    let dev = f.repo("dev");
    let url = url::Url::from_file_path(upstream).unwrap().to_string();
    f.git(&dev, &["remote", "add", "origin", &url]);
    f.configure(json!({"a":{"url":url}}), json!({"a":dev}));
    let links = f.app.join("sources");
    std::fs::create_dir(&links).unwrap();
    let external = f.root.join("external");
    std::fs::create_dir(&external).unwrap();
    let valuable = external.join("valuable.txt");
    std::fs::write(&valuable, b"keep\n").unwrap();
    symlink_dir(&external, links.join("obsolete-dir")).unwrap();
    symlink_file(&valuable, links.join("obsolete-file")).unwrap();
    symlink_dir(f.root.join("missing"), links.join("obsolete-dangling")).unwrap();
    success(f.cli().arg("dev").output().unwrap());
    for name in ["obsolete-dir", "obsolete-file", "obsolete-dangling"] {
        assert!(std::fs::symlink_metadata(links.join(name)).is_err());
    }
    assert_eq!(std::fs::read(&valuable).unwrap(), b"keep\n");

    std::fs::remove_dir(links.join("a")).unwrap();
    symlink_dir(f.root.join("missing"), links.join("a")).unwrap();
    success(f.cli().arg("dev").output().unwrap());
    assert_eq!(
        links.join("a").canonicalize().unwrap(),
        dev.canonicalize().unwrap()
    );
    let disposable_link = f.app.join(".sources/disposable-link");
    symlink_dir(&external, &disposable_link).unwrap();
    success(f.cli().arg("lock").output().unwrap());
    success(f.cli().args(["release", "--clean"]).output().unwrap());
    assert!(std::fs::symlink_metadata(disposable_link).is_err());
    assert_eq!(std::fs::read(&valuable).unwrap(), b"keep\n");
    assert!(dev.join("tracked.txt").exists());
}
