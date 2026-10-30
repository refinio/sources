mod common;
#[path = "common/turbo.rs"]
mod turbo_fixture;

use common::{Fixture, success};
use serde_json::{Value, json};
use std::collections::BTreeMap;
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

fn edit_json(path: &Path, edit: impl FnOnce(&mut Value)) {
    let mut value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    edit(&mut value);
    write_json(path, &value);
}

#[test]
fn only_consumed_hashes_in_transitive_tasks_are_resolved_with_turbo_env_patterns() {
    let t = Chain::new();
    // The configured but nonexistent task must never be queried. Its excluded
    // hash also must not cause recursion or prevent the selected build.
    connect(
        &t.f,
        &t.middle,
        &t.leaf,
        "leaf",
        &["build", "other", "missing"],
    );
    edit_json(&t.leaf.join("turbo.json"), |config| {
        config["tasks"]["other"] = json!({"inputs":["other.txt"]});
    });
    edit_json(&t.leaf.join("packages/pkg/package.json"), |manifest| {
        manifest["scripts"]["other"] = json!("node -e \"throw new Error('must not execute')\"");
    });
    std::fs::write(t.leaf.join("packages/pkg/other.txt"), "unrelated alpha\n").unwrap();
    edit_json(&t.middle.join("turbo.json"), |config| {
        config["tasks"]["build"]
            .as_object_mut()
            .unwrap()
            .remove("env");
        config["tasks"]["prepare"]["env"] = json!([
            "SOURCES_*",
            "!SOURCES_LEAF__OTHER",
            "!SOURCES_LEAF__MISSING"
        ]);
    });
    t.run();
    let initial = t.output(&t.app);
    std::fs::write(t.leaf.join("packages/pkg/other.txt"), "unrelated beta\n").unwrap();
    t.run();
    assert_eq!(t.output(&t.app), initial);
    for root in [&t.leaf, &t.middle, &t.app] {
        assert_eq!(t.executions(root), "build\n");
    }
    std::fs::write(t.leaf.join("packages/pkg/input.txt"), "beta\n").unwrap();
    t.run();
    assert_eq!(t.output(&t.app)["value"], "beta\n");
    assert_ne!(t.output(&t.app)["env"], initial["env"]);
}

#[test]
fn repository_cycle_through_independent_foundation_task_builds_and_restores_cache() {
    let t = Chain::new();
    // ONE application -> Fotos core -> ONE foundation. Both ONE tasks are
    // configured in the parent's same source group, but only application uses Fotos.
    connect(&t.f, &t.leaf, &t.middle, "middle", &["build"]);
    let application = t.leaf.join("packages/application");
    std::fs::create_dir_all(&application).unwrap();
    write_json(
        &application.join("package.json"),
        &json!({
            "name":"application", "version":"1.0.0", "scripts":{
                "build":"node build.cjs", "prepare":"node ../../prepare.cjs"
            }
        }),
    );
    std::fs::copy(t.middle.join("build.cjs"), application.join("build.cjs")).unwrap();
    let script = std::fs::read_to_string(application.join("build.cjs"))
        .unwrap()
        .replace("sources/leaf", "sources/middle")
        .replace("../../executions.log", "../../application-executions.log");
    std::fs::write(application.join("build.cjs"), script).unwrap();
    let script = std::fs::read_to_string(t.middle.join("prepare.cjs"))
        .unwrap()
        .replace("leaf:build", "middle:build");
    std::fs::write(t.leaf.join("prepare.cjs"), script).unwrap();
    edit_json(&t.leaf.join("turbo.json"), |config| {
        config["tasks"]["application#build"] = json!({
            "env":["SOURCES_MIDDLE__BUILD"], "dependsOn":["prepare"], "outputs":["dist/**"]
        });
        config["tasks"]["prepare"] = json!({"cache":false});
    });
    connect(&t.f, &t.app, &t.leaf, "application", &["build"]);
    edit_json(&t.app.join(".sources.conf"), |config| {
        config["sources"]["application"]["packages"]["leaf"] = json!({"tasks":["build"]});
    });
    edit_json(&t.app.join("turbo.json"), |config| {
        config["tasks"]["build"]["env"] = json!(["SOURCES_APPLICATION__BUILD"]);
    });
    let script = std::fs::read_to_string(t.app.join("prepare.cjs"))
        .unwrap()
        .replace("middle:build", "application:build");
    std::fs::write(t.app.join("prepare.cjs"), script).unwrap();
    let script = std::fs::read_to_string(t.app.join("build.cjs"))
        .unwrap()
        .replace(
            "sources/middle/packages/pkg",
            "sources/application/packages/application",
        );
    std::fs::write(t.app.join("build.cjs"), script).unwrap();
    t.run();
    assert_eq!(t.output(&t.app)["value"], "alpha\n");
    std::fs::write(t.leaf.join("packages/pkg/input.txt"), "beta\n").unwrap();
    t.run();
    assert_eq!(t.output(&t.app)["value"], "beta\n");
    let current = t.output(&t.app);
    for directory in [
        t.leaf.join("packages/pkg/dist"),
        application.join("dist"),
        t.middle.join("packages/pkg/dist"),
        t.app.join("packages/pkg/dist"),
    ] {
        std::fs::remove_dir_all(directory).unwrap();
    }
    t.run();
    assert_eq!(t.output(&t.app), current);
    for root in [&t.leaf, &t.middle, &t.app] {
        assert_eq!(t.output(root)["value"], "beta\n");
        assert_eq!(t.executions(root), "build\nbuild\n");
    }
    assert_eq!(
        std::fs::read_to_string(t.leaf.join("application-executions.log")).unwrap(),
        "build\nbuild\n"
    );
}

#[test]
fn genuine_task_hash_cycle_reports_the_task_path_before_any_execution() {
    let t = Chain::new();
    connect(&t.f, &t.leaf, &t.middle, "middle", &["build"]);
    edit_json(&t.leaf.join("turbo.json"), |config| {
        config["tasks"]["build"]["env"] = json!(["SOURCES_MIDDLE__BUILD"]);
    });
    for arguments in [vec!["turbo", "run", "build"], vec!["task", "middle:build"]] {
        let output =
            t.f.cli()
                .current_dir(&t.app)
                .env("CI", "true")
                .env("TURBO_TELEMETRY_DISABLED", "1")
                .args(arguments)
                .output()
                .unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("middle#build") && stderr.contains("leaf#build"),
            "{stderr}"
        );
        assert!(
            stderr.contains("remove a cyclic SOURCES_* task env dependency"),
            "{stderr}"
        );
        common::failure(output, "Turbo source task hash cycle");
        for root in [&t.leaf, &t.middle, &t.app] {
            assert!(!root.join("executions.log").exists());
        }
    }
}

#[test]
fn content_edges_prepare_transitively_and_keep_child_owned_wildcard_env() {
    let t = Chain::new();
    // Content may be declared only in the child. A later invalid direct selector
    // must still prevent all preparation, including that child's upstream task.
    edit_json(&t.middle.join(".sources.conf"), |config| {
        config["sources"]["leaf"]["packages"]["leaf"]["content"] = json!({
            "build":{"artifacts":[{"path":"packages/pkg/dist/result.json"}]}
        });
    });
    edit_json(&t.middle.join("turbo.json"), |config| {
        config["tasks"]["build"]["env"] = json!(["SOURCES_LEAF__BUILD__CONTENT"]);
        config["tasks"]["missing"] = json!({});
    });
    connect(&t.f, &t.app, &t.middle, "middle", &["build", "missing"]);
    common::failure(
        t.f.cli()
            .current_dir(&t.app)
            .env("CI", "true")
            .args(["task", "middle:build", "middle:missing"])
            .output()
            .unwrap(),
        "has no executable package script",
    );
    for root in [&t.leaf, &t.middle, &t.app] {
        assert!(
            !root.join("executions.log").exists(),
            "must validate all selected tasks before upstream preparation"
        );
    }
    connect(&t.f, &t.app, &t.middle, "middle", &["build"]);
    for (root, dep) in [(&t.middle, "leaf"), (&t.app, "middle")] {
        edit_json(&root.join(".sources.conf"), |config| {
            config["sources"][dep]["packages"][dep]["content"] = json!({
                "build":{"artifacts":[{"path":"packages/pkg/dist/result.json"}]}
            });
        });
        edit_json(&root.join("turbo.json"), |config| {
            config["tasks"]["build"]["env"] = json!([
                "SOURCES_*",
                format!("!SOURCES_{}__BUILD", dep.to_uppercase())
            ]);
        });
    }
    let script = std::fs::read_to_string(t.leaf.join("build.cjs"))
        .unwrap()
        .replace(
            "fs.readFileSync('input.txt', 'utf8')",
            "fs.readFileSync('input.txt', 'utf8').split('\\n')[0] + '\\n'",
        );
    std::fs::write(t.leaf.join("build.cjs"), script).unwrap();
    t.run();
    let first = t.output(&t.app);
    assert_eq!(first["value"], "alpha\n");
    assert!(
        t.output(&t.middle)["env"]["SOURCES_LEAF__BUILD__CONTENT"]
            .as_str()
            .unwrap()
            .starts_with("sha256:")
    );
    assert_eq!(t.output(&t.middle)["env"].as_object().unwrap().len(), 1);
    std::fs::write(
        t.leaf.join("packages/pkg/input.txt"),
        "alpha\ncomment only\n",
    )
    .unwrap();
    t.run();
    assert_eq!(t.output(&t.app), first);
    assert_eq!(t.executions(&t.leaf), "build\nbuild\n");
    for root in [&t.middle, &t.app] {
        assert_eq!(t.executions(root), "build\n");
    }
    std::fs::write(t.leaf.join("packages/pkg/input.txt"), "beta\n").unwrap();
    t.run();
    assert_eq!(t.output(&t.app)["value"], "beta\n");
    // A task cycle must be diagnosed before executing any node in that cycle.
    connect(&t.f, &t.leaf, &t.middle, "middle", &["build"]);
    edit_json(&t.leaf.join(".sources.conf"), |config| {
        config["sources"]["middle"]["packages"]["middle"]["content"] =
            json!({"build":{"artifacts":[{"path":"packages/pkg/dist/result.json"}]}});
    });
    edit_json(&t.leaf.join("turbo.json"), |config| {
        config["tasks"]["build"]["env"] = json!(["SOURCES_MIDDLE__BUILD__CONTENT"]);
    });
    let before = [&t.leaf, &t.middle, &t.app].map(|root| t.executions(root));
    common::failure(
        t.f.cli()
            .current_dir(&t.app)
            .env("CI", "true")
            .args(["turbo", "run", "build"])
            .output()
            .unwrap(),
        "Turbo source task hash cycle",
    );
    assert_eq!(
        [&t.leaf, &t.middle, &t.app].map(|root| t.executions(root)),
        before
    );

    // A different task in the caller is a valid producer, not a repository cycle.
    let foundation = t.app.join("packages/foundation");
    std::fs::create_dir_all(&foundation).unwrap();
    write_json(
        &foundation.join("package.json"),
        &json!({"name":"foundation","version":"1.0.0","scripts":{"build":"node build.cjs"}}),
    );
    std::fs::write(
        foundation.join("build.cjs"),
        r#"
const fs = require('node:fs');
fs.mkdirSync('dist',{recursive:true});
fs.writeFileSync('dist/result.json',JSON.stringify({value:'foundation\n'}));
fs.appendFileSync('../../foundation-executions.log','build\n');
"#,
    )
    .unwrap();
    edit_json(&t.app.join("turbo.json"), |config| {
        config["tasks"]["foundation#build"] =
            json!({"env":[],"dependsOn":[],"outputs":["dist/**"]});
    });
    connect(&t.f, &t.leaf, &t.app, "foundation", &["build"]);
    edit_json(&t.leaf.join(".sources.conf"), |config| {
        config["sources"]["foundation"]["packages"]["foundation"]["content"] =
            json!({"build":{"artifacts":[{"path":"packages/foundation/dist"}]}});
    });
    edit_json(&t.leaf.join("turbo.json"), |config| {
        config["tasks"]["build"]["env"] = json!(["SOURCES_FOUNDATION__BUILD__CONTENT"]);
    });
    let script = std::fs::read_to_string(t.leaf.join("build.cjs"))
        .unwrap()
        .replace(
            "const input = null;",
            "const input = '../../sources/foundation/packages/foundation/dist/result.json';",
        );
    std::fs::write(t.leaf.join("build.cjs"), script).unwrap();
    for restore in [false, true] {
        if restore {
            for path in [
                foundation.join("dist"),
                t.leaf.join("packages/pkg/dist"),
                t.middle.join("packages/pkg/dist"),
                t.app.join("packages/pkg/dist"),
            ] {
                std::fs::remove_dir_all(path).unwrap();
            }
        }
        success(
            t.f.cli()
                .current_dir(&t.app)
                .env("CI", "true")
                .args(["turbo", "run", "build", "--filter=top"])
                .output()
                .unwrap(),
        );
        assert_eq!(t.output(&t.app)["value"], "foundation\n");
        assert_eq!(
            std::fs::read_to_string(t.app.join("foundation-executions.log")).unwrap(),
            "build\n"
        );
    }
}

#[test]
fn content_group_refresh_preserves_inputs_generated_for_all_selected_tasks() {
    let t = Chain::new();
    edit_json(&t.leaf.join("packages/pkg/package.json"), |manifest| {
        manifest["scripts"]["other"] = json!("node ../../other.cjs");
    });
    edit_json(&t.leaf.join("turbo.json"), |config| {
        config["tasks"]["other"] = json!({"inputs":["other.txt"],"outputs":["other/**"]});
    });
    std::fs::write(t.leaf.join("packages/pkg/other.txt"), "delta\n").unwrap();
    std::fs::write(
        t.leaf.join("other.cjs"),
        r#"
const fs = require('node:fs');
fs.mkdirSync('other', {recursive:true});
fs.writeFileSync('other/result.json', JSON.stringify({value:fs.readFileSync('other.txt','utf8')}));
"#,
    )
    .unwrap();
    connect(&t.f, &t.middle, &t.leaf, "leaf", &["build", "other"]);
    success(
        t.f.cli()
            .current_dir(&t.middle)
            .env("CI", "true")
            .args(["task", "leaf:build", "leaf:other"])
            .output()
            .unwrap(),
    );
    edit_json(&t.middle.join("package.json"), |manifest| {
        manifest["dependencies"] = json!({"leaf":"file:./sources/leaf/packages/pkg"});
        manifest["scripts"] = json!({"postinstall":"node generate.cjs"});
    });
    // One ordinary native lifecycle generates inputs for both tasks. Each input
    // depends on its own upstream content key; neither may disappear on refresh.
    std::fs::write(t.middle.join("generate.cjs"), r#"
const fs = require('node:fs');
fs.mkdirSync('generated', {recursive:true});
fs.writeFileSync('generated/env.json',JSON.stringify(Object.fromEntries(Object.entries(process.env).filter(([key])=>key.startsWith('SOURCES_')))));
for (const [task, directory] of [['build','dist'],['other','other']]) {
  const key = 'SOURCES_LEAF__' + task.toUpperCase() + '__CONTENT';
  const value = process.env[key] ? JSON.parse(fs.readFileSync('node_modules/leaf/'+directory+'/result.json')).value : 'missing';
  fs.writeFileSync('generated/'+task+'.json', JSON.stringify({value}));
}
"#).unwrap();
    edit_json(&t.middle.join("packages/pkg/package.json"), |manifest| {
        manifest["scripts"] =
            json!({"build":"node ../../consume.cjs build","other":"node ../../consume.cjs other"});
    });
    std::fs::write(
        t.middle.join("consume.cjs"),
        r#"
const fs = require('node:fs');
const task = process.argv[2];
fs.mkdirSync('dist', {recursive:true});
fs.copyFileSync('../../generated/'+task+'.json', 'dist/'+task+'.json');
fs.appendFileSync('../../group-executions.log', task+'\n');
"#,
    )
    .unwrap();
    edit_json(&t.middle.join("turbo.json"), |config| {
        for task in ["build", "other"] {
            config["tasks"][task] = json!({
                "inputs":[format!("../../generated/{task}.json")],
                "env":[format!("SOURCES_LEAF__{}__CONTENT",task.to_uppercase())],
                "outputs":[format!("dist/{task}.json")]
            });
        }
    });
    success(
        pnpm(&t.f, &t.middle)
            .args(["install", "--offline"])
            .output()
            .unwrap(),
    );
    install_turbo(&t.middle);
    let lock = std::fs::read(t.middle.join("pnpm-lock.yaml")).unwrap();
    edit_json(&t.middle.join(".sources.conf"), |config| {
        for (task, directory) in [("build", "dist"), ("other", "other")] {
            config["sources"]["leaf"]["packages"]["leaf"]["content"][task] = json!({
                "artifacts":[{"path":format!("packages/pkg/{directory}"),"installed":format!("node_modules/leaf/{directory}")}]
            });
        }
    });
    connect(&t.f, &t.app, &t.middle, "middle", &["build", "other"]);
    let run = || {
        success(
            t.f.cli()
                .current_dir(&t.app)
                .env("CI", "true")
                .env("TURBO_TELEMETRY_DISABLED", "1")
                .args(["task", "middle:build", "middle:other"])
                .output()
                .unwrap(),
        );
        assert_eq!(
            std::fs::read(t.middle.join("pnpm-lock.yaml")).unwrap(),
            lock
        );
        for (task, expected) in [
            (
                "build",
                std::fs::read_to_string(t.leaf.join("packages/pkg/input.txt")).unwrap(),
            ),
            ("other", "delta\n".to_owned()),
        ] {
            let path = t.middle.join(format!("packages/pkg/dist/{task}.json"));
            let actual: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
            assert_eq!(
                actual["value"], expected,
                "{task} must consume freshly generated input from the combined snapshot"
            );
        }
    };
    run();
    run();
    std::fs::write(t.leaf.join("packages/pkg/input.txt"), "beta\n").unwrap();
    run();
    let executions = std::fs::read_to_string(t.middle.join("group-executions.log")).unwrap();
    assert_eq!(
        executions.lines().filter(|line| *line == "build").count(),
        2
    );
    assert_eq!(
        executions.lines().filter(|line| *line == "other").count(),
        1
    );

    // Discovering all required closures first allows shared tasks to settle in
    // the same owner frontier, even when first is listed before middle directly.
    let generator = std::fs::read_to_string(t.middle.join("generate.cjs"))
        .unwrap()
        .replace(
            "JSON.stringify({value})",
            "JSON.stringify({value, otherPresent:!!process.env.SOURCES_LEAF__OTHER__CONTENT})",
        );
    std::fs::write(t.middle.join("generate.cjs"), generator).unwrap();
    let first = repository(&t.f, "first", Some("middle"));
    connect(&t.f, &first, &t.middle, "middle", &["build"]);
    write_json(
        &t.app.join(".sources.conf"),
        &json!({"version":1,"directory":"sources","sources":{
            "a-first":{"url":t.f.git(&first,&["remote","get-url","origin"]),"packages":{"first":{"tasks":["build"]}}},
            "z-middle":{"url":t.f.git(&t.middle,&["remote","get-url","origin"]),"packages":{"middle":{"tasks":["build","other"]}}}
        }}),
    );
    write_json(
        &t.app.join(".sources.local.conf"),
        &json!({"sources":{"a-first":first,"z-middle":t.middle}}),
    );
    success(t.f.cli().current_dir(&t.app).arg("dev").output().unwrap());
    edit_json(&t.app.join("turbo.json"), |config| {
        config["tasks"]["build"] = json!({"env":["SOURCES_FIRST__BUILD"]});
    });
    edit_json(&t.app.join("packages/pkg/package.json"), |manifest| {
        manifest["scripts"]["build"] = json!(
            "node -e \"require('node:fs').writeFileSync('../../parent-started',process.env.SOURCES_FIRST__BUILD)\""
        );
    });
    success(
        t.f.cli()
            .current_dir(&t.app)
            .env("CI", "true")
            .args(["turbo", "run", "build"])
            .output()
            .unwrap(),
    );
    let env: BTreeMap<String, String> =
        serde_json::from_slice(&std::fs::read(t.middle.join("generated/env.json")).unwrap())
            .unwrap();
    let middle = success(
        pnpm(&t.f, &t.middle)
            .envs(env)
            .args(["exec", "turbo", "run", "middle#build", "--dry=json"])
            .output()
            .unwrap(),
    );
    let middle: Value = serde_json::from_slice(&middle.stdout).unwrap();
    let hash = middle["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|task| task["taskId"] == "middle#build")
        .unwrap()["hash"]
        .as_str()
        .unwrap();
    let expected = success(
        pnpm(&t.f, &first)
            .env("SOURCES_MIDDLE__BUILD", hash)
            .args(["exec", "turbo", "run", "first#build", "--dry=json"])
            .output()
            .unwrap(),
    );
    let expected: Value = serde_json::from_slice(&expected.stdout).unwrap();
    let hash = expected["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|task| task["taskId"] == "first#build")
        .unwrap()["hash"]
        .as_str()
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(t.app.join("parent-started")).unwrap(),
        hash
    );
}

#[cfg(unix)]
#[test]
fn filtered_parent_graph_uses_only_required_batched_foreign_closures() {
    use std::os::unix::fs::PermissionsExt;
    let t = Chain::new();
    for root in [&t.middle, &t.leaf] {
        edit_json(&root.join("packages/pkg/package.json"), |v| {
            v["scripts"]["other"] = json!("node -e \"throw Error('dry graph must not execute')\"");
        });
        edit_json(&root.join("turbo.json"), |v| {
            v["tasks"]["other"] = json!({"env":["SOURCES_LEAF__OTHER"]});
        });
    }
    connect(
        &t.f,
        &t.middle,
        &t.leaf,
        "leaf",
        &["build", "other", "unrelated"],
    );
    connect(
        &t.f,
        &t.app,
        &t.middle,
        "middle",
        &["build", "other", "unrelated"],
    );
    edit_json(&t.app.join(".sources.conf"), |v| {
        v["sources"]["absent"] = json!({"url":"https://example.invalid/absent.git", "packages":{"missing":{"tasks":["build"]}}});
    });
    let unused = t.app.join("packages/unused");
    std::fs::create_dir_all(&unused).unwrap();
    write_json(
        &unused.join("package.json"),
        &json!({"name":"unused","version":"1.0.0","scripts":{"build":"node -e \"throw Error('unselected')\""}}),
    );
    edit_json(&t.app.join("turbo.json"), |v| {
        v["tasks"]["build"]["env"] = json!(["SOURCES_MIDDLE__BUILD", "SOURCES_MIDDLE__OTHER"]);
        v["tasks"]["unused#build"] = json!({"env":["SOURCES_MISSING__BUILD"]});
        v["tasks"]["prepare"]["env"] = json!(["SOURCES_MISSING__BUILD"]);
    });
    let trace = t.f.root.join("native-calls.jsonl");
    for root in [&t.app, &t.middle, &t.leaf] {
        let bin = root.join("node_modules/.bin/turbo");
        let native = root.join("node_modules/.bin/turbo-native");
        std::fs::rename(&bin, &native).unwrap();
        std::fs::write(
            &bin,
            format!(
                r#"#!/usr/bin/env node
const fs = require('node:fs'), cp = require('node:child_process');
fs.appendFileSync({}, JSON.stringify({{cwd:process.cwd(),args:process.argv.slice(2)}})+'\n');
const p = cp.spawnSync({},process.argv.slice(2),{{stdio:'inherit'}});
process.exit(p.status ?? 1);
"#,
                serde_json::to_string(&trace).unwrap(),
                serde_json::to_string(&native).unwrap()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let output = success(
        t.f.cli()
            .current_dir(&t.app)
            .env("CI", "true")
            .args([
                "turbo",
                "run",
                "build",
                "--filter=top",
                "--only",
                "--dry=json",
            ])
            .output()
            .unwrap(),
    );
    let metadata: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(metadata["tasks"].as_array().unwrap().len(), 1);
    assert_eq!(metadata["tasks"][0]["taskId"], "top#build");
    let calls: Vec<Value> = std::fs::read_to_string(&trace)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    for (owner, ids) in [
        (&t.middle, ["middle#build", "middle#other"]),
        (&t.leaf, ["leaf#build", "leaf#other"]),
    ] {
        let queries: Vec<_> = calls
            .iter()
            .filter(|call| Path::new(call["cwd"].as_str().unwrap()) == owner)
            .collect();
        assert!(
            queries.len() <= 2,
            "required tasks must be batched per owner, got {queries:?}"
        );
        assert!(
            queries.iter().any(|call| ids
                .iter()
                .all(|id| call["args"].as_array().unwrap().contains(&json!(id)))),
            "required exact pairs must appear together: {queries:?}"
        );
    }
    assert!(
        calls
            .iter()
            .all(|call| !call["args"].to_string().contains("unrelated")),
        "{calls:?}"
    );
    // Removing --only makes the native prerequisite's external input required.
    common::failure(
        t.f.cli()
            .current_dir(&t.app)
            .env("CI", "true")
            .args(["turbo", "run", "build", "--filter=top", "--dry=json"])
            .output()
            .unwrap(),
        "absent",
    );
    for root in [&t.app, &t.middle, &t.leaf] {
        assert!(!root.join("executions.log").exists());
    }
}
