use anyhow::{Context, Result, ensure};
use sources::acquisition::activation_issues;
use sources::config::Project;
use sources::git::Git;
use std::ffi::OsString;
#[cfg(unix)]
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::Command;

pub fn execute(project: &Project, name: &str, args: &[OsString]) -> Result<i32> {
    let checkout = active_checkout(project, name)?;
    let git = Git::at(&checkout);
    warn_changes(&git, name)?;
    let mut command = Command::new(&args[0]);
    command.args(&args[1..]).current_dir(&checkout);
    let status = wait_command(&mut command, name)?;
    if let Err(error) = warn_changes(&git, name) {
        eprintln!(
            "warning: {name}: cannot inspect source after command: {}",
            sources::git::redact(&format!("{error:#}"))
        );
    }
    Ok(exit_code(status))
}

pub(crate) fn exit_code(status: std::process::ExitStatus) -> i32 {
    #[cfg(unix)]
    let code = status
        .code()
        .unwrap_or_else(|| 128 + status.signal().unwrap_or(0));
    #[cfg(not(unix))]
    let code = status.code().unwrap_or(1);
    code
}

/// Resolve an active workspace checkout without selecting, fetching, or installing.
pub(crate) fn active_checkout(project: &Project, name: &str) -> Result<std::path::PathBuf> {
    let source = project
        .config
        .sources
        .get(name)
        .with_context(|| format!("unknown source {name}"))?;
    if source.has_packages() {
        ensure!(
            crate::pnpm::source_mode(project, name)? == "file",
            "{name}: published package selection has no active source checkout; run sources dev or sources release --git before executing checkout tasks"
        );
    }
    let link = project.links().join(name);
    let metadata = std::fs::symlink_metadata(&link).with_context(|| {
        format!(
            "missing workspace link for {name} at {}; run sources dev or sources release",
            link.display()
        )
    })?;
    ensure!(
        metadata.file_type().is_symlink(),
        "workspace path for {name} must be a link: {}; remove the collision and run sources dev or sources release",
        link.display()
    );
    let target = link.canonicalize().with_context(|| {
        format!(
            "broken workspace link for {name} at {}; run sources dev or sources release",
            link.display()
        )
    })?;
    let managed = project.managed(name);
    let is_managed = managed.canonicalize().is_ok_and(|path| path == target);
    if is_managed {
        sources::config::real_directory_if_present(&project.internal().join("checkouts"))?;
    }
    let is_dev = project
        .dev(name)
        .ok()
        .and_then(|path| path.canonicalize().ok())
        .is_some_and(|path| path == target);
    ensure!(
        is_managed || is_dev,
        "unexpected workspace link target for {name} at {}; run sources dev or sources release",
        link.display()
    );
    let checkout = if is_managed { managed } else { target };
    let git = Git::at(&checkout);
    git.validate(&source.url, is_managed)
        .with_context(|| format!("{name}: invalid active checkout"))?;
    if is_managed {
        ensure!(
            git.head()?.is_some(),
            "{name}: managed checkout is not activated; run sources release"
        );
        let issues = activation_issues(&checkout).with_context(|| {
            format!("{name}: cannot inspect active managed submodules; run sources release")
        })?;
        ensure!(
            issues.is_empty(),
            "{name}: incomplete managed submodule activation: {}",
            issues.join("; ")
        );
    }
    Ok(checkout)
}

/// Wait using the same terminal/job supervision policy for every locked command.
pub(crate) fn wait_command(command: &mut Command, name: &str) -> Result<std::process::ExitStatus> {
    #[cfg(unix)]
    let status = if std::fs::File::open("/dev/tty").is_ok() {
        // Keep the shell's group even for background jobs and redirected streams.
        let mut signals = signal_hook::iterator::Signals::new([libc::SIGINT, libc::SIGTERM])?;
        let child = command.spawn().with_context(|| {
            format!(
                "cannot start command {:?} for {name}",
                command.get_program()
            )
        })?;
        loop {
            for signal in signals.pending() {
                // SIGINT reaches the child through the shell's job group already.
                if signal == libc::SIGTERM {
                    forward_signal(child.id() as i32, signal, name);
                }
            }
            let mut raw = 0;
            let result = unsafe {
                libc::waitpid(child.id() as i32, &mut raw, libc::WNOHANG | libc::WUNTRACED)
            };
            if result == -1 {
                let error = std::io::Error::last_os_error();
                if error.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(error).context("cannot wait for interactive child command");
            }
            if result > 0 {
                if libc::WIFSTOPPED(raw) {
                    // Mirror only this child, leaving the shell's other job members alone.
                    unsafe { libc::kill(libc::getpid(), libc::SIGSTOP) };
                    forward_signal(child.id() as i32, libc::SIGCONT, name);
                } else {
                    break std::process::ExitStatus::from_raw(raw);
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    } else {
        wait_group(command, name)?
    };
    #[cfg(not(unix))]
    let status = command
        .spawn()
        .with_context(|| {
            format!(
                "cannot start command {:?} for {name}",
                command.get_program()
            )
        })?
        .wait()?;
    Ok(status)
}

#[cfg(unix)]
fn forward_signal(target: i32, signal: i32, name: &str) {
    if unsafe { libc::kill(target, signal) } == -1 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::ESRCH) {
            eprintln!("warning: {name}: cannot forward signal {signal}: {error}");
        }
    }
}

#[cfg(unix)]
fn wait_group(command: &mut Command, name: &str) -> Result<std::process::ExitStatus> {
    command.process_group(0);
    let mut signals = signal_hook::iterator::Signals::new([libc::SIGINT, libc::SIGTERM])?;
    #[cfg(target_os = "linux")]
    ensure!(
        unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0) } == 0,
        "cannot supervise orphaned child processes: {}",
        std::io::Error::last_os_error()
    );
    let child = command.spawn().with_context(|| {
        format!(
            "cannot start command {:?} for {name}",
            command.get_program()
        )
    })?;
    let group = -(child.id() as i32);
    let mut child_status = None;
    loop {
        for signal in signals.pending() {
            forward_signal(group, signal, name);
        }
        let mut raw = 0;
        let result = unsafe { libc::waitpid(group, &mut raw, libc::WNOHANG | libc::WUNTRACED) };
        if result == -1 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            if error.raw_os_error() != Some(libc::ECHILD) || child_status.is_none() {
                return Err(error).context("cannot wait for child command");
            }
        }
        if result > 0 {
            if libc::WIFSTOPPED(raw) {
                unsafe {
                    libc::kill(libc::getpid(), libc::SIGSTOP);
                    libc::kill(group, libc::SIGCONT);
                }
            } else if result == child.id() as i32 {
                child_status = Some(std::process::ExitStatus::from_raw(raw));
            }
        }
        if let Some(status) = child_status
            && unsafe { libc::kill(group, 0) } == -1
            && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
        {
            return Ok(status);
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

pub(crate) fn warn_changes(git: &Git, name: &str) -> Result<()> {
    for change in git.changes()? {
        eprintln!("warning: {name}: {} {}", change.status, change.path);
    }
    Ok(())
}
