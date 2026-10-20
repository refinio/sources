#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;

pub struct Fixture {
    temp: Option<TempDir>,
    pub root: PathBuf,
    pub app: PathBuf,
}

impl Fixture {
    pub fn new() -> Self {
        let temp = tempfile::Builder::new()
            .prefix("sources-test-")
            .tempdir()
            .unwrap();
        let root = temp.path().to_owned();
        let app = root.join("app");
        std::fs::create_dir_all(&app).unwrap();
        std::fs::create_dir(root.join("home")).unwrap();
        std::fs::write(root.join("gitconfig"), "").unwrap();
        Self {
            temp: Some(temp),
            root,
            app,
        }
    }

    pub fn command(&self, program: impl AsRef<std::ffi::OsStr>) -> Command {
        let mut command = Command::new(program);
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("GIT_") {
                command.env_remove(key);
            }
        }
        command
            .env("HOME", self.root.join("home"))
            .env("XDG_CONFIG_HOME", self.root.join("home"))
            .env("GIT_CONFIG_GLOBAL", self.root.join("gitconfig"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_AUTHOR_NAME", "Fixture")
            .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
            .env("GIT_COMMITTER_NAME", "Fixture")
            .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid")
            .env("GIT_AUTHOR_DATE", "2024-01-01T00:00:00Z")
            .env("GIT_COMMITTER_DATE", "2024-01-01T00:00:00Z")
            .env("LC_ALL", "C");
        command
    }

    pub fn cli(&self) -> Command {
        let mut command = self.command(assert_cmd::cargo::cargo_bin!("sources"));
        command.current_dir(&self.app);
        command
    }

    pub fn git(&self, cwd: &Path, args: &[&str]) -> String {
        let output = self
            .command("git")
            .current_dir(cwd)
            .args(["-c", "init.defaultBranch=main", "-c", "core.autocrlf=false"])
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    }

    pub fn repo(&self, name: &str) -> PathBuf {
        let path = self.root.join(name);
        std::fs::create_dir_all(&path).unwrap();
        self.git(&path, &["init"]);
        std::fs::write(path.join("tracked.txt"), "initial\n").unwrap();
        std::fs::write(path.join(".gitignore"), "artifact.txt\nnode_modules/\n").unwrap();
        self.git(&path, &["add", "."]);
        self.git(&path, &["commit", "-qm", "initial"]);
        path
    }

    pub fn configure(&self, sources: serde_json::Value, local: serde_json::Value) {
        std::fs::write(
            self.app.join(".sources.conf"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "version": 1, "directory": "sources", "sources": sources
            }))
            .unwrap(),
        )
        .unwrap();
        std::fs::write(
            self.app.join(".sources.local.conf"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "sources": local
            }))
            .unwrap(),
        )
        .unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if std::thread::panicking()
            && let Some(temp) = self.temp.take()
        {
            eprintln!("Failed fixture retained at {}", temp.keep().display());
        }
    }
}

pub fn success(output: Output) -> Output {
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

pub fn failure(output: Output, text: &str) {
    assert_eq!(
        output.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains(text),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
