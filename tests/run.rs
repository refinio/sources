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
}

#[test]
fn run_warns_before_and_after_source_changes_and_preserves_child_failure() {
    let f = Fixture::new();
    let dev = active_dev(&f);
    std::fs::write(dev.join("tracked.txt"), "before\n").unwrap();
    let out = f.cli().args([
        "run", "a", "--", "sh", "-c",
        "printf 'child boundary\\n' >&2; printf 'after\\n' > new.txt; printf ignored > artifact.txt; exit 23",
    ]).output().unwrap();
    assert_eq!(out.status.code(), Some(23));
    let stderr = String::from_utf8_lossy(&out.stderr);
    let (before, after) = stderr.split_once("child boundary").unwrap();
    assert!(
        before.contains("warning: a:") && before.contains("tracked.txt"),
        "{stderr}"
    );
    assert!(
        after.contains("warning: a:") && after.contains("tracked.txt") && after.contains("new.txt"),
        "{stderr}"
    );
    assert!(!stderr.contains("artifact.txt"), "{stderr}");
}

#[test]
fn run_accepts_managed_checkout_without_development_mapping_or_pins() {
    let f = Fixture::new();
    let managed = f.repo("app/.sources/checkouts/a");
    f.git(&managed, &["remote", "add", "origin", UPSTREAM]);
    f.configure(json!({"a": {"url": UPSTREAM}}), json!({}));
    std::fs::create_dir(f.app.join("sources")).unwrap();
    symlink(&managed, f.app.join("sources/a")).unwrap();
    let out = success(
        f.cli()
            .args(["run", "a", "--", "git", "rev-parse", "--show-toplevel"])
            .output()
            .unwrap(),
    );
    assert_eq!(
        String::from_utf8(out.stdout).unwrap().trim(),
        managed.canonicalize().unwrap().to_str().unwrap()
    );
    assert!(!f.app.join(".sources.lock").exists());
}

#[test]
fn run_does_not_start_a_child_from_a_linked_unborn_managed_cache() {
    let f = Fixture::new();
    let managed = f.app.join(".sources/checkouts/a");
    std::fs::create_dir_all(&managed).unwrap();
    f.git(&managed, &["init"]);
    f.git(&managed, &["remote", "add", "origin", UPSTREAM]);
    f.configure(json!({"a": {"url": UPSTREAM}}), json!({}));
    std::fs::create_dir(f.app.join("sources")).unwrap();
    symlink(&managed, f.app.join("sources/a")).unwrap();
    common::failure(
        f.cli()
            .args(["run", "a", "--", "sh", "-c", "touch child-started"])
            .output()
            .unwrap(),
        "sources release",
    );
    assert!(!managed.join("child-started").exists());
}

#[test]
fn run_rejects_incomplete_recursive_managed_modules_until_release_repairs_them() {
    let f = Fixture::new();
    let nested = f.repo("nested");
    let first = f.git(&nested, &["rev-parse", "HEAD"]);
    std::fs::write(nested.join("tracked.txt"), "second\n").unwrap();
    f.git(&nested, &["commit", "-qam", "second"]);
    let wanted = f.git(&nested, &["rev-parse", "HEAD"]);
    let module = f.repo("module");
    f.git(
        &module,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            "../nested",
            "nested",
        ],
    );
    f.git(&module, &["commit", "-qam", "nested input"]);
    let upstream = f.repo("upstream");
    f.git(
        &upstream,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            "../module",
            "dep",
        ],
    );
    f.git(&upstream, &["commit", "-qam", "module input"]);
    let parent_pin = f.git(&upstream, &["rev-parse", "HEAD"]);
    f.configure(json!({"a": {"url": upstream}}), json!({}));
    std::fs::write(
        f.app.join(".sources.lock"),
        json!({"version": 2, "sources": {"a": {"git": parent_pin}}}).to_string(),
    )
    .unwrap();
    success(f.cli().arg("release").output().unwrap());
    let managed = f.app.join(".sources/checkouts/a");
    let nested_worktree = managed.join("dep/nested");
    std::fs::remove_dir_all(&nested_worktree).unwrap();
    common::failure(
        f.cli()
            .args(["run", "a", "--", "sh", "-c", "touch child-started"])
            .output()
            .unwrap(),
        "sources release",
    );
    assert!(!managed.join("child-started").exists());
    success(f.cli().args(["release", "--offline"]).output().unwrap());
    f.git(&nested_worktree, &["fetch", "--depth=1", "origin", &first]);
    f.git(&nested_worktree, &["checkout", "--detach", &first]);
    common::failure(
        f.cli()
            .args(["run", "a", "--", "sh", "-c", "touch child-started"])
            .output()
            .unwrap(),
        "sources release",
    );
    assert!(!managed.join("child-started").exists());
    success(f.cli().args(["release", "--offline"]).output().unwrap());
    assert_eq!(f.git(&nested_worktree, &["rev-parse", "HEAD"]), wanted);
    std::fs::write(managed.join("tracked.txt"), "allowed source edit\n").unwrap();
    let output = f
        .cli()
        .args([
            "run",
            "a",
            "--",
            "sh",
            "-c",
            "test -f dep/nested/tracked.txt || exit 73; exit 17",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(17));
    assert!(String::from_utf8_lossy(&output.stderr).contains("tracked.txt"));
}

#[test]
fn run_refuses_missing_broken_unexpected_or_ordinary_workspace_paths_and_wrong_upstream() {
    use common::failure;
    let f = Fixture::new();
    let dev = active_dev(&f);
    let link = f.app.join("sources/a");
    let mut run = f.cli();
    run.args(["run", "a", "--", "sh", "-c", "touch ran"]);
    std::fs::remove_file(&link).unwrap();
    failure(run.output().unwrap(), "missing workspace link");
    std::fs::write(&link, "ordinary file").unwrap();
    failure(run.output().unwrap(), "must be a link");
    assert_eq!(std::fs::read_to_string(&link).unwrap(), "ordinary file");
    std::fs::remove_file(&link).unwrap();
    std::fs::create_dir(&link).unwrap();
    failure(run.output().unwrap(), "must be a link");
    std::fs::remove_dir(&link).unwrap();
    symlink(f.root.join("missing"), &link).unwrap();
    failure(run.output().unwrap(), "broken workspace link");
    std::fs::remove_file(&link).unwrap();
    let other = f.repo("other");
    f.git(&other, &["remote", "add", "origin", UPSTREAM]);
    symlink(&other, &link).unwrap();
    failure(run.output().unwrap(), "unexpected workspace link target");
    assert!(!other.join("ran").exists());
    std::fs::remove_file(&link).unwrap();
    symlink(&dev, &link).unwrap();
    f.git(
        &dev,
        &[
            "remote",
            "set-url",
            "origin",
            "https://example.invalid/wrong/repo",
        ],
    );
    failure(run.output().unwrap(), "repository mismatch");
    failure(
        f.cli()
            .args(["run", "unknown", "--", "true"])
            .output()
            .unwrap(),
        "unknown source",
    );
    assert!(!dev.join("ran").exists());
}

#[test]
fn run_reports_spawn_failure_as_tool_error() {
    let f = Fixture::new();
    active_dev(&f);
    common::failure(
        f.cli()
            .args(["run", "a", "--", "sources-nonexistent-command-234f"])
            .output()
            .unwrap(),
        "cannot start command",
    );
}

#[test]
fn run_preserves_child_status_even_if_child_breaks_post_execution_inspection() {
    for code in [0, 37] {
        let f = Fixture::new();
        active_dev(&f);
        let out = f
            .cli()
            .args([
                "run",
                "a",
                "--",
                "sh",
                "-c",
                &format!("rm -rf .git; exit {code}"),
            ])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(code));
        assert!(
            String::from_utf8_lossy(&out.stderr).contains("cannot inspect source after command")
        );
    }
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

fn pseudoterminal() -> (std::fs::File, std::fs::File) {
    use std::os::fd::FromRawFd;
    let mut master = -1;
    let mut slave = -1;
    assert_eq!(
        unsafe {
            libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        },
        0
    );
    for fd in [master, slave] {
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
        assert_ne!(flags, -1);
        assert_eq!(
            unsafe { libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) },
            0
        );
    }
    let flags = unsafe { libc::fcntl(master, libc::F_GETFL) };
    assert_ne!(flags, -1);
    assert_eq!(
        unsafe { libc::fcntl(master, libc::F_SETFL, flags | libc::O_NONBLOCK) },
        0
    );
    unsafe {
        (
            std::fs::File::from_raw_fd(master),
            std::fs::File::from_raw_fd(slave),
        )
    }
}

fn kill_shell(shell: &mut std::process::Child) {
    if let Err(error) = shell.kill()
        && shell.try_wait().unwrap().is_none()
    {
        eprintln!("cannot kill fixture process {}: {error}", shell.id());
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
    while shell.try_wait().unwrap().is_none() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    if shell.try_wait().unwrap().is_none() {
        eprintln!(
            "fixture process {} was not reaped after SIGKILL",
            shell.id()
        );
    }
}

fn wait_pty_child(
    child: &mut std::process::Child,
    master: &mut std::fs::File,
    phase: &str,
    drain_terminal: bool,
) -> std::process::ExitStatus {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut output = String::new();
    loop {
        // A PTY slave can wait for output to drain during final shell shutdown.
        if drain_terminal {
            output.push_str(&available_shell_output(master));
        }
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        if std::time::Instant::now() >= deadline {
            output.push_str(&available_shell_output(master));
            kill_shell(child);
            panic!(
                "{phase}: process {} did not exit within 5 seconds; terminal output: {output}",
                child.id()
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

fn pty_command_output(
    command: &mut std::process::Command,
    master: &mut std::fs::File,
    phase: &str,
) -> std::process::Output {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let status = wait_pty_child(&mut child, master, phase, false);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let stdout =
        capture_pty_command_pipe(&mut child.stdout.take().unwrap(), master, deadline, phase);
    let stderr =
        capture_pty_command_pipe(&mut child.stderr.take().unwrap(), master, deadline, phase);
    std::process::Output {
        status,
        stdout,
        stderr,
    }
}

fn capture_pty_command_pipe<R: std::io::Read + std::os::fd::AsRawFd>(
    reader: &mut R,
    master: &mut std::fs::File,
    deadline: std::time::Instant,
    phase: &str,
) -> Vec<u8> {
    let mut output = Vec::new();
    loop {
        let mut bytes = [0; 4096];
        match read_before_deadline(reader, &mut bytes, deadline) {
            Ok(0) => return output,
            Ok(count) => output.extend_from_slice(&bytes[..count]),
            Err(error) => panic!(
                "{phase}: captured output did not close: {error}; terminal output: {}",
                available_shell_output(master)
            ),
        }
    }
}

fn available_shell_output(master: &mut std::fs::File) -> String {
    let mut output = Vec::new();
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(10);
    loop {
        let mut bytes = [0; 4096];
        match read_before_deadline(master, &mut bytes, deadline) {
            Ok(0) | Err(_) => break,
            Ok(count) => output.extend_from_slice(&bytes[..count]),
        }
    }
    String::from_utf8_lossy(&output).into_owned()
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
fn blocked_child() {
    let Some(socket) = std::env::var_os("RUN_TEST_SOCKET") else {
        return;
    };
    let mut stream = std::os::unix::net::UnixStream::connect(socket).unwrap();
    stream.write_all(b"R").unwrap();
    stream.write_all(&std::process::id().to_be_bytes()).unwrap();
    let mut byte = [0];
    read_handshake(&mut stream, &mut byte).unwrap();
    std::fs::write("new.txt", "child source edit\n").unwrap();
}

#[test]
fn run_holds_project_lock_while_child_is_blocked_and_releases_it_after_post_check() {
    let f = Fixture::new();
    active_dev(&f);
    let (child, mut stream) = spawn_ready_child(&f, "blocked_child");
    let mut ready = [0];
    read_handshake(&mut stream, &mut ready).unwrap();
    assert_eq!(ready, [b'R']);
    common::failure(f.cli().arg("check").output().unwrap(), "locked");
    stream.write_all(b"G").unwrap();
    let out = success(child.wait_with_output().unwrap());
    assert!(String::from_utf8_lossy(&out.stderr).contains("new.txt"));
    success(f.cli().arg("check").output().unwrap());
}

#[test]
fn run_forwards_interrupt_and_termination_and_returns_signaled_child_status() {
    use std::time::{Duration, Instant};
    for signal in [libc::SIGINT, libc::SIGTERM] {
        let f = Fixture::new();
        active_dev(&f);
        let (mut child, mut stream) = spawn_ready_child(&f, "blocked_child");
        let mut ready = [0; 5];
        read_handshake(&mut stream, &mut ready).unwrap();
        assert_eq!(ready[0], b'R');
        let child_pid = u32::from_be_bytes(ready[1..].try_into().unwrap()) as i32;
        // Signal only sources, so the child cannot receive it from the test's process group.
        assert_eq!(unsafe { libc::kill(child.id() as i32, signal) }, 0);
        let deadline = Instant::now() + Duration::from_secs(10);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if Instant::now() > deadline {
                unsafe {
                    libc::kill(child_pid, libc::SIGKILL);
                }
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("sources did not supervise signal {signal}");
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        // Clean up the test helper if an incorrect implementation orphaned it.
        if status.code() != Some(128 + signal) {
            unsafe {
                libc::kill(child_pid, libc::SIGKILL);
            }
        }
        assert_eq!(status.code(), Some(128 + signal));
        success(f.cli().arg("check").output().unwrap());
    }
}

#[test]
fn run_child_can_read_from_the_controlling_terminal() {
    terminal_input(false);
}

#[test]
fn interactive_run_mirrors_a_direct_child_stop_and_continues_it_with_the_lock_held() {
    terminal_input(true);
}

fn terminal_input(self_stop: bool) {
    use std::os::unix::process::CommandExt;
    use std::time::{Duration, Instant};
    let f = Fixture::new();
    active_dev(&f);
    let pid_file = f.root.join("terminal-child.pid");
    let (mut master, slave) = pseudoterminal();
    let mut command = f.cli();
    command
        .args([
            "run",
            "a",
            "--",
            "sh",
            "-c",
            if self_stop {
                "printf '%s' $$ > \"$1\"; kill -STOP $$; read -r input; printf 'read:%s\\n' \"$input\""
            } else {
                "printf '%s' $$ > \"$1\"; read -r input; printf 'read:%s\\n' \"$input\""
            },
            "child",
        ])
        .arg(&pid_file)
        .stdin(slave.try_clone().unwrap())
        .stdout(slave.try_clone().unwrap())
        .stderr(slave);
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 || libc::ioctl(0, libc::TIOCSCTTY as _, 0) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = command.spawn().unwrap();
    let locked = if self_stop {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let mut raw = 0;
            let result = unsafe {
                libc::waitpid(child.id() as i32, &mut raw, libc::WNOHANG | libc::WUNTRACED)
            };
            if result > 0 && libc::WIFSTOPPED(raw) {
                break;
            }
            if Instant::now() >= deadline {
                unsafe { libc::kill(-(child.id() as i32), libc::SIGKILL) };
                kill_shell(&mut child);
                panic!("sources did not mirror its stopped direct child");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let output = pty_command_output(
            f.cli().arg("check"),
            &mut master,
            "source check around direct-child suspension",
        );
        assert_eq!(unsafe { libc::kill(child.id() as i32, libc::SIGCONT) }, 0);
        Some(output)
    } else {
        None
    };
    master.write_all(b"terminal input\n").unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut output = Vec::new();
    let status = loop {
        let mut bytes = [0; 4096];
        let poll_deadline = deadline.min(Instant::now() + Duration::from_millis(10));
        if let Ok(count) = read_before_deadline(&mut master, &mut bytes, poll_deadline) {
            output.extend_from_slice(&bytes[..count]);
        }
        if let Some(status) = child.try_wait().unwrap() {
            break Some(status);
        }
        if Instant::now() > deadline {
            unsafe { libc::kill(-(child.id() as i32), libc::SIGKILL) };
            let _ = child.kill();
            kill_shell(&mut child);
            break None;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(
        status.is_some_and(|status| status.success()),
        "terminal child failed or stopped: {}",
        String::from_utf8_lossy(&output)
    );
    if let Some(locked) = locked {
        common::failure(locked, "locked");
        success(pty_command_output(
            f.cli().arg("check"),
            &mut master,
            "source check around direct-child suspension",
        ));
    }
    assert!(
        String::from_utf8_lossy(&output).contains("read:terminal input"),
        "{}",
        String::from_utf8_lossy(&output)
    );
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
    if std::env::var_os("RUN_ORPHAN_CLEANUP").is_some() {
        let mut stream = std::os::unix::net::UnixStream::connect(&socket).unwrap();
        stream.write_all(b"R").unwrap();
        stream.write_all(&std::process::id().to_be_bytes()).unwrap();
        let mut finish = [0];
        read_handshake(&mut stream, &mut finish).unwrap();
    }
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

#[test]
fn run_refuses_managed_path_aliases_into_external_checkouts() {
    let f = Fixture::new();
    let external = f.repo("external");
    f.git(&external, &["remote", "add", "origin", UPSTREAM]);
    f.configure(json!({"a": {"url": UPSTREAM}}), json!({}));
    std::fs::create_dir_all(f.app.join(".sources/checkouts")).unwrap();
    std::fs::create_dir(f.app.join("sources")).unwrap();
    symlink(&external, f.app.join(".sources/checkouts/a")).unwrap();
    symlink(f.app.join(".sources/checkouts/a"), f.app.join("sources/a")).unwrap();
    common::failure(
        f.cli()
            .args(["run", "a", "--", "sh", "-c", "touch ran"])
            .output()
            .unwrap(),
        "ordinary clone",
    );
    assert!(!external.join("ran").exists());
}

#[test]
fn orphaning_child() {
    let Some(socket) = std::env::var_os("RUN_TEST_SOCKET") else {
        return;
    };
    let root = std::path::Path::new(&socket).parent().unwrap();
    let mut descendant = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "group_grandchild", "--nocapture"])
        .env("RUN_ORPHAN_CLEANUP", "1")
        .spawn()
        .unwrap();
    while !root.join("grandchild.ready").exists() {
        assert!(descendant.try_wait().unwrap().is_none());
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let mut stream = std::os::unix::net::UnixStream::connect(&socket).unwrap();
    stream.write_all(b"R").unwrap();
    stream.write_all(&std::process::id().to_be_bytes()).unwrap();
    // Let the caller know the leader has exited before it signals the remaining group.
    std::process::exit(31);
}

#[test]
fn run_keeps_lock_for_remaining_group_after_direct_child_exits() {
    use std::time::{Duration, Instant};
    let f = Fixture::new();
    active_dev(&f);
    let child_socket = f.root.join("child.sock");
    let ready_listener = std::os::unix::net::UnixListener::bind(&child_socket).unwrap();
    ready_listener.set_nonblocking(true).unwrap();
    let mut child = detached_cli(&f)
        .args(["run", "a", "--"])
        .arg(std::env::current_exe().unwrap())
        .args(["--exact", "orphaning_child", "--nocapture"])
        .env("RUN_TEST_SOCKET", &child_socket)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut ready = loop {
        if let Ok((stream, _)) = ready_listener.accept() {
            stream.set_nonblocking(false).unwrap();
            break stream;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    };
    let mut header = [0; 5];
    read_handshake(&mut ready, &mut header).unwrap();
    let group = u32::from_be_bytes(header[1..].try_into().unwrap()) as i32;
    // EOF proves the direct child has exited. The descendant is still alive in its group.
    assert_eq!(
        read_before_deadline(
            &mut ready,
            &mut [0],
            Instant::now() + Duration::from_secs(10)
        )
        .unwrap(),
        0
    );
    let forwarded = unsafe { libc::kill(child.id() as i32, libc::SIGTERM) };
    if forwarded == -1 {
        unsafe {
            libc::kill(-group, libc::SIGTERM);
        }
    }
    // The descendant announces that it is holding its shutdown handshake open.
    let mut cleanup = loop {
        if let Ok((stream, _)) = ready_listener.accept() {
            stream.set_nonblocking(false).unwrap();
            break stream;
        }
        if Instant::now() >= deadline {
            unsafe {
                libc::kill(-group, libc::SIGKILL);
            }
            let _ = child.kill();
            child.wait().unwrap();
            panic!("sources did not forward termination after the direct child exited");
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    read_handshake(&mut cleanup, &mut header).unwrap();
    let lock_result = f.cli().arg("check").output().unwrap();
    let exited_early = child.try_wait().unwrap().is_some();
    cleanup.write_all(b"G").unwrap();
    assert_eq!(child.wait().unwrap().code(), Some(31));
    common::failure(lock_result, "locked");
    assert_eq!(forwarded, 0);
    assert!(
        !exited_early,
        "sources exited while a process-group member was still running"
    );
}

#[test]
fn run_refuses_managed_parent_directory_aliases_into_external_checkouts() {
    let f = Fixture::new();
    let external = f.repo("external/a");
    f.git(&external, &["remote", "add", "origin", UPSTREAM]);
    f.configure(json!({"a": {"url": UPSTREAM}}), json!({}));
    std::fs::create_dir_all(f.app.join(".sources")).unwrap();
    std::fs::create_dir(f.app.join("sources")).unwrap();
    symlink(f.root.join("external"), f.app.join(".sources/checkouts")).unwrap();
    symlink(&external, f.app.join("sources/a")).unwrap();
    common::failure(
        f.cli()
            .args(["run", "a", "--", "sh", "-c", "touch ran"])
            .output()
            .unwrap(),
        "must be a real directory",
    );
    assert!(!external.join("ran").exists());
}

#[cfg(target_os = "linux")]
#[test]
fn run_reaps_orphans_without_relying_on_container_init() {
    if std::env::var_os("RUN_ZOMBIE_CASE").is_none() {
        success(
            std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "run_reaps_orphans_without_relying_on_container_init",
                    "--nocapture",
                ])
                .env("RUN_ZOMBIE_CASE", "1")
                .output()
                .unwrap(),
        );
        return;
    }
    // Isolate a non-reaping ancestor so host init cannot hide orphan zombies.
    assert_eq!(
        unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0) },
        0
    );
    let f = Fixture::new();
    active_dev(&f);
    let mut child = detached_cli(&f)
        .args(["run", "a", "--", "sh", "-c", "sleep 0.05 & exit 31"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break Some(status);
        }
        if std::time::Instant::now() > deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            break None;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    };
    unsafe { while libc::waitpid(-1, std::ptr::null_mut(), libc::WNOHANG) > 0 {} }
    assert_eq!(
        status.and_then(|status| status.code()),
        Some(31),
        "sources waited indefinitely for an orphan zombie"
    );
}

#[test]
fn terminal_reader() {
    let Some(ready) = std::env::var_os("RUN_TERMINAL_READY") else {
        return;
    };
    let mut signals = signal_hook::iterator::Signals::new([libc::SIGCONT]).unwrap();
    std::fs::write(&ready, "ready").unwrap();
    if std::env::var_os("RUN_START_BACKGROUND").is_none() {
        signals.forever().next().unwrap();
    }
    std::fs::write(
        std::path::Path::new(&ready).with_extension("continued"),
        "continued",
    )
    .unwrap();
    let mut input = String::new();
    std::io::stdin().read_line(&mut input).unwrap();
    print!("read:{input}");
}

fn shell_output_until(
    master: &mut std::fs::File,
    shell: &mut std::process::Child,
    expected: &str,
) -> String {
    use std::os::fd::AsRawFd;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut output = Vec::new();
    loop {
        let mut bytes = [0; 4096];
        if let Ok(count) = read_before_deadline(master, &mut bytes, deadline) {
            output.extend_from_slice(&bytes[..count]);
        }
        if String::from_utf8_lossy(&output).contains(expected) {
            return String::from_utf8(output).unwrap();
        }
        if std::time::Instant::now() > deadline {
            let foreground = unsafe { libc::tcgetpgrp(master.as_raw_fd()) };
            if foreground > 0 {
                unsafe { libc::kill(-foreground, libc::SIGKILL) };
            }
            kill_shell(shell);
            panic!(
                "shell did not output {expected:?}: {}",
                String::from_utf8_lossy(&output)
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

#[test]
fn run_pipeline_can_suspend_background_and_return_to_foreground() {
    shell_pipeline(false);
}

#[test]
fn run_pipeline_started_in_background_can_return_to_foreground() {
    shell_pipeline(true);
}

fn interactive_shell(f: &Fixture) -> (std::fs::File, std::process::Command) {
    use std::os::fd::AsRawFd;
    use std::os::unix::process::CommandExt;
    let (master, slave) = pseudoterminal();
    let mut attributes = unsafe { std::mem::zeroed() };
    assert_eq!(
        unsafe { libc::tcgetattr(slave.as_raw_fd(), &mut attributes) },
        0
    );
    attributes.c_lflag &= !libc::ECHO;
    assert_eq!(
        unsafe { libc::tcsetattr(slave.as_raw_fd(), libc::TCSANOW, &attributes) },
        0
    );
    // Bash-based /bin/sh must execute fixture lines without history expansion.
    let startup = f.root.join("interactive-shell.conf");
    std::fs::write(
        &startup,
        "if [ -n \"${BASH_VERSION-}\" ]; then set +H; fi\n",
    )
    .unwrap();
    let mut command = f.command("sh");
    command
        .current_dir(&f.app)
        .arg("-i")
        .env("ENV", startup)
        .env("PS1", "SOURCES-PROMPT> ")
        .env("SOURCES_BIN", assert_cmd::cargo::cargo_bin!("sources"))
        .env("RUN_TEST_BIN", std::env::current_exe().unwrap())
        .env("RUN_TERMINAL_READY", f.root.join("terminal.ready"))
        .env("RUN_FIXTURE_ROOT", &f.root)
        .stdin(slave.try_clone().unwrap())
        .stdout(slave.try_clone().unwrap())
        .stderr(slave);
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 || libc::ioctl(0, libc::TIOCSCTTY as _, 0) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    (master, command)
}

fn shell_pipeline(start_background: bool) {
    use std::os::fd::AsRawFd;
    use std::time::{Duration, Instant};
    let f = Fixture::new();
    active_dev(&f);
    let ready = f.root.join("terminal.ready");
    let (mut master, mut command) = interactive_shell(&f);
    if start_background {
        command.env("RUN_START_BACKGROUND", "1");
    }
    let mut shell = command.spawn().unwrap();
    shell_output_until(&mut master, &mut shell, "SOURCES-PROMPT>");
    let background = if start_background { " &" } else { "" };
    writeln!(master, "\"$SOURCES_BIN\" run a -- \"$RUN_TEST_BIN\" --exact terminal_reader --nocapture | cat{background}").unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !ready.exists() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    if !start_background {
        master.write_all(&[26]).unwrap();
        shell_output_until(&mut master, &mut shell, "SOURCES-PROMPT>");
        master.write_all(b"bg\n").unwrap();
    }
    shell_output_until(&mut master, &mut shell, "SOURCES-PROMPT>");
    while !ready.with_extension("continued").exists() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    common::failure(
        pty_command_output(
            f.cli().arg("check"),
            &mut master,
            "source check during interactive job",
        ),
        "locked",
    );
    let background_foreground = unsafe { libc::tcgetpgrp(master.as_raw_fd()) };
    // Wait until terminal input has stopped the background job before running fg.
    loop {
        master.write_all(b"jobs\n").unwrap();
        let output = shell_output_until(&mut master, &mut shell, "SOURCES-PROMPT>");
        if output.contains("Stopped") {
            break;
        }
        if Instant::now() > deadline {
            unsafe {
                libc::kill(-background_foreground, libc::SIGKILL);
            }
            kill_shell(&mut shell);
            panic!("background job did not stop for terminal input: {output}");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    master.write_all(b"fg\nresumed input\n").unwrap();
    let output = shell_output_until(&mut master, &mut shell, "read:resumed input");
    if !output.contains("SOURCES-PROMPT>") {
        shell_output_until(&mut master, &mut shell, "SOURCES-PROMPT>");
    }
    master.write_all(b"exit\n").unwrap();
    assert!(
        wait_pty_child(
            &mut shell,
            &mut master,
            "interactive shell after exit command",
            true,
        )
        .success()
    );
    assert_eq!(
        background_foreground,
        shell.id() as i32,
        "background job stole terminal ownership"
    );
}

#[test]
fn interactive_pipeline_interrupt_reaches_the_wrapped_child_and_its_sibling() {
    use std::time::{Duration, Instant};
    let f = Fixture::new();
    active_dev(&f);
    let (mut master, mut command) = interactive_shell(&f);
    let mut shell = command.spawn().unwrap();
    shell_output_until(&mut master, &mut shell, "SOURCES-PROMPT>");
    writeln!(master, "\"$SOURCES_BIN\" run a -- sh -c 'echo $$ > \"$RUN_FIXTURE_ROOT/wrapped.pid\"; exec sleep 300' | sh -c 'echo $$ > \"$RUN_FIXTURE_ROOT/sibling.pid\"; exec sleep 300'").unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !f.root.join("wrapped.pid").exists() || !f.root.join("sibling.pid").exists() {
        assert!(
            Instant::now() < deadline,
            "pipeline did not become ready: {}",
            available_shell_output(&mut master)
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    master.write_all(&[3]).unwrap();
    shell_output_until(&mut master, &mut shell, "SOURCES-PROMPT>");
    success(pty_command_output(
        f.cli().arg("check"),
        &mut master,
        "source check during interactive job",
    ));
    master.write_all(b"exit 0\n").unwrap();
    assert!(
        wait_pty_child(
            &mut shell,
            &mut master,
            "interactive shell after exit command",
            true,
        )
        .success()
    );
}

#[test]
fn interactive_pipeline_sibling_can_read_the_controlling_terminal() {
    use std::time::{Duration, Instant};
    let f = Fixture::new();
    active_dev(&f);
    let (mut master, mut command) = interactive_shell(&f);
    let mut shell = command.spawn().unwrap();
    shell_output_until(&mut master, &mut shell, "SOURCES-PROMPT>");
    writeln!(master, "\"$SOURCES_BIN\" run a -- sh -c 'echo $$ > \"$RUN_FIXTURE_ROOT/wrapped.pid\"; exec sleep 300' | sh -c 'echo $$ > \"$RUN_FIXTURE_ROOT/sibling.pid\"; while ! test -f \"$RUN_FIXTURE_ROOT/wrapped.pid\"; do sleep 0.01; done; read -r value </dev/tty; printf \"sibling-read:%s\\n\" \"$value\" >&2; kill -TERM \"$(cat \"$RUN_FIXTURE_ROOT/wrapped.pid\")\"'").unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !f.root.join("wrapped.pid").exists() || !f.root.join("sibling.pid").exists() {
        assert!(
            Instant::now() < deadline,
            "pipeline did not become ready: {}",
            available_shell_output(&mut master)
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    common::failure(
        pty_command_output(
            f.cli().arg("check"),
            &mut master,
            "source check during interactive job",
        ),
        "locked",
    );
    master.write_all(b"terminal input\n").unwrap();
    let output = shell_output_until(&mut master, &mut shell, "sibling-read:terminal input");
    if !output.contains("SOURCES-PROMPT>") {
        shell_output_until(&mut master, &mut shell, "SOURCES-PROMPT>");
    }
    success(pty_command_output(
        f.cli().arg("check"),
        &mut master,
        "source check during interactive job",
    ));
    master.write_all(b"exit 0\n").unwrap();
    assert!(
        wait_pty_child(
            &mut shell,
            &mut master,
            "interactive shell after exit command",
            true,
        )
        .success()
    );
}

#[test]
fn interactive_run_uses_job_group_interrupts_and_forwards_direct_termination() {
    use std::time::{Duration, Instant};
    for signal in [libc::SIGINT, libc::SIGTERM] {
        let f = Fixture::new();
        active_dev(&f);
        let socket = f.root.join("child.sock");
        let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        listener.set_nonblocking(true).unwrap();
        let (mut master, mut command) = interactive_shell(&f);
        command.env("RUN_TEST_SOCKET", &socket);
        let mut shell = command.spawn().unwrap();
        shell_output_until(&mut master, &mut shell, "SOURCES-PROMPT>");
        writeln!(master, "\"$SOURCES_BIN\" run a -- \"$RUN_TEST_BIN\" --exact blocked_child --nocapture & run_pid=$!; echo \"$run_pid\" > \"$RUN_FIXTURE_ROOT/sources.pid\"").unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut stream = loop {
            if let Ok((stream, _)) = listener.accept() {
                stream.set_nonblocking(false).unwrap();
                break stream;
            }
            assert!(
                Instant::now() < deadline,
                "interactive child did not become ready: {}",
                available_shell_output(&mut master)
            );
            std::thread::sleep(Duration::from_millis(10));
        };
        let mut ready = [0; 5];
        read_handshake(&mut stream, &mut ready).unwrap();
        let source_pid: i32 = std::fs::read_to_string(f.root.join("sources.pid"))
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        common::failure(
            pty_command_output(
                f.cli().arg("check"),
                &mut master,
                "source check during interactive job",
            ),
            "locked",
        );
        let target = if signal == libc::SIGINT {
            // Interactive interruption belongs to the complete shell job.
            -unsafe { libc::getpgid(source_pid) }
        } else {
            source_pid
        };
        assert_eq!(unsafe { libc::kill(target, signal) }, 0);
        assert_eq!(
            read_before_deadline(
                &mut stream,
                &mut [0],
                Instant::now() + Duration::from_secs(10)
            )
            .unwrap(),
            0
        );
        writeln!(
            master,
            "wait \"$run_pid\"; printf 'child-status:%s\\n' \"$?\""
        )
        .unwrap();
        shell_output_until(
            &mut master,
            &mut shell,
            &format!("child-status:{}", 128 + signal),
        );
        success(pty_command_output(
            f.cli().arg("check"),
            &mut master,
            "source check during interactive job",
        ));
        master.write_all(b"exit 0\n").unwrap();
        assert!(
            wait_pty_child(
                &mut shell,
                &mut master,
                "interactive shell after exit command",
                true,
            )
            .success()
        );
    }
}
