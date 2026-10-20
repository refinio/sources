#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Child;
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
        use std::io::Write;
        let output = self
            .command("git")
            .current_dir(cwd)
            .args(["-c", "init.defaultBranch=main", "-c", "core.autocrlf=false"])
            .args(args)
            .output()
            .unwrap();
        let args = args
            .iter()
            .map(|arg| sources::git::redact(arg))
            .collect::<Vec<_>>();
        let stderr = sources::git::redact(&String::from_utf8_lossy(&output.stderr));
        let mut transcript = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.root.join("git-commands.jsonl"))
            .unwrap();
        serde_json::to_writer(
            &mut transcript,
            &serde_json::json!({
                "directory": cwd.to_string_lossy(), "arguments": args,
                "status": output.status.code(), "stderr": stderr,
            }),
        )
        .unwrap();
        transcript.write_all(b"\n").unwrap();
        assert!(output.status.success(), "git {args:?}: {}", stderr);
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

    pub fn server(&self, repository: &str) -> GitServer {
        use std::net::{SocketAddr, TcpListener, TcpStream};
        use std::time::{Duration, Instant};
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let log = std::fs::File::create(self.root.join(format!("daemon-{port}.log"))).unwrap();
        let exec_path = self.git(&self.root, &["--exec-path"]);
        let daemon = Path::new(&exec_path).join(if cfg!(windows) {
            "git-daemon.exe"
        } else {
            "git-daemon"
        });
        let mut child = self
            .command(daemon)
            .args([
                "--verbose",
                "--reuseaddr",
                "--export-all",
                "--listen=127.0.0.1",
            ])
            .arg(format!("--port={port}"))
            .arg(format!("--base-path={}", self.root.display()))
            .arg(&self.root)
            .stdout(std::process::Stdio::null())
            .stderr(log)
            .spawn()
            .unwrap();
        let address: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            assert!(
                child.try_wait().unwrap().is_none(),
                "git daemon exited before listening"
            );
            if TcpStream::connect_timeout(&address, Duration::from_millis(20)).is_ok() {
                break;
            }
            assert!(Instant::now() < deadline, "git daemon did not listen");
            std::thread::yield_now();
        }
        GitServer {
            child,
            url: format!("git://127.0.0.1:{port}/{repository}"),
        }
    }
}

pub struct GitServer {
    child: Child,
    pub url: String,
}
impl Drop for GitServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
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
