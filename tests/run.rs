#![cfg(unix)]

mod common;
use common::{Fixture, success};
use serde_json::json;
use std::io::Write;
use std::os::unix::fs::symlink;
use std::path::PathBuf;
use std::process::Stdio;

const UPSTREAM: &str = "https://example.invalid/team/repo";

fn active_dev(f: &Fixture) -> PathBuf {
    let dev = f.repo("development checkout");
    f.git(&dev, &["remote", "add", "origin", UPSTREAM]);
    f.configure(json!({"a": {"url": UPSTREAM}}), json!({"a": dev}));
    std::fs::create_dir(f.app.join("sources")).unwrap();
    symlink(&dev, f.app.join("sources/a")).unwrap();
    dev
}

#[test]
fn run_uses_active_checkout_with_literal_arguments_and_inherited_environment_and_stdio() {
    let f = Fixture::new();
    let dev = active_dev(&f);
    // A different spelling of the same checkout is still the configured development target.
    symlink(&dev, f.root.join("alias")).unwrap();
    std::fs::remove_file(f.app.join("sources/a")).unwrap();
    symlink(f.root.join("alias"), f.app.join("sources/a")).unwrap();
    let mut child = f
        .cli()
        .args([
            "run", "a", "--", "sh", "-c",
            "read -r input; printf '%s\\n' \"$PWD\" \"$RUN_VALUE\" \"$input\" \"$@\"; printf 'child stderr\\n' >&2",
            "child", "two words", "$(touch injected)", "*; exit 77", "-C", "elsewhere",
        ])
        .env("RUN_VALUE", "inherited value")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"terminal input\n")
        .unwrap();
    let out = success(child.wait_with_output().unwrap());
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        format!(
            "{}\ninherited value\nterminal input\ntwo words\n$(touch injected)\n*; exit 77\n-C\nelsewhere\n",
            dev.canonicalize().unwrap().display()
        )
    );
    assert!(String::from_utf8_lossy(&out.stderr).contains("child stderr"));
    assert!(!dev.join("injected").exists());
    std::fs::write(dev.join("tracked.txt"), "source edit\n").unwrap();
    let failed = f
        .cli()
        .args(["run", "a", "--", "sh", "-c", "exit 23"])
        .output()
        .unwrap();
    assert_eq!(failed.status.code(), Some(23));
    assert!(String::from_utf8_lossy(&failed.stderr).contains("tracked.txt"));
}

fn read_before_deadline<R: std::io::Read + std::os::fd::AsRawFd>(
    reader: &mut R,
    bytes: &mut [u8],
    deadline: std::time::Instant,
) -> std::io::Result<usize> {
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "fixture read timed out",
            ));
        }
        let mut descriptor = libc::pollfd {
            fd: reader.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let timeout = remaining.as_millis().clamp(1, i32::MAX as u128) as i32;
        if unsafe { libc::poll(&mut descriptor, 1, timeout) } == -1 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error);
        }
        if descriptor.revents == 0 {
            continue;
        }
        match reader.read(bytes) {
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                ) =>
            {
                continue;
            }
            result => return result,
        }
    }
}

fn read_handshake(
    stream: &mut std::os::unix::net::UnixStream,
    bytes: &mut [u8],
) -> std::io::Result<()> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let mut offset = 0;
    while offset < bytes.len() {
        let count = read_before_deadline(stream, &mut bytes[offset..], deadline)?;
        if count == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "fixture handshake closed",
            ));
        }
        offset += count;
    }
    Ok(())
}

fn detached_cli(f: &Fixture) -> std::process::Command {
    use std::os::unix::process::CommandExt;
    let mut command = f.cli();
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    command
}

fn spawn_ready_child(
    f: &Fixture,
    helper: &str,
) -> (std::process::Child, std::os::unix::net::UnixStream) {
    use std::os::unix::net::UnixListener;
    use std::time::{Duration, Instant};
    let socket = f.root.join("child.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    listener.set_nonblocking(true).unwrap();
    let mut child = detached_cli(f)
        .args(["run", "a", "--"])
        .arg(std::env::current_exe().unwrap())
        .args(["--exact", helper, "--nocapture"])
        .env("RUN_TEST_SOCKET", socket)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match listener.accept() {
            Ok((socket, _)) => {
                socket.set_nonblocking(false).unwrap();
                return (child, socket);
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(
                    child.try_wait().unwrap().is_none(),
                    "child exited before readiness"
                );
                if Instant::now() > deadline {
                    child.kill().unwrap();
                    child.wait().unwrap();
                    panic!("child did not become ready");
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) => panic!("cannot accept child readiness: {error}"),
        }
    }
}

#[test]
fn group_grandchild() {
    let Some(socket) = std::env::var_os("RUN_TEST_SOCKET") else {
        return;
    };
    let root = std::path::Path::new(&socket).parent().unwrap();
    let mut signals = signal_hook::iterator::Signals::new([libc::SIGINT, libc::SIGTERM]).unwrap();
    std::fs::write(root.join("grandchild.ready"), "ready").unwrap();
    let signal = signals.forever().next().unwrap();
    std::fs::write(root.join("grandchild.signal"), signal.to_string()).unwrap();
}

#[test]
fn group_child() {
    let Some(socket) = std::env::var_os("RUN_TEST_SOCKET") else {
        return;
    };
    let root = std::path::Path::new(&socket).parent().unwrap();
    let mut signals = signal_hook::iterator::Signals::new([libc::SIGINT, libc::SIGTERM]).unwrap();
    let mut descendant = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "group_grandchild", "--nocapture"])
        .spawn()
        .unwrap();
    while !root.join("grandchild.ready").exists() {
        assert!(descendant.try_wait().unwrap().is_none());
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let mut stream = std::os::unix::net::UnixStream::connect(&socket).unwrap();
    stream.write_all(b"R").unwrap();
    stream.write_all(&std::process::id().to_be_bytes()).unwrap();
    let signal = signals.forever().next().unwrap();
    assert!(descendant.wait().unwrap().success());
    stream.write_all(&[signal as u8]).unwrap();
    let mut finish = [0];
    read_handshake(&mut stream, &mut finish).unwrap();
    std::process::exit(47);
}

#[test]
fn run_forwards_to_descendants_and_keeps_lock_until_supervised_shutdown_finishes() {
    let f = Fixture::new();
    active_dev(&f);
    let (mut child, mut stream) = spawn_ready_child(&f, "group_child");
    let mut ready = [0; 5];
    read_handshake(&mut stream, &mut ready).unwrap();
    let group = u32::from_be_bytes(ready[1..].try_into().unwrap()) as i32;
    assert_eq!(unsafe { libc::kill(child.id() as i32, libc::SIGTERM) }, 0);
    let mut caught = [0];
    if let Err(error) = read_handshake(&mut stream, &mut caught) {
        unsafe {
            libc::kill(-group, libc::SIGKILL);
        }
        let _ = child.kill();
        child.wait().unwrap();
        panic!("child and descendant did not receive termination: {error}");
    }
    assert_eq!(caught, [libc::SIGTERM as u8]);
    assert_eq!(
        std::fs::read_to_string(f.root.join("grandchild.signal")).unwrap(),
        libc::SIGTERM.to_string()
    );
    assert!(child.try_wait().unwrap().is_none());
    common::failure(f.cli().arg("check").output().unwrap(), "locked");
    stream.write_all(b"G").unwrap();
    assert_eq!(child.wait().unwrap().code(), Some(47));
    success(f.cli().arg("check").output().unwrap());
}
