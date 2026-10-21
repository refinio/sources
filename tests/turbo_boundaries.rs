mod common;
#[path = "common/turbo.rs"]
mod turbo_fixture;

use common::{failure, success};
use serde_json::json;
use std::path::Path;
use turbo_fixture::{TurboFixture, install_turbo, pnpm, write_json};

fn remove_link(path: &Path) {
    #[cfg(unix)]
    std::fs::remove_file(path).unwrap();
    #[cfg(windows)]
    std::fs::remove_dir(path).unwrap();
}

fn directory_link(target: &Path, path: &Path) {
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, path).unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_dir(target, path).unwrap();
}

#[test]
fn task_rejects_missing_links_and_mismatched_repository_identity_before_execution() {
    let t = TurboFixture::new();
    remove_link(&t.f.app.join("sources/shared"));
    failure(
        t.cli()
            .args(["task", "@example/protocol:pack"])
            .output()
            .unwrap(),
        "missing workspace link",
    );
    success(t.cli().arg("dev").output().unwrap());
    t.f.git(
        &t.child,
        &[
            "remote",
            "set-url",
            "origin",
            "https://example.invalid/other/repository",
        ],
    );
    failure(
        t.cli()
            .args(["task", "@example/protocol:pack"])
            .output()
            .unwrap(),
        "repository mismatch",
    );
    assert!(!t.marker("protocol", "pack").exists());
}

#[test]
fn task_does_not_fall_back_to_an_available_dev_checkout_for_registry_selection() {
    let t = TurboFixture::new();
    let path = t.f.app.join(".sources.conf");
    let mut config: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    config["sources"]["shared"]["pnpm"] =
        json!({"@example/protocol":{"path":"packages/protocol","version":"1.2.3"}});
    write_json(&path, &config);
    write_json(
        &t.f.app.join("package.json"),
        &json!({"name":"parent","private":true}),
    );
    std::fs::write(
        t.f.app.join("pnpm-workspace.yaml"),
        "packages: []\noverrides:\n  '@example/protocol': 1.2.3\n",
    )
    .unwrap();
    remove_link(&t.f.app.join("sources/shared"));
    failure(
        t.cli()
            .args(["task", "@example/protocol:pack"])
            .output()
            .unwrap(),
        "published package selection has no active source checkout",
    );
    assert!(t.child.join("node_modules/.bin").is_dir());
    assert!(!t.marker("protocol", "pack").exists());
}

#[test]
fn task_warns_for_managed_source_edits_before_and_after_child_execution() {
    let t = TurboFixture::new();
    let managed = t.f.app.join(".sources/checkouts/shared");
    std::fs::create_dir_all(managed.parent().unwrap()).unwrap();
    t.f.git(
        &t.f.root,
        &[
            "clone",
            "--no-hardlinks",
            t.child.to_str().unwrap(),
            managed.to_str().unwrap(),
        ],
    );
    t.f.git(
        &managed,
        &[
            "remote",
            "set-url",
            "origin",
            t.f.root.join("upstream").to_str().unwrap(),
        ],
    );
    install_turbo(&managed);
    std::fs::write(
        managed.join("tracked.txt"),
        "managed source edit before task\n",
    )
    .unwrap();
    let task_script = managed.join("task.cjs");
    let mut script = std::fs::read_to_string(&task_script).unwrap();
    script.push_str("fs.writeFileSync(require('node:path').join(__dirname, 'after-task.txt'), 'managed edit after task\\n');\n");
    std::fs::write(task_script, script).unwrap();
    let link = t.f.app.join("sources/shared");
    remove_link(&link);
    directory_link(&managed, &link);
    let output = success(
        t.cli()
            .args(["task", "@example/protocol:pack"])
            .output()
            .unwrap(),
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.matches("tracked.txt").count() >= 2,
        "source changes must be reported before and after task execution: {stderr}"
    );
    assert!(
        stderr.contains("after-task.txt"),
        "post-task source changes were not reported: {stderr}"
    );
    assert!(managed.join("packages/protocol/dist/pack.txt").is_file());
    assert!(!t.marker("protocol", "pack").exists());
}

fn broken_tool(t: &TurboFixture) {
    let bin = t.child.join("node_modules/.bin");
    let native = bin.join(if cfg!(windows) { "turbo.exe" } else { "turbo" });
    if native.exists() {
        std::fs::remove_file(native).unwrap();
    }
    std::fs::write(
        bin.join("broken-turbo.cjs"),
        r#"
const fs = require('node:fs');
if (process.argv.includes('--dry=json')) {
  process.stdout.write(fs.readFileSync('broken-response.json'));
} else {
  fs.writeFileSync('broken-tool-executed', 'must not execute after invalid metadata');
}
"#,
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let path = bin.join("turbo");
        std::fs::write(&path, "#!/usr/bin/env node\nrequire(require('node:path').join(__dirname, 'broken-turbo.cjs'));\n").unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    #[cfg(windows)]
    std::fs::write(
        bin.join("turbo.cmd"),
        "@node \"%~dp0broken-turbo.cjs\" %*\r\n",
    )
    .unwrap();
}

#[test]
fn task_rejects_missing_bootstrap_and_broken_external_turbo_metadata() {
    let t = TurboFixture::new();
    let native =
        t.child
            .join("node_modules/.bin")
            .join(if cfg!(windows) { "turbo.exe" } else { "turbo" });
    std::fs::remove_file(native).unwrap();
    failure(
        t.cli()
            .args(["task", "@example/protocol:pack"])
            .output()
            .unwrap(),
        "missing locally installed Turbo",
    );
    broken_tool(&t);
    for (response, message) in [
        ("not JSON".to_owned(), "invalid or unsupported Turbo dry-run JSON"),
        (json!({"version":"99","monorepo":true,"tasks":[]}).to_string(), "unsupported Turbo metadata version"),
        (json!({"version":"1","monorepo":true,"tasks":[{"taskId":"@example/protocol#pack","hash":[],"command":"node task.cjs"}]}).to_string(), "invalid or unsupported Turbo dry-run JSON"),
        (json!({"version":"1","monorepo":true,"tasks":[]}).to_string(), "must contain exactly one selected task"),
    ] {
        std::fs::write(t.child.join("broken-response.json"), response).unwrap();
        failure(
            t.cli().args(["task", "@example/protocol:pack"]).output().unwrap(),
            message,
        );
        assert!(!t.child.join("broken-tool-executed").exists());
        assert!(!t.marker("protocol", "pack").exists());
    }
}

#[test]
fn turbo_does_not_launch_parent_after_child_metadata_failure() {
    let t = TurboFixture::new();
    t.parent();
    broken_tool(&t);
    std::fs::write(
        t.child.join("broken-response.json"),
        "broken child metadata",
    )
    .unwrap();
    failure(
        t.cli().args(["turbo", "run", "inspect"]).output().unwrap(),
        "invalid or unsupported Turbo dry-run JSON",
    );
    assert!(!t.f.app.join("observed-web.json").exists());
    assert!(!t.f.app.join("observed-other.json").exists());
    assert!(!t.child.join("broken-tool-executed").exists());
}

#[test]
fn turbo_requires_the_parents_own_local_installation_before_launch() {
    let t = TurboFixture::new();
    t.parent();
    let native =
        t.f.app
            .join("node_modules/.bin")
            .join(if cfg!(windows) { "turbo.exe" } else { "turbo" });
    std::fs::remove_file(native).unwrap();
    let output = t.cli().args(["turbo", "run", "inspect"]).output().unwrap();
    assert!(String::from_utf8_lossy(&output.stderr).contains(t.f.app.to_str().unwrap()));
    failure(output, "missing locally installed Turbo");
    assert!(!t.f.app.join("observed-web.json").exists());
    assert!(!t.marker("protocol", "build").exists());
}

#[test]
fn turbo_forwards_help_version_and_cwd_flags_after_sources_directory_selection() {
    let t = TurboFixture::new();
    t.parent();
    for flag in ["--help", "--version"] {
        let expected = success(
            pnpm(&t.f, &t.f.app)
                .args(["exec", "turbo", flag])
                .output()
                .unwrap(),
        );
        let actual = success(
            t.cli()
                .current_dir(&t.f.root)
                .arg("-C")
                .arg(&t.f.app)
                .args(["turbo", flag])
                .output()
                .unwrap(),
        );
        assert_eq!(
            actual.stdout, expected.stdout,
            "parent flag {flag} was not forwarded"
        );
    }
    success(
        t.cli()
            .current_dir(&t.f.root)
            .arg("-C")
            .arg(&t.f.app)
            .args(["turbo", "--cwd=.", "run", "inspect", "--filter=@app/web"])
            .output()
            .unwrap(),
    );
    let observed: serde_json::Value =
        serde_json::from_slice(&std::fs::read(t.f.app.join("observed-web.json")).unwrap()).unwrap();
    assert_eq!(
        std::fs::canonicalize(observed["cwd"].as_str().unwrap()).unwrap(),
        std::fs::canonicalize(t.f.app.join("packages/web")).unwrap()
    );
    assert!(!t.f.app.join("observed-other.json").exists());
}
