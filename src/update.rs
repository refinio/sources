use anyhow::Result;

#[cfg(unix)]
pub fn execute() -> Result<()> {
    use anyhow::{Context, ensure};
    use std::process::{Command, Stdio};

    let executable = std::env::current_exe().context("cannot locate the running Sources binary")?;
    let directory = executable
        .parent()
        .context("Sources binary has no parent directory")?;
    let temp = tempfile::tempdir().context("cannot create update download directory")?;
    let installer = temp.path().join("sources-installer.sh");
    let url = std::env::var_os("SOURCES_UPDATE_URL").unwrap_or_else(|| {
        "https://github.com/refinio/sources/releases/latest/download/sources-installer.sh".into()
    });

    println!("Updating Sources from the latest stable GitHub Release...");
    let status = Command::new("curl")
        .args([
            "--proto",
            "=https,http",
            "--proto-redir",
            "=https,http",
            "--tlsv1.2",
            "--fail",
            "--location",
            "--silent",
            "--show-error",
            "--output",
        ])
        .arg(&installer)
        .arg("--")
        .arg(url)
        .status()
        .context("cannot download the update installer; install curl and retry sources update")?;
    ensure!(
        status.success(),
        "update installer download failed ({status}); retry sources update"
    );

    let output = Command::new("sh")
        .arg(&installer)
        .env_remove("SOURCES_INSTALL_DIR")
        .env_remove("CARGO_DIST_FORCE_INSTALL_DIR")
        .env("CARGO_HOME", temp.path().join("cargo-home"))
        .env("SOURCES_UNMANAGED_INSTALL", directory)
        .env("SOURCES_NO_MODIFY_PATH", "1")
        .env("SOURCES_PRINT_QUIET", "0")
        .stdin(Stdio::null())
        .output()
        .context("cannot start the update installer; sources update requires sh")?;
    ensure!(
        output.status.success(),
        "Sources update failed ({}) in {}:\n{}{}\nfix the installer error and retry sources update",
        output.status,
        directory.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    println!("Sources updated. To update installed agent skills, run: npx skills update");
    Ok(())
}

#[cfg(not(unix))]
pub fn execute() -> Result<()> {
    anyhow::bail!(
        "self-update requires Linux or macOS; close Sources and reinstall from PowerShell: powershell -NoProfile -ExecutionPolicy Bypass -Command \"irm https://github.com/refinio/sources/releases/latest/download/sources-installer.ps1 | iex\""
    )
}
