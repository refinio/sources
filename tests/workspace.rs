mod common;

#[cfg(unix)]
#[test]
fn case_only_source_rename_keeps_its_workspace_link_on_case_insensitive_filesystems() {
    use common::{Fixture, success};
    use serde_json::json;

    let f = Fixture::new();
    std::fs::write(f.root.join("CaseProbe"), "probe").unwrap();
    if !f.root.join("caseprobe").exists() {
        eprintln!("Case-insensitive filesystem required for this regression");
        return;
    }
    let dev = f.repo("dev");
    let upstream = "https://example.invalid/team/library";
    f.git(&dev, &["remote", "add", "origin", upstream]);
    f.configure(json!({"Foo": {"url": upstream}}), json!({"Foo": dev}));
    success(f.cli().arg("dev").output().unwrap());
    f.configure(json!({"foo": {"url": upstream}}), json!({"foo": dev}));
    success(f.cli().arg("dev").output().unwrap());

    let workspace = f.app.join("sources/foo");
    assert_eq!(
        workspace.canonicalize().unwrap(),
        dev.canonicalize().unwrap()
    );
    let entries = std::fs::read_dir(f.app.join("sources"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<Vec<_>>();
    assert_eq!(entries, vec![std::ffi::OsString::from("foo")]);
    assert_eq!(
        std::fs::read_to_string(dev.join("tracked.txt")).unwrap(),
        "initial\n"
    );
    let output = success(f.cli().args(["status", "--json"]).output().unwrap());
    let status: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(status["mode"], "dev");
}
