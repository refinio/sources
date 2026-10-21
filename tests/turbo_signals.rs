#![cfg(unix)]

mod common;
#[path = "common/turbo.rs"]
mod turbo_fixture;

use common::{failure, success};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};
use turbo_fixture::{TurboFixture, write_json};

const HOLD: &str = r#"
const net = require('node:net');
const socket = net.createConnection(Number(process.argv[2]), '127.0.0.1', () => {
  const ready = Buffer.alloc(5); ready[0] = 82; ready.writeUInt32BE(process.pid, 1); socket.write(ready);
});
let caught = false;
for (const [name, code] of [['SIGINT', 2], ['SIGTERM', 15]]) {
  process.on(name, () => { if (!caught) { caught = true; socket.write(Buffer.from([code])); } });
}
socket.on('data', () => process.exit(47));
socket.on('close', () => process.exit(0));
socket.on('error', () => process.exit(71));
"#;

struct Running {
    child: Option<Child>,
    task_group: Option<i32>,
    stdout: PathBuf,
    stderr: PathBuf,
}

impl Running {
    fn spawn(command: &mut Command, log: PathBuf) -> Self {
        let stdout = log.with_extension("stdout");
        let stderr = log.with_extension("stderr");
        command
            .stdin(Stdio::null())
            .stdout(std::fs::File::create(&stdout).unwrap())
            .stderr(std::fs::File::create(&stderr).unwrap());
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        Self {
            child: Some(command.spawn().unwrap()),
            task_group: None,
            stdout,
            stderr,
        }
    }

    fn signal(&self, signal: i32) {
        assert_eq!(
            unsafe { libc::kill(self.child.as_ref().unwrap().id() as i32, signal) },
            0
        );
    }

    fn finish(mut self) -> Output {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(status) = self.child.as_mut().unwrap().try_wait().unwrap() {
                self.child.take();
                return Output {
                    status,
                    stdout: std::fs::read(&self.stdout).unwrap(),
                    stderr: std::fs::read(&self.stderr).unwrap(),
                };
            }
            assert!(
                Instant::now() < deadline,
                "Sources did not finish; stderr: {}",
                std::fs::read_to_string(&self.stderr).unwrap()
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        if let Some(group) = self.task_group {
            unsafe {
                libc::kill(-group, libc::SIGKILL);
            }
        }
        if let Some(child) = &mut self.child {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn ready(listener: &TcpListener, running: &mut Running) -> TcpStream {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut stream = loop {
        match listener.accept() {
            Ok((stream, _)) => break stream,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(
                    running
                        .child
                        .as_mut()
                        .unwrap()
                        .try_wait()
                        .unwrap()
                        .is_none(),
                    "task exited before readiness; stderr: {}",
                    std::fs::read_to_string(&running.stderr).unwrap()
                );
                assert!(Instant::now() < deadline, "task did not reach readiness");
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(error) => panic!("cannot accept task readiness: {error}"),
        }
    };
    stream.set_nonblocking(false).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let mut bytes = [0; 5];
    stream.read_exact(&mut bytes).unwrap();
    assert_eq!(bytes[0], b'R');
    let pid = u32::from_be_bytes(bytes[1..].try_into().unwrap()) as i32;
    let group = unsafe { libc::getpgid(pid) };
    assert!(group > 0);
    assert_ne!(group, unsafe { libc::getpgrp() });
    running.task_group = Some(group);
    stream
}

fn listener() -> TcpListener {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    listener
}

#[test]
fn nonterminal_task_forwards_signals_and_holds_the_lock_through_shutdown() {
    for signal in [libc::SIGINT, libc::SIGTERM] {
        let t = TurboFixture::new();
        let listener = listener();
        let port = listener.local_addr().unwrap().port();
        std::fs::write(t.child.join("hold.cjs"), HOLD).unwrap();
        let path = t.child.join("packages/protocol/package.json");
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        manifest["scripts"]["pack"] = serde_json::json!(format!("node ../../hold.cjs {port}"));
        write_json(&path, &manifest);
        let path = t.child.join("turbo.json");
        let mut config: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        config["tasks"]["pack"]["cache"] = serde_json::json!(false);
        write_json(&path, &config);
        let mut running = Running::spawn(
            t.cli().args(["task", "@example/protocol:pack"]),
            t.f.root.join("task-signal"),
        );
        let mut stream = ready(&listener, &mut running);
        failure(
            Running::spawn(t.cli().arg("status"), t.f.root.join("busy-before-signal")).finish(),
            "locked",
        );
        running.signal(signal);
        let mut caught = [0];
        stream.read_exact(&mut caught).unwrap();
        // Turbo may translate SIGTERM into SIGINT when stopping its task subprocesses.
        assert!(matches!(caught[0], 2 | 15));
        assert!(
            running
                .child
                .as_mut()
                .unwrap()
                .try_wait()
                .unwrap()
                .is_none()
        );
        failure(
            Running::spawn(t.cli().arg("status"), t.f.root.join("busy-during-shutdown")).finish(),
            "locked",
        );
        stream.write_all(b"G").unwrap();
        let output = running.finish();
        assert_eq!(
            output.status.code(),
            Some(128 + signal),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        success(Running::spawn(t.cli().arg("status"), t.f.root.join("after-task-signal")).finish());
    }
}

#[test]
fn nonterminal_parent_turbo_preserves_failure_and_interrupt_status_without_retaining_the_lock() {
    let t = TurboFixture::new();
    t.parent();
    std::fs::write(
        t.f.app.join("fail.cjs"),
        "console.log('parent failure marker'); process.exit(37);\n",
    )
    .unwrap();
    let path = t.f.app.join("packages/web/package.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    manifest["scripts"]["inspect"] = serde_json::json!("node ../../fail.cjs");
    write_json(&path, &manifest);
    let output = Running::spawn(
        t.cli()
            .args(["turbo", "run", "inspect", "--filter=@app/web"]),
        t.f.root.join("parent-failure"),
    )
    .finish();
    assert_eq!(
        output.status.code(),
        Some(37),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("parent failure marker"));
    success(Running::spawn(t.cli().arg("status"), t.f.root.join("after-parent-failure")).finish());

    let listener = listener();
    let port = listener.local_addr().unwrap().port();
    std::fs::write(t.f.app.join("hold.cjs"), HOLD).unwrap();
    manifest["scripts"]["inspect"] = serde_json::json!(format!("node ../../hold.cjs {port}"));
    write_json(&path, &manifest);
    let mut running = Running::spawn(
        t.cli()
            .args(["turbo", "run", "inspect", "--filter=@app/web"]),
        t.f.root.join("parent-signal"),
    );
    let mut stream = ready(&listener, &mut running);
    success(Running::spawn(t.cli().arg("status"), t.f.root.join("during-parent-task")).finish());
    running.signal(libc::SIGTERM);
    let mut caught = [0];
    stream.read_exact(&mut caught).unwrap();
    assert!(matches!(caught[0], 2 | 15));
    assert!(
        running
            .child
            .as_mut()
            .unwrap()
            .try_wait()
            .unwrap()
            .is_none()
    );
    success(
        Running::spawn(
            t.cli().arg("status"),
            t.f.root.join("during-parent-shutdown"),
        )
        .finish(),
    );
    stream.write_all(b"G").unwrap();
    let output = running.finish();
    assert_eq!(
        output.status.code(),
        Some(128 + libc::SIGTERM),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    success(Running::spawn(t.cli().arg("status"), t.f.root.join("after-parent-signal")).finish());
}
