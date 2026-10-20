mod common;

#[cfg(unix)]
#[test]
fn check_warns_about_source_edits_inside_a_module_with_a_non_utf8_path() {
    use common::{Fixture, success};
    use serde_json::json;
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let f = Fixture::new();
    let raw_probe = f.root.join(OsString::from_vec(b"raw-\xff-\xfe".to_vec()));
    if let Err(error) = std::fs::create_dir(&raw_probe) {
        eprintln!("Skipping raw-byte paths: fixture filesystem rejected the name: {error}");
        return;
    }
    std::fs::remove_dir(&raw_probe).unwrap();
    let module = f.repo("module");
    let dev = f.repo("development");
    let name = OsString::from_vec(b"module-\xff".to_vec());
    success(
        f.command("git")
            .current_dir(&dev)
            .args(["-c", "protocol.file.allow=always", "submodule", "add", "--"])
            .arg(&module)
            .arg(&name)
            .output()
            .unwrap(),
    );
    f.git(&dev, &["commit", "-qam", "module"]);
    f.git(
        &dev,
        &[
            "remote",
            "add",
            "origin",
            "https://example.invalid/team/repo",
        ],
    );
    f.configure(
        json!({"a": {"url": "https://example.invalid/team/repo"}}),
        json!({"a": dev}),
    );
    std::fs::write(dev.join(&name).join("tracked.txt"), "module edit\n").unwrap();
    let output = success(f.cli().arg("check").output().unwrap());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("module-") && stderr.contains("tracked.txt"),
        "{stderr}"
    );
}
