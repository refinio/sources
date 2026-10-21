mod common;
#[path = "common/turbo.rs"]
mod turbo_fixture;

use common::{Fixture, success};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Output;
use turbo_fixture::{install_turbo, pnpm, write_json};

struct Chain {
    f: Fixture,
    leaf: PathBuf,
    middle: PathBuf,
    app: PathBuf,
}

impl Chain {
    fn new() -> Self {
        let f = Fixture::new();
        let leaf = repository(&f, "leaf", None);
        let middle = repository(&f, "middle", Some("leaf"));
        let app = repository(&f, "top", Some("middle"));
        connect(&f, &middle, &leaf, "leaf", &["build"]);
        connect(&f, &app, &middle, "middle", &["build"]);
        Self {
            f,
            leaf,
            middle,
            app,
        }
    }

    fn run(&self) -> Output {
        success(
            self.f
                .cli()
                .current_dir(&self.app)
                .env("CI", "true")
                .env("TURBO_TELEMETRY_DISABLED", "1")
                .env("SOURCES_LEAF__BUILD", "forged-ancestor")
                .env("SOURCES_STALE__BUILD", "obsolete")
                .args(["turbo", "run", "build"])
                .output()
                .unwrap(),
        )
    }

    fn output(&self, root: &Path) -> Value {
        serde_json::from_slice(&std::fs::read(root.join("packages/pkg/dist/result.json")).unwrap())
            .unwrap()
    }

    fn executions(&self, root: &Path) -> String {
        std::fs::read_to_string(root.join("executions.log")).unwrap()
    }
}

fn repository(f: &Fixture, name: &str, dependency: Option<&str>) -> PathBuf {
    let root = f.repo(name);
    let upstream = f.repo(&format!("upstream-{name}"));
    f.git(
        &root,
        &["remote", "add", "origin", upstream.to_str().unwrap()],
    );
    write_json(
        &root.join("package.json"),
        &json!({"name":format!("{name}-root"),"private":true,"packageManager":"pnpm@10.15.0"}),
    );
    std::fs::write(
        root.join("pnpm-workspace.yaml"),
        "packages:\n  - packages/*\n",
    )
    .unwrap();
    let package = root.join("packages/pkg");
    std::fs::create_dir_all(&package).unwrap();
    let mut scripts = json!({"build":"node ../../build.cjs"});
    let mut build = json!({"inputs":["input.txt"],"outputs":["dist/**"]});
    let mut tasks = json!({});
    if let Some(dep) = dependency {
        build["env"] = json!([format!("SOURCES_{}__BUILD", dep.to_uppercase())]);
        build["dependsOn"] = json!(["prepare"]);
        tasks["prepare"] = json!({"cache":false});
        scripts["prepare"] = json!("node ../../prepare.cjs");
        let binary =
            serde_json::to_string(assert_cmd::cargo::cargo_bin!("sources").to_str().unwrap())
                .unwrap();
        std::fs::write(root.join("prepare.cjs"), format!("const {{spawnSync}} = require('node:child_process'); const result = spawnSync({binary}, ['task', '{dep}:build'], {{stdio:'inherit'}}); process.exit(result.status ?? 1);\n")).unwrap();
    }
    tasks["build"] = build;
    write_json(&root.join("turbo.json"), &json!({"tasks":tasks}));
    write_json(
        &package.join("package.json"),
        &json!({"name":name,"version":"1.0.0","scripts":scripts}),
    );
    std::fs::write(package.join("input.txt"), "alpha\n").unwrap();
    let input = dependency.map(|dep| format!("../../sources/{dep}/packages/pkg/dist/result.json"));
    std::fs::write(root.join("build.cjs"), format!(r#"
const fs = require('node:fs');
const input = {};
const value = input ? JSON.parse(fs.readFileSync(input)).value : fs.readFileSync('input.txt', 'utf8');
const env = Object.fromEntries(Object.entries(process.env).filter(([key]) => key.startsWith('SOURCES_')));
fs.mkdirSync('dist', {{recursive:true}});
fs.writeFileSync('dist/result.json', JSON.stringify({{value, env}}));
fs.appendFileSync('../../executions.log', 'build\n');
"#, serde_json::to_string(&input).unwrap())).unwrap();
    std::fs::write(
        root.join(".gitignore"),
        "node_modules/\ndist/\n.turbo/\n.sources/\nsources/\n.sources.local.conf\nexecutions.log\n",
    )
    .unwrap();
    success(
        pnpm(f, &root)
            .args(["install", "--offline", "--ignore-scripts"])
            .output()
            .unwrap(),
    );
    install_turbo(&root);
    f.git(&root, &["add", "."]);
    f.git(&root, &["commit", "-qm", "Turbo repository"]);
    root
}

fn connect(f: &Fixture, parent: &Path, child: &Path, package: &str, tasks: &[&str]) {
    let upstream = f.git(child, &["remote", "get-url", "origin"]);
    write_json(
        &parent.join(".sources.conf"),
        &json!({"version":1,"directory":"sources","sources":{
            package:{"url":upstream,"packages":{package:{"tasks":tasks}}}
        }}),
    );
    write_json(
        &parent.join(".sources.local.conf"),
        &json!({"sources":{package:child}}),
    );
    success(f.cli().current_dir(parent).arg("dev").output().unwrap());
}

#[test]
fn direct_edges_compose_hashes_execution_and_cache_restoration_across_three_repositories() {
    let t = Chain::new();
    // Disposing managed state must not break active development links or hashing.
    std::fs::remove_dir_all(t.middle.join(".sources")).unwrap();
    t.run();
    assert_eq!(t.output(&t.app)["value"], "alpha\n");
    let initial = t.output(&t.app)["env"]["SOURCES_MIDDLE__BUILD"].clone();
    let leaf_hash = t.output(&t.middle)["env"]["SOURCES_LEAF__BUILD"].clone();
    std::fs::write(t.leaf.join("packages/pkg/input.txt"), "beta\n").unwrap();
    t.run();
    for root in [&t.leaf, &t.middle, &t.app] {
        assert_eq!(t.output(root)["value"], "beta\n");
        assert_eq!(t.executions(root), "build\nbuild\n");
    }
    assert!(
        leaf_hash.is_string(),
        "child execution must receive its own source hash"
    );
    assert_ne!(leaf_hash, "forged-ancestor");
    assert_eq!(t.output(&t.middle)["env"].as_object().unwrap().len(), 1);
    assert_ne!(t.output(&t.app)["env"]["SOURCES_MIDDLE__BUILD"], initial);
    let current = t.output(&t.app);
    t.run();
    for root in [&t.leaf, &t.middle, &t.app] {
        assert_eq!(t.executions(root), "build\nbuild\n");
        std::fs::remove_dir_all(root.join("packages/pkg/dist")).unwrap();
    }
    t.run();
    assert_eq!(t.output(&t.app), current);
    for root in [&t.leaf, &t.middle, &t.app] {
        assert_eq!(t.output(root)["value"], "beta\n");
        assert_eq!(t.executions(root), "build\nbuild\n");
    }
}
