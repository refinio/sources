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

    pub fn add_source(&self, name: &str, package: &str) -> PathBuf {
        let child = self.f.repo(name);
        let upstream = self.f.root.join("upstream");
        self.f.git(
            &child,
            &["remote", "add", "origin", upstream.to_str().unwrap()],
        );
        for file in [
            "package.json",
            "pnpm-workspace.yaml",
            "turbo.json",
            "task.cjs",
            ".gitignore",
        ] {
            std::fs::copy(self.child.join(file), child.join(file)).unwrap();
        }
        let directory = child.join("packages/other");
        std::fs::create_dir_all(&directory).unwrap();
        write_json(
            &directory.join("package.json"),
            &json!({"name":package,"version":"1.0.0","scripts":{
                "prepare":"node ../../task.cjs prepare",
                "build":"node ../../task.cjs build",
                "pack":"node ../../task.cjs pack"
            }}),
        );
        success(
            pnpm(&self.f, &child)
                .args(["install", "--offline", "--ignore-scripts"])
                .output()
                .unwrap(),
        );
        install_turbo(&child);
        self.f.git(&child, &["add", "."]);
        self.f
            .git(&child, &["commit", "-qm", "Additional Turbo source"]);
        let path = self.f.app.join(".sources.conf");
        let mut config: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        config["sources"][name] =
            json!({"url":upstream,"packages":{package:{"tasks":["build","pack"]}}});
        write_json(&path, &config);
        let path = self.f.app.join(".sources.local.conf");
        let mut local: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        local["sources"][name] = json!(child);
        write_json(&path, &local);
        success(self.cli().arg("dev").output().unwrap());
        child
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
    assert_eq!(String::from_utf8(version.stdout).unwrap().trim(), "2.7.6");
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
