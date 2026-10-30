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
    // A task literally named content still has a native-hash input distinct from
    // its optional CONTENT artifact input. Declaring the latter does not require it.
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(child.join("package.json")).unwrap()).unwrap();
    manifest["scripts"]["content"] = serde_json::json!("node task.cjs content");
    write_json(&child.join("package.json"), &manifest);
    let mut turbo: serde_json::Value =
        serde_json::from_slice(&std::fs::read(child.join("turbo.json")).unwrap()).unwrap();
    turbo["tasks"]["content"] = serde_json::json!({"outputs":["dist/content.txt"]});
    write_json(&child.join("turbo.json"), &turbo);
    f.configure(serde_json::json!({"shared":{"url":upstream,"packages":{
        "@example/single":{"tasks":["content"],"content":{"content":{"artifacts":[{"path":"deliberately-missing-content"}]}}}
    }}}), serde_json::json!({"shared":child}));
    write_json(
        &f.app.join("package.json"),
        &serde_json::json!({"name":"parent-single","packageManager":"pnpm@10.15.0","scripts":{"inspect":"node inspect.cjs"}}),
    );
    write_json(
        &f.app.join("turbo.json"),
        &serde_json::json!({"tasks":{"inspect":{"cache":false,"env":["SOURCES_EXAMPLE_SINGLE__CONTENT"]}}}),
    );
    std::fs::write(f.app.join("inspect.cjs"), "require('node:fs').writeFileSync('observed-content-hash.txt',process.env.SOURCES_EXAMPLE_SINGLE__CONTENT);\n").unwrap();
    success(
        pnpm(&f, &f.app)
            .args(["install", "--offline", "--ignore-scripts"])
            .output()
            .unwrap(),
    );
    install_turbo(&f.app);
    f.git(&f.app, &["init"]);
    f.git(&f.app, &["add", "."]);
    f.git(&f.app, &["commit", "-qm", "Native content task input"]);
    let expected = success(
        pnpm(&f, &child)
            .args(["exec", "turbo", "run", "content", "--dry=json"])
            .output()
            .unwrap(),
    );
    let expected: serde_json::Value = serde_json::from_slice(&expected.stdout).unwrap();
    success(
        f.cli()
            .env("CI", "true")
            .args(["turbo", "run", "inspect"])
            .output()
            .unwrap(),
    );
    assert_eq!(
        std::fs::read_to_string(f.app.join("observed-content-hash.txt")).unwrap(),
        expected["tasks"][0]["hash"].as_str().unwrap()
    );
    assert!(
        !child.join("dist/content.txt").exists(),
        "hash-only input must not execute the producer or require its artifact contract"
    );
}
