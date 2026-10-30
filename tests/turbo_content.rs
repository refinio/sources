mod common;
#[path = "common/turbo.rs"]
mod turbo_fixture;

use common::{failure, success};
use serde_json::{Value, json};
use std::path::Path;
use std::time::Instant;
use turbo_fixture::{TurboFixture, pnpm, write_json};

fn edit(path: &Path, change: impl FnOnce(&mut Value)) {
    let mut value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    change(&mut value);
    write_json(path, &value);
}

#[test]
fn content_bridge_refreshes_installed_payload_before_downstream_hashing() {
    let t = TurboFixture::new();
    t.parent();
    let package = t.child.join("packages/protocol");
    edit(&package.join("package.json"), |v| {
        v["files"] = json!(["dist", "assets"]);
        v["types"] = json!("dist/index.d.ts");
    });
    std::fs::create_dir(package.join("assets")).unwrap();
    std::fs::write(package.join("assets/schema.json"), "{\"version\":1}\n").unwrap();
    std::fs::write(package.join("input.txt"), "alpha").unwrap();
    std::fs::write(
        t.child.join("task.cjs"),
        r#"
const fs = require('node:fs');
const value = fs.readFileSync('input.txt','utf8').split('\n')[0];
if (value === 'fail') process.exit(17);
fs.rmSync('dist', {recursive:true, force:true});
fs.mkdirSync('dist');
if (value !== 'missing') {
  const name = value === 'rename' ? 'renamed.txt' : 'value.txt';
  fs.writeFileSync('dist/' + name + '.new', value);
  fs.renameSync('dist/' + name + '.new', 'dist/' + name);
  fs.writeFileSync('dist/index.d.ts','export declare const value: string;\n');
}
"#,
    )
    .unwrap();
    edit(&t.child.join("turbo.json"), |v| {
        v["tasks"]["pack"] = json!({"outputs":["dist/**"]});
    });
    // Bootstrap native file installation without Sources orchestration first.
    success(
        t.cli()
            .args(["task", "@example/protocol:pack"])
            .output()
            .unwrap(),
    );
    edit(&t.f.app.join("package.json"), |v| {
        v["dependencies"] = json!({"@example/protocol":"file:./sources/shared/packages/protocol"});
    });
    success(
        pnpm(&t.f, &t.f.app)
            .args(["install", "--offline"])
            .output()
            .unwrap(),
    );
    turbo_fixture::install_turbo(&t.f.app);
    edit(&t.f.app.join("package.json"), |v| {
        v["scripts"] = json!({"postinstall":"node refresh.cjs"});
    });
    std::fs::write(t.f.app.join("refresh.cjs"), r#"
const fs = require('node:fs');
fs.writeFileSync('refresh-env.json', JSON.stringify(Object.fromEntries(Object.entries(process.env).filter(([key])=>key.startsWith('SOURCES_')))));
"#).unwrap();
    let lock = std::fs::read(t.f.app.join("pnpm-lock.yaml")).unwrap();
    std::fs::remove_dir_all(t.child.join(".turbo")).unwrap();
    edit(&t.f.app.join(".sources.conf"), |v| {
        v["sources"]["shared"]["packages"] = json!({"@example/protocol":{
            "tasks":["pack"], "content":{"pack":{"artifacts":[
                {"path":"packages/protocol/dist", "installed":"node_modules/@example/protocol/dist"},
                {"path":"packages/protocol/package.json", "installed":"node_modules/@example/protocol/package.json"},
                {"path":"packages/protocol/assets", "installed":"node_modules/@example/protocol/assets"}
            ]}}
        }});
    });
    edit(&t.f.app.join("turbo.json"), |v| {
        v["tasks"]["inspect"] = json!({
            "inputs":["package.json"], "env":["SOURCES_EXAMPLE_PROTOCOL__PACK__CONTENT"],
            "outputs":["dist/**"]
        });
    });
    std::fs::write(t.f.app.join("inspect.cjs"), r#"
const fs = require('node:fs');
const root = '../../node_modules/@example/protocol/';
const files = fs.readdirSync(root + 'dist').sort();
fs.mkdirSync('dist', {recursive:true});
fs.writeFileSync('dist/result.json', JSON.stringify({files, value:files.filter(n=>n.endsWith('.txt')).map(n=>fs.readFileSync(root+'dist/'+n,'utf8')), schema:fs.readFileSync(root+'assets/schema.json','utf8'), types:fs.readFileSync(root+'dist/index.d.ts','utf8'), manifest:JSON.parse(fs.readFileSync(root+'package.json','utf8'))}));
fs.appendFileSync('../../executions.log', 'run\n');
"#).unwrap();
    let result = t.f.app.join("packages/web/dist/result.json");
    let run = |case: &str, expected: &str| {
        let runs = t.f.app.join(".turbo/runs");
        let before = std::fs::read_dir(&runs)
            .into_iter()
            .flatten()
            .map(|entry| entry.unwrap().path())
            .collect::<std::collections::BTreeSet<_>>();
        let start = Instant::now();
        let output = success(
            t.cli()
                .env("SOURCES_STALE__PACK", "obsolete")
                .env("SOURCES_EXAMPLE_PROTOCOL__PACK__CONTENT", "forged")
                .args([
                    "turbo",
                    "run",
                    "inspect",
                    "--filter=@app/web",
                    "--summarize",
                ])
                .output()
                .unwrap(),
        );
        let refresh_env: Value =
            serde_json::from_slice(&std::fs::read(t.f.app.join("refresh-env.json")).unwrap())
                .unwrap();
        assert!(
            refresh_env.get("SOURCES_STALE__PACK").is_none(),
            "{refresh_env}"
        );
        assert!(
            refresh_env["SOURCES_EXAMPLE_PROTOCOL__PACK__CONTENT"]
                .as_str()
                .unwrap()
                .starts_with("sha256:"),
            "{refresh_env}"
        );
        let text = String::from_utf8_lossy(&output.stdout);
        let summary_path = std::fs::read_dir(&runs)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| !before.contains(path))
            .expect("native parent Turbo run summary");
        let summary: Value = serde_json::from_slice(&std::fs::read(summary_path).unwrap()).unwrap();
        let task = summary["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|task| task["taskId"] == "@app/web#inspect")
            .unwrap();
        let status = if expected == "cache hit" {
            "HIT"
        } else {
            "MISS"
        };
        assert_eq!(task["cache"]["status"], status, "{case}: {text}");
        if case == "cache restoration" {
            assert!(text.contains("@example/protocol:pack: cache hit"), "{text}");
        }
        if case == "source change identical bytes" {
            assert!(
                text.contains("@example/protocol:pack: cache miss"),
                "{text}"
            );
        }
        assert_eq!(std::fs::read(t.f.app.join("pnpm-lock.yaml")).unwrap(), lock);
        eprintln!(
            "content proof {case}: {:.3}s {status} parent_hash={} upstream={} refresh={}",
            start.elapsed().as_secs_f64(),
            task["hash"],
            text.lines()
                .find(|line| line.contains("@example/protocol:pack: cache"))
                .unwrap_or("no cache status"),
            text.lines()
                .find(|line| line.starts_with("Done in "))
                .unwrap_or("native filtered install")
        );
        serde_json::from_slice::<Value>(&std::fs::read(&result).unwrap()).unwrap()
    };
    let first = run("initial", "cache miss");
    assert_eq!(first["value"], json!(["alpha"]));
    assert_eq!(first["types"], "export declare const value: string;\n");
    assert_eq!(first["manifest"]["types"], "dist/index.d.ts");
    std::fs::remove_dir_all(package.join("dist")).unwrap();
    std::fs::remove_dir_all(t.f.app.join("packages/web/dist")).unwrap();
    assert_eq!(run("cache restoration", "cache hit"), first);
    assert_eq!(run("no change", "cache hit"), first);
    std::fs::write(package.join("input.txt"), "alpha\ncomment-only source edit").unwrap();
    assert_eq!(run("source change identical bytes", "cache hit"), first);
    std::fs::write(package.join("input.txt"), "beta").unwrap();
    assert_eq!(run("changed bytes", "cache miss")["value"], json!(["beta"]));
    assert_eq!(
        std::fs::read_to_string(
            t.f.app
                .join("node_modules/@example/protocol/dist/value.txt")
        )
        .unwrap(),
        "beta"
    );
    std::fs::write(package.join("input.txt"), "rename").unwrap();
    let renamed = run("path rename and deletion", "cache miss");
    assert!(
        !renamed["files"]
            .as_array()
            .unwrap()
            .contains(&json!("value.txt"))
    );
    assert_eq!(renamed["value"], json!(["rename"]));
    std::fs::write(package.join("assets/schema.json"), "{\"version\":2}\n").unwrap();
    assert_eq!(
        run("asset metadata change", "cache miss")["schema"],
        "{\"version\":2}\n"
    );
    let executions = std::fs::read(t.f.app.join("executions.log")).unwrap();
    for (input, message) in [
        ("fail", "upstream Turbo failed"),
        ("missing", "required artifact"),
    ] {
        std::fs::write(package.join("input.txt"), input).unwrap();
        let output = t
            .cli()
            .args(["turbo", "run", "inspect", "--filter=@app/web"])
            .output()
            .unwrap();
        if input == "fail" {
            assert_eq!(output.status.code(), Some(17));
            assert!(String::from_utf8_lossy(&output.stderr).contains(message));
        } else {
            failure(output, message);
        }
        assert_eq!(
            std::fs::read(t.f.app.join("executions.log")).unwrap(),
            executions
        );
    }
    // Preserve native lifecycle handling, but refuse to hash/execute downstream
    // when an installation script leaves the payload different from upstream.
    std::fs::write(package.join("input.txt"), "beta").unwrap();
    edit(&t.f.app.join("package.json"), |v| {
        v["scripts"] = json!({"postinstall":"node corrupt.cjs"});
    });
    std::fs::write(
        t.f.app.join("corrupt.cjs"),
        r#"
const fs = require('node:fs');
const path = 'node_modules/@example/protocol/dist/value.txt';
fs.writeFileSync(path + '.new', 'stale-installed-copy');
fs.renameSync(path + '.new', path);
"#,
    )
    .unwrap();
    failure(
        t.cli()
            .args(["turbo", "run", "inspect", "--filter=@app/web"])
            .output()
            .unwrap(),
        "installed consumed artifact",
    );
    assert_eq!(
        std::fs::read(t.f.app.join("executions.log")).unwrap(),
        executions
    );
    edit(&t.f.app.join("package.json"), |v| {
        v["scripts"] = json!({"postinstall":"node refresh.cjs"});
    });

    // Frozen refresh cannot choose new dependency versions or rewrite the lock.
    edit(&t.f.app.join("package.json"), |v| {
        v["dependencies"]["unresolved-fixture"] = json!("9.0.0");
    });
    failure(
        t.cli()
            .args(["turbo", "run", "inspect", "--filter=@app/web"])
            .output()
            .unwrap(),
        "pnpm content refresh failed",
    );
    assert_eq!(std::fs::read(t.f.app.join("pnpm-lock.yaml")).unwrap(), lock);
    assert_eq!(
        std::fs::read(t.f.app.join("executions.log")).unwrap(),
        executions
    );
    edit(&t.f.app.join("package.json"), |v| {
        v["dependencies"]
            .as_object_mut()
            .unwrap()
            .remove("unresolved-fixture");
    });

    // Select the owning tooling root through the native filtered refresh seam.
    edit(&t.f.app.join(".sources.conf"), |v| {
        v["pnpm_refresh"] = json!({"filters":["parent-fixture"]});
    });
    let development = run("development recovery before pin", "cache miss");
    assert_eq!(development["value"], json!(["beta"]));
    // The same stable Sources path and native directory-file installation also
    // work after selecting a pinned, independently bootstrapped managed clone.
    t.f.git(&t.child, &["add", "."]);
    t.f.git(&t.child, &["commit", "-qm", "Content package"]);
    t.f.git(&t.child, &["push", "origin", "HEAD:content-fixture"]);
    success(t.cli().args(["lock", "--git", "--check"]).output().unwrap());
    success(t.cli().args(["release", "--git"]).output().unwrap());
    let managed = t.f.app.join(".sources/checkouts/shared");
    success(
        pnpm(&t.f, &managed)
            .args([
                "--filter=child-fixture",
                "--filter=@example/protocol",
                "install",
                "--offline",
                "--frozen-lockfile",
            ])
            .output()
            .unwrap(),
    );
    turbo_fixture::install_turbo(&managed);
    assert_eq!(
        std::fs::canonicalize(t.f.app.join("sources/shared")).unwrap(),
        std::fs::canonicalize(&managed).unwrap()
    );
    assert_eq!(run("pinned checkout refresh", "cache hit"), development);
    assert_eq!(
        run("pinned no change", "cache hit")["value"],
        json!(["beta"])
    );
}
