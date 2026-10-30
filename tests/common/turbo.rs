#![allow(dead_code)]

use crate::common::{Fixture, success};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct TurboFixture {
    pub f: Fixture,
    pub child: PathBuf,
}

impl TurboFixture {
    pub fn new() -> Self {
        let f = Fixture::new();
        let child = f.repo("child");
        let upstream = f.repo("upstream");
        f.git(
            &child,
            &["remote", "add", "origin", upstream.to_str().unwrap()],
        );
        write_json(
            &child.join("package.json"),
            &json!({"name":"child-fixture","private":true,"packageManager":"pnpm@10.15.0"}),
        );
        std::fs::write(
            child.join("pnpm-workspace.yaml"),
            "packages:\n  - packages/*\n",
        )
        .unwrap();
        write_json(
            &child.join("turbo.json"),
            &json!({"tasks":{
                "prepare":{"outputs":["dist/prepare.txt"]},
                "build":{"dependsOn":["prepare"],"outputs":["dist/build.txt"]},
                "pack":{"outputs":["dist/pack.txt"]}
            }}),
        );
        std::fs::write(
            child.join("task.cjs"),
            "const fs = require('node:fs'); fs.mkdirSync('dist', {recursive:true}); fs.appendFileSync('dist/' + process.argv[2] + '.txt', process.argv[2] + '\\n');\n",
        ).unwrap();
        for name in ["protocol", "ui"] {
            let package = child.join("packages").join(name);
            std::fs::create_dir_all(&package).unwrap();
            write_json(
                &package.join("package.json"),
                &json!({"name":format!("@example/{name}"),"version":"1.0.0","scripts":{
                    "prepare":"node ../../task.cjs prepare",
                    "build":"node ../../task.cjs build",
                    "pack":"node ../../task.cjs pack"
                }}),
            );
        }
        std::fs::write(child.join(".gitignore"), "node_modules/\ndist/\n.turbo/\n").unwrap();
        success(
            pnpm(&f, &child)
                .args(["install", "--offline", "--ignore-scripts"])
                .output()
                .unwrap(),
        );
        install_turbo(&child);
        f.git(&child, &["add", "."]);
        f.git(&child, &["commit", "-qm", "Turbo fixture"]);
        f.configure(json!({"shared":{"url":upstream}}), json!({"shared":child}));
        success(f.cli().arg("dev").output().unwrap());
        f.configure(
            json!({"shared":{"url":upstream,"packages":{
                "@example/protocol":{"tasks":["build","pack"]},
                "@example/ui":{"tasks":["pack"]}
            }}}),
            json!({"shared":child}),
        );
        Self { f, child }
    }

    pub fn marker(&self, package: &str, task: &str) -> PathBuf {
        self.child
            .join("packages")
            .join(package)
            .join("dist")
            .join(format!("{task}.txt"))
    }

    pub fn cli(&self) -> Command {
        let mut command = self.f.cli();
        command
            .env("CI", "true")
            .env("TURBO_TELEMETRY_DISABLED", "1");
        command
    }

    pub fn parent(&self) {
        write_json(
            &self.f.app.join("package.json"),
            &json!({"name":"parent-fixture","private":true,"packageManager":"pnpm@10.15.0"}),
        );
        std::fs::write(
            self.f.app.join("pnpm-workspace.yaml"),
            "packages:\n  - packages/*\n",
        )
        .unwrap();
        for (directory, name) in [("web", "@app/web"), ("other", "@app/other")] {
            let package = self.f.app.join("packages").join(directory);
            std::fs::create_dir_all(&package).unwrap();
            write_json(
                &package.join("package.json"),
                &json!({"name":name,"version":"1.0.0","scripts":{
                    "inspect":"node ../../inspect.cjs"
                }}),
            );
        }
        write_json(
            &self.f.app.join("turbo.json"),
            &json!({"tasks":{"inspect":{
                "cache":false,"passThroughEnv":["SOURCES_*","UNRELATED_PROBE"]
            }}}),
        );
        std::fs::write(self.f.app.join("inspect.cjs"), r#"
const fs = require('node:fs');
const env = Object.fromEntries(Object.entries(process.env).filter(([key]) => key.startsWith('SOURCES_') || key === 'UNRELATED_PROBE'));
fs.writeFileSync('../../observed-' + JSON.parse(fs.readFileSync('package.json', 'utf8')).name.split('/').pop() + '.json', JSON.stringify({env, args:process.argv.slice(2), cwd:process.cwd()}));
console.log('parent-output');
"#).unwrap();
        std::fs::write(self.f.app.join(".gitignore"), "node_modules/\n.turbo/\ndist/\nsources/\n.sources/\n.sources.local.conf\nobserved-*.json\n").unwrap();
        success(
            pnpm(&self.f, &self.f.app)
                .args(["install", "--offline", "--ignore-scripts"])
                .output()
                .unwrap(),
        );
        install_turbo(&self.f.app);
        self.f.git(&self.f.app, &["init"]);
        self.f.git(&self.f.app, &["add", "."]);
        self.f
            .git(&self.f.app, &["commit", "-qm", "Parent Turbo fixture"]);
    }
}

pub fn pnpm(f: &Fixture, root: &Path) -> Command {
    #[cfg(windows)]
    let program = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .flat_map(|directory| [directory.join("pnpm.exe"), directory.join("pnpm.cmd")])
        .find(|path| path.is_file())
        .expect("tests require installed pnpm 10.15.0");
    #[cfg(not(windows))]
    let program = "pnpm";
    let mut command = f.command(program);
    for (key, _) in std::env::vars_os() {
        if key
            .to_string_lossy()
            .to_ascii_uppercase()
            .starts_with("SOURCES_")
        {
            command.env_remove(key);
        }
    }
    command
        .current_dir(root)
        .env("CI", "true")
        .env("TURBO_TELEMETRY_DISABLED", "1");
    command
}

pub fn install_turbo(root: &Path) {
    let binary = PathBuf::from(std::env::var_os("SOURCES_TEST_TURBO").expect(
        "set SOURCES_TEST_TURBO to the installed Turbo 2.7.6 native executable before running tests",
    ));
    let version = success(Command::new(&binary).arg("--version").output().unwrap());
    assert!(matches!(
        String::from_utf8(version.stdout).unwrap().trim(),
        "2.7.6" | "2.8.0"
    ));
    let package = binary.parent().unwrap().parent().unwrap();
    let modules = root.join("node_modules");
    copy_tree(package, &modules.join(package.file_name().unwrap()));
    copy_tree(
        &package.parent().unwrap().join("turbo"),
        &modules.join("turbo"),
    );
    let bin = modules.join(".bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::copy(
        binary,
        bin.join(if cfg!(windows) { "turbo.exe" } else { "turbo" }),
    )
    .unwrap();
}

fn copy_tree(source: &Path, target: &Path) {
    std::fs::create_dir_all(target).unwrap();
    for entry in std::fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let path = target.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &path);
        } else {
            std::fs::copy(entry.path(), path).unwrap();
        }
    }
}

pub fn write_json(path: &Path, value: &serde_json::Value) {
    std::fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}
