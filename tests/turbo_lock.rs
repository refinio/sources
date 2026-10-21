mod common;
#[path = "common/turbo.rs"]
mod turbo_fixture;

use common::{failure, success};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};
use turbo_fixture::{TurboFixture, write_json};

struct Running(Option<Child>);

impl Running {
    fn spawn(command: &mut Command) -> Self {
        Self(Some(
            command
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap(),
        ))
    }

    fn finish(mut self) -> Output {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if self.0.as_mut().unwrap().try_wait().unwrap().is_some() {
                return self.0.take().unwrap().wait_with_output().unwrap();
            }
            assert!(Instant::now() < deadline, "Sources process did not finish");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        if let Some(child) = &mut self.0 {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn accept_task(listener: &TcpListener, child: &mut Running, expected: &str) -> TcpStream {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut stream = loop {
        match listener.accept() {
            Ok((stream, _)) => break stream,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(
                    child.0.as_mut().unwrap().try_wait().unwrap().is_none(),
                    "task exited before its readiness barrier"
                );
                assert!(
                    Instant::now() < deadline,
                    "task did not reach its readiness barrier"
                );
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(error) => panic!("cannot accept task readiness: {error}"),
        }
    };
    stream.set_nonblocking(false).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let mut ready = vec![0; expected.len()];
    stream.read_exact(&mut ready).unwrap();
    assert_eq!(ready, expected.as_bytes());
    stream
}

#[test]
fn concurrent_task_commands_wait_for_the_lock_while_other_commands_fail_fast() {
    let t = TurboFixture::new();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    std::fs::write(
        t.child.join("lock-task.cjs"),
        r#"
const fs = require('node:fs');
const net = require('node:net');
const name = process.argv[3];
const socket = net.createConnection(Number(process.argv[2]), '127.0.0.1', () => {
  fs.mkdirSync('dist', {recursive:true});
  fs.writeFileSync('dist/started.txt', name);
  socket.write(name);
});
socket.on('data', () => {
  fs.mkdirSync('dist', {recursive:true});
  fs.writeFileSync('dist/pack.txt', name);
  socket.end();
  process.exit(0);
});
socket.on('close', () => process.exit(0));
socket.on('error', () => process.exit(71));
"#,
    )
    .unwrap();
    for name in ["protocol", "ui"] {
        let path = t.child.join("packages").join(name).join("package.json");
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        manifest["scripts"]["pack"] =
            serde_json::json!(format!("node ../../lock-task.cjs {port} {name}"));
        write_json(&path, &manifest);
    }
    let path = t.child.join("turbo.json");
    let mut turbo: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    turbo["tasks"]["pack"]["cache"] = serde_json::json!(false);
    write_json(&path, &turbo);

    let mut first = Running::spawn(t.cli().args(["task", "@example/protocol:pack"]));
    let mut first_ready = accept_task(&listener, &mut first, "protocol");
    let mut second = Running::spawn(t.cli().args(["task", "@example/ui:pack"]));
    let stderr = second.0.as_mut().unwrap().stderr.take().unwrap();
    let (send, receive) = std::sync::mpsc::channel();
    let stderr_reader = std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines() {
            let line = line.unwrap();
            eprintln!("second task: {line}");
            let _ = send.send(line);
        }
    });
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let line = receive
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .expect("second task did not announce that it was waiting for the project lock");
        if line.contains("waiting for project lock at") {
            break;
        }
    }
    assert!(!t.marker("ui", "started").exists());
    assert!(!t.marker("ui", "pack").exists());
    failure(Running::spawn(t.cli().arg("status")).finish(), "locked");
    first_ready.write_all(b"G").unwrap();
    success(first.finish());

    let mut second_ready = accept_task(&listener, &mut second, "ui");
    assert!(t.marker("ui", "started").is_file());
    assert!(t.marker("protocol", "pack").is_file());
    assert!(!t.marker("ui", "pack").exists());
    second_ready.write_all(b"G").unwrap();
    success(second.finish());
    stderr_reader.join().unwrap();
    assert!(t.marker("ui", "pack").is_file());
    success(Running::spawn(t.cli().arg("status")).finish());
}
