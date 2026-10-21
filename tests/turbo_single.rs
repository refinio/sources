mod common;
#[path = "common/turbo.rs"]
mod turbo_fixture;

use common::{Fixture, success};
use turbo_fixture::{install_turbo, pnpm, write_json};

#[test]
fn single_package_tasks_use_the_manifest_package_name_and_preserve_colons() {
    let f = Fixture::new();
    let child = f.repo("child");
    let upstream = f.repo("upstream");
    f.git(
        &child,
        &["remote", "add", "origin", upstream.to_str().unwrap()],
    );
    write_json(
        &child.join("package.json"),
        &serde_json::json!({
            "name":"@example/single","version":"1.0.0","packageManager":"pnpm@10.15.0",
            "scripts":{"build":"node task.cjs build","pack:app":"node task.cjs pack-app","pack":"node task.cjs unrequested"}
        }),
    );
    write_json(
        &child.join("turbo.json"),
        &serde_json::json!({"tasks":{
            "build":{"outputs":["dist/build.txt"]},
            "pack:app":{"dependsOn":["build"],"outputs":["dist/pack-app.txt"]},
            "pack":{"outputs":["dist/unrequested.txt"]}
        }}),
    );
    std::fs::write(child.join("task.cjs"), "const fs = require('node:fs'); fs.mkdirSync('dist', {recursive:true}); fs.writeFileSync('dist/' + process.argv[2] + '.txt', process.argv[2]);\n").unwrap();
    std::fs::write(child.join(".gitignore"), "node_modules/\ndist/\n.turbo/\n").unwrap();
    success(
        pnpm(&f, &child)
            .args(["install", "--offline", "--ignore-scripts"])
            .output()
            .unwrap(),
    );
    install_turbo(&child);
    f.git(&child, &["add", "."]);
    f.git(&child, &["commit", "-qm", "single-package Turbo fixture"]);
    f.configure(
        serde_json::json!({"shared":{"url":upstream}}),
        serde_json::json!({"shared":child}),
    );
    success(f.cli().arg("dev").output().unwrap());
    f.configure(
        serde_json::json!({"shared":{"url":upstream,"packages":{
            "@example/single":{"tasks":["build","pack:app"]}
        }}}),
        serde_json::json!({"shared":child}),
    );

    success(
        f.cli()
            .env("CI", "true")
            .env("TURBO_TELEMETRY_DISABLED", "1")
            .args(["task", "@example/single:build", "@example/single:pack:app"])
            .output()
            .unwrap(),
    );
    assert_eq!(
        std::fs::read_to_string(child.join("dist/build.txt")).unwrap(),
        "build"
    );
    assert_eq!(
        std::fs::read_to_string(child.join("dist/pack-app.txt")).unwrap(),
        "pack-app"
    );
    assert!(!child.join("dist/unrequested.txt").exists());
}

#[test]
fn monorepo_root_tasks_use_the_root_manifest_package_name() {
    let t = turbo_fixture::TurboFixture::new();
    let manifest_path = t.child.join("package.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
    manifest["scripts"] = serde_json::json!({"root:pack":"node task.cjs root-pack"});
    write_json(&manifest_path, &manifest);
    let turbo_path = t.child.join("turbo.json");
    let mut turbo: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&turbo_path).unwrap()).unwrap();
    turbo["tasks"]["//#root:pack"] = serde_json::json!({"outputs":["dist/root-pack.txt"]});
    write_json(&turbo_path, &turbo);
    t.f.configure(
        serde_json::json!({"shared":{"url":t.f.root.join("upstream"),"packages":{
            "child-fixture":{"tasks":["root:pack"]}
        }}}),
        serde_json::json!({"shared":t.child}),
    );
    success(
        t.cli()
            .args(["task", "child-fixture:root:pack"])
            .output()
            .unwrap(),
    );
    assert_eq!(
        std::fs::read_to_string(t.child.join("dist/root-pack.txt")).unwrap(),
        "root-pack\n"
    );
    assert!(!t.marker("protocol", "pack").exists());
    assert!(!t.marker("ui", "pack").exists());
}
