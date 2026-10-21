mod common;
#[path = "common/turbo.rs"]
mod turbo_fixture;

use common::success;
use serde_json::json;
use turbo_fixture::{TurboFixture, pnpm, write_json};

#[test]
fn turbo_forwards_parent_arguments_and_injects_all_exact_child_hashes() {
    let t = TurboFixture::new();
    t.parent();
    let path = t.child.join("turbo.json");
    let mut child_config: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    child_config["globalEnv"] = json!(["SOURCES_*"]);
    write_json(&path, &child_config);
    let output = success(
        pnpm(&t.f, &t.child)
            .args([
                "exec",
                "turbo",
                "run",
                "@example/protocol#build",
                "@example/protocol#pack",
                "@example/ui#pack",
                "--dry=json",
            ])
            .output()
            .unwrap(),
    );
    let metadata: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let hash = |id: &str| {
        metadata["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|task| task["taskId"] == id)
            .unwrap()["hash"]
            .clone()
    };
    let output = success(
        t.cli()
            .current_dir(t.f.app.join("packages/web"))
            .env("SOURCES_EXAMPLE_PROTOCOL__BUILD", "forged")
            .env("SOURCES_STALE__BUILD", "old-configuration")
            .env("UNRELATED_PROBE", "preserved")
            .args([
                "turbo",
                "run",
                "inspect",
                "--filter=@app/web",
                "--",
                "forwarded-argument",
            ])
            .output()
            .unwrap(),
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("parent-output"));
    let observed: serde_json::Value =
        serde_json::from_slice(&std::fs::read(t.f.app.join("observed-web.json")).unwrap()).unwrap();
    assert_eq!(
        observed["env"],
        json!({
            "SOURCES_EXAMPLE_PROTOCOL__BUILD":hash("@example/protocol#build"),
            "SOURCES_EXAMPLE_PROTOCOL__PACK":hash("@example/protocol#pack"),
            "SOURCES_EXAMPLE_UI__PACK":hash("@example/ui#pack"),
            "UNRELATED_PROBE":"preserved"
        })
    );
    assert_eq!(observed["args"], json!(["forwarded-argument"]));
    assert!(!t.f.app.join("observed-other.json").exists());
    assert!(!t.marker("protocol", "build").exists());
    assert!(!t.marker("protocol", "pack").exists());
    assert!(!t.marker("ui", "pack").exists());
}

#[test]
fn turbo_task_hashes_invalidate_only_the_parent_tasks_that_declare_them() {
    let t = TurboFixture::new();
    t.parent();
    for package in ["protocol", "ui"] {
        for input in ["build.txt", "pack.txt"] {
            std::fs::write(
                t.child.join("packages").join(package).join(input),
                "initial\n",
            )
            .unwrap();
        }
    }
    let path = t.child.join("turbo.json");
    let mut config: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    config["tasks"]["prepare"]["inputs"] = json!(["build.txt"]);
    config["tasks"]["build"]["inputs"] = json!(["build.txt"]);
    config["tasks"]["pack"]["inputs"] = json!(["pack.txt"]);
    write_json(&path, &config);
    let path = t.f.app.join("turbo.json");
    let mut config: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    config["globalDependencies"] = json!(["cache-task.cjs"]);
    config["tasks"]["@app/web#build"] =
        json!({"env":["SOURCES_EXAMPLE_PROTOCOL__PACK"],"outputs":["dist/out.txt"]});
    config["tasks"]["@app/other#build"] =
        json!({"env":["SOURCES_EXAMPLE_UI__PACK"],"outputs":["dist/out.txt"]});
    write_json(&path, &config);
    for package in ["web", "other"] {
        let path = t.f.app.join("packages").join(package).join("package.json");
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        manifest["scripts"]["build"] = json!(format!("node ../../cache-task.cjs {package}"));
        write_json(&path, &manifest);
    }
    std::fs::write(t.f.app.join("cache-task.cjs"), r#"
const fs = require('node:fs');
const name = process.argv[2];
fs.mkdirSync('dist', {recursive:true});
fs.writeFileSync('dist/out.txt', process.env[name === 'web' ? 'SOURCES_EXAMPLE_PROTOCOL__PACK' : 'SOURCES_EXAMPLE_UI__PACK']);
fs.appendFileSync('../../executions.log', name + '\n');
"#).unwrap();
    let run = || {
        success(
            t.cli()
                .args(["turbo", "run", "build", "inspect"])
                .output()
                .unwrap(),
        )
    };
    let snapshot = || -> serde_json::Value {
        serde_json::from_slice(&std::fs::read(t.f.app.join("observed-web.json")).unwrap()).unwrap()
    };
    let executions = || -> Vec<String> {
        let mut lines: Vec<_> = std::fs::read_to_string(t.f.app.join("executions.log"))
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect();
        lines.sort();
        lines
    };
    run();
    let original = snapshot();
    assert_eq!(executions(), ["other", "web"]);
    // Populate child task caches explicitly. A cache-hit dry run must export the same hashes.
    success(
        t.cli()
            .args(["task", "@example/protocol:pack", "@example/ui:pack"])
            .output()
            .unwrap(),
    );
    run();
    assert_eq!(snapshot()["env"], original["env"]);
    assert_eq!(executions(), ["other", "web"]);
    std::fs::write(
        t.child.join("packages/protocol/pack.txt"),
        "changed packaging input\n",
    )
    .unwrap();
    run();
    let changed = snapshot();
    assert_ne!(
        changed["env"]["SOURCES_EXAMPLE_PROTOCOL__PACK"],
        original["env"]["SOURCES_EXAMPLE_PROTOCOL__PACK"]
    );
    assert_eq!(
        changed["env"]["SOURCES_EXAMPLE_PROTOCOL__BUILD"],
        original["env"]["SOURCES_EXAMPLE_PROTOCOL__BUILD"]
    );
    assert_eq!(
        changed["env"]["SOURCES_EXAMPLE_UI__PACK"],
        original["env"]["SOURCES_EXAMPLE_UI__PACK"]
    );
    assert_eq!(executions(), ["other", "web", "web"]);
}

#[cfg(unix)]
mod nested {
    use super::*;
    use common::failure;
    use std::io::{BufRead, BufReader, Write};
    use std::net::{TcpListener, TcpStream};
    use std::os::fd::AsRawFd;
    use std::process::{Child, Stdio};
    use std::time::{Duration, Instant};

    struct Running(Option<Child>);
    impl Drop for Running {
        fn drop(&mut self) {
            if let Some(child) = &mut self.0 {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }

    fn event(listener: &TcpListener) -> (serde_json::Value, TcpStream) {
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            let mut descriptor = libc::pollfd {
                fd: listener.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(!remaining.is_zero(), "nested Turbo handshake timed out");
            let result = unsafe {
                libc::poll(
                    &mut descriptor,
                    1,
                    remaining.as_millis().min(i32::MAX as u128) as i32,
                )
            };
            if result == -1
                && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted
            {
                continue;
            }
            assert!(result > 0, "nested Turbo did not reach its next handshake");
            let (stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(10)))
                .unwrap();
            let mut line = String::new();
            BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut line)
                .unwrap();
            return (serde_json::from_str(&line).unwrap(), stream);
        }
    }

    #[test]
    fn parent_turbo_releases_its_lock_and_serializes_concurrent_nested_tasks() {
        let t = TurboFixture::new();
        t.parent();
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let binary =
            serde_json::to_string(assert_cmd::cargo::cargo_bin!("sources").to_str().unwrap())
                .unwrap();
        let launcher = r#"
const net = require('node:net');
const {spawn} = require('node:child_process');
const name = process.argv[2], selector = process.argv[3];
const notify = (kind, extra = {}) => net.connect(Number(process.env.NESTED_TEST_PORT), '127.0.0.1', function() { this.end(JSON.stringify({kind, name, ...extra}) + '\n'); });
const socket = net.connect(Number(process.env.NESTED_TEST_PORT), '127.0.0.1', () => socket.write(JSON.stringify({kind:'ready', name, sourceCount:Object.keys(process.env).filter(key => key.startsWith('SOURCES_')).length}) + '\n'));
const deadline = setTimeout(() => process.exit(88), 60000);
socket.once('data', () => {
  socket.end();
  const child = spawn(SOURCES_BINARY, ['task', selector], {stdio:['ignore','pipe','pipe'], env:{...process.env, TURBO_TELEMETRY_DISABLED:'1'}});
  child.stdout.on('data', bytes => process.stdout.write(bytes));
  let stderr = '', reported = false;
  child.stderr.on('data', bytes => {
    process.stderr.write(bytes); stderr += bytes;
    if (!reported && stderr.includes('waiting for project lock')) { reported = true; notify('waiting'); }
  });
  child.on('exit', code => { notify('exit', {code}); clearTimeout(deadline); process.exitCode = code; });
});
"#.replace("SOURCES_BINARY", &binary);
        std::fs::write(t.f.app.join("nested.cjs"), launcher).unwrap();
        std::fs::write(t.child.join("hold.cjs"), r#"
const fs = require('node:fs'), net = require('node:net');
const name = process.argv[2];
const socket = net.connect(Number(process.env.NESTED_TEST_PORT), '127.0.0.1', () => socket.write(JSON.stringify({kind:'child', name, env:Object.fromEntries(Object.entries(process.env).filter(([key]) => key.startsWith('SOURCES_')))}) + '\n'));
const deadline = setTimeout(() => process.exit(88), 60000);
socket.once('data', () => { fs.mkdirSync('dist', {recursive:true}); fs.writeFileSync('dist/pack.txt', name); clearTimeout(deadline); socket.end(); });
socket.on('end', () => { clearTimeout(deadline); process.exit(0); });
"#).unwrap();
        for (package, selector) in [
            ("web", "@example/protocol:pack"),
            ("other", "@example/ui:pack"),
        ] {
            let path = t.f.app.join("packages").join(package).join("package.json");
            let mut manifest: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            manifest["scripts"]["prepare"] =
                json!(format!("node ../../nested.cjs {package} {selector}"));
            write_json(&path, &manifest);
        }
        let path = t.f.app.join("turbo.json");
        let mut config: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        config["tasks"]["prepare"] =
            json!({"cache":false,"passThroughEnv":["NESTED_TEST_PORT", "SOURCES_*"]});
        write_json(&path, &config);
        for package in ["protocol", "ui"] {
            let path = t.child.join("packages").join(package).join("package.json");
            let mut manifest: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            manifest["scripts"]["pack"] = json!(format!("node ../../hold.cjs {package}"));
            write_json(&path, &manifest);
        }
        let path = t.child.join("turbo.json");
        let mut config: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        config["globalEnv"] = json!(["SOURCES_*"]);
        config["tasks"]["pack"]["cache"] = json!(false);
        config["tasks"]["pack"]["passThroughEnv"] = json!(["NESTED_TEST_PORT"]);
        write_json(&path, &config);
        let mut running = Running(Some(
            t.cli()
                .args(["turbo", "run", "prepare", "--concurrency=2"])
                .env(
                    "NESTED_TEST_PORT",
                    listener.local_addr().unwrap().port().to_string(),
                )
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap(),
        ));
        let mut ready = Vec::new();
        for _ in 0..2 {
            let (message, stream) = event(&listener);
            assert_eq!(message["kind"], "ready", "{message}");
            assert!(
                message["sourceCount"].as_u64().unwrap() > 0,
                "nested task must inherit the parent snapshot"
            );
            ready.push(stream);
        }
        for mut stream in ready {
            stream.write_all(b"go\n").unwrap();
        }
        let mut first = None;
        let mut waiting = false;
        while first.is_none() || !waiting {
            let (message, stream) = event(&listener);
            match message["kind"].as_str().unwrap() {
                "child" => {
                    assert_eq!(
                        message["env"],
                        json!({}),
                        "child execution must clear inherited hash inputs"
                    );
                    assert!(
                        first.is_none(),
                        "second child ran while the first held the lock"
                    );
                    first = Some(stream);
                }
                "waiting" => waiting = true,
                "exit" => panic!("nested command exited before the lock handoff: {message}"),
                _ => panic!("unexpected nested event {message}"),
            }
        }
        failure(
            t.cli().args(["status", "--json"]).output().unwrap(),
            "locked",
        );
        first.take().unwrap().write_all(b"release\n").unwrap();
        let mut second = None;
        let mut exits = 0;
        while second.is_none() || exits < 1 {
            let (message, stream) = event(&listener);
            match message["kind"].as_str().unwrap() {
                "child" => {
                    assert_eq!(message["env"], json!({}));
                    second = Some(stream);
                }
                "exit" => {
                    assert_eq!(message["code"], 0, "{message}");
                    exits += 1;
                }
                _ => panic!("unexpected nested event {message}"),
            }
        }
        second.take().unwrap().write_all(b"release\n").unwrap();
        let (message, _) = event(&listener);
        assert_eq!(message["kind"], "exit");
        assert_eq!(message["code"], 0, "{message}");
        let child = running.0.take().unwrap();
        let pid = child.id();
        let (sender, receiver) = std::sync::mpsc::channel();
        let waiter = std::thread::spawn(move || {
            let _ = sender.send(child.wait_with_output());
        });
        let output = receiver
            .recv_timeout(Duration::from_secs(30))
            .unwrap_or_else(|_| {
                unsafe {
                    libc::kill(pid as i32, libc::SIGTERM);
                }
                panic!("parent Turbo did not exit after both nested tasks completed");
            })
            .unwrap();
        waiter.join().unwrap();
        success(output);
        assert!(t.marker("protocol", "pack").is_file());
        assert!(t.marker("ui", "pack").is_file());
    }
}
