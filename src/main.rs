use anyhow::{Context, Result, ensure};
use clap::{Parser, Subcommand};
use sources::config::{self, CONFIG};
use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

mod commands;
mod pnpm;
mod run;
mod turbo;
mod update;
mod workspace;

#[derive(Parser)]
#[command(
    version,
    about = "Select development checkouts and pinned release sources"
)]
struct Cli {
    /// Start project discovery in this directory
    #[arg(short = 'C', global = true)]
    directory: Option<PathBuf>,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Update this CLI to the latest stable GitHub Release
    Update,
    /// Run parent Turbo with hashes from every configured child package/task
    #[command(disable_help_flag = true)]
    Turbo {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        arguments: Vec<OsString>,
    },
    /// Run allowed package:task entry points through the selected child Turbo repositories
    Task {
        #[arg(required = true, num_args = 1..)]
        selectors: Vec<String>,
    },
    /// Create configuration and ignore entries without downloading sources
    Setup,
    /// Run a command in a source's active workspace checkout
    Run {
        name: String,
        #[arg(last = true, required = true, num_args = 1..)]
        command: Vec<OsString>,
    },
    /// Validate development checkouts without downloading or changing sources
    Check,
    /// Cache required revisions without activating them
    Fetch { names: Vec<String> },
    /// Check and cache all locked release inputs without activation
    CheckRelease,
    /// Select development checkouts and install configured pnpm file dependencies
    Dev,
    /// Inspect actual workspace links, commits, and source changes locally
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Activate pinned Git checkouts or published registry packages
    Release {
        #[arg(long, conflicts_with = "offline")]
        clean: bool,
        /// Select pinned Git checkouts (the default)
        #[arg(long, conflicts_with = "registry")]
        git: bool,
        /// Select pinned registry packages, retaining Git selection for other sources
        #[arg(long)]
        registry: bool,
        #[arg(long)]
        offline: bool,
    },
    /// Capture development HEADs or update selected source pins
    Lock {
        #[arg(required_if_eq("registry", "true"))]
        pins: Vec<String>,
        /// Capture or update Git commit pins (the default)
        #[arg(long, conflicts_with = "registry")]
        git: bool,
        /// Pin exact package versions using <package>:<version>
        #[arg(long)]
        registry: bool,
        #[arg(long)]
        check: bool,
    },
}

fn project_lock(root: &Path, wait: bool) -> Result<File> {
    let internal = root.join(".sources");
    config::real_directory_if_present(&internal)?;
    std::fs::create_dir_all(&internal)?;
    let path = internal.join("project.lock");
    if let Ok(metadata) = std::fs::symlink_metadata(&path) {
        ensure!(
            metadata.is_file() && !metadata.file_type().is_symlink(),
            "{} must be an ordinary lock file",
            path.display()
        );
    }
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)?;
    if wait {
        match lock.try_lock() {
            Ok(()) => {}
            Err(std::fs::TryLockError::WouldBlock) => {
                eprintln!("waiting for project lock at {}", root.display());
                lock.lock().with_context(|| {
                    format!("cannot wait for project lock at {}", root.display())
                })?;
            }
            Err(error) => return Err(error.into()),
        }
    } else {
        lock.try_lock().map_err(|error| anyhow::anyhow!("project {} is locked or unavailable: {error}; retry after the other sources command finishes", root.display()))?;
    }
    Ok(lock)
}

fn execute(cli: Cli) -> Result<i32> {
    if matches!(cli.command, Commands::Update) {
        update::execute()?;
        return Ok(0);
    }
    let start = cli.directory.unwrap_or(std::env::current_dir()?);
    let root = match config::discover(&start)? {
        Some(root) => root,
        None if matches!(cli.command, Commands::Setup) => std::fs::canonicalize(start)?,
        None => anyhow::bail!("no {CONFIG} found; run sources setup in the project root"),
    };
    let lock = project_lock(&root, matches!(cli.command, Commands::Task { .. }))
        .context("cannot acquire project lock")?;
    match cli.command {
        Commands::Setup => config::setup(root)?,
        Commands::Check => {
            let project = config::Project::load(root)?;
            project.validate()?;
            let mut errors = Vec::new();
            for (name, source) in &project.config.sources {
                let result = (|| -> Result<()> {
                    let repo = sources::git::Git::at(&project.dev(name)?);
                    repo.validate(&source.url, false)?;
                    for change in repo.changes()? {
                        eprintln!("warning: {name}: {} {}", change.status, change.path);
                    }
                    println!("{name}: development checkout valid");
                    Ok(())
                })();
                if let Err(error) = result {
                    errors.push(format!("{name}: {error:#}"));
                }
            }
            ensure!(errors.is_empty(), "{}", errors.join("\n"));
        }
        command => {
            let project = config::Project::load(root)?;
            project.validate()?;
            match command {
                Commands::Run { name, command } => return run::execute(&project, &name, &command),
                Commands::Task { selectors } => return turbo::task(&project, &selectors),
                Commands::Turbo { arguments } => {
                    let mut command = turbo::prepare_parent(&project, &arguments)?;
                    // Parent Turbo may invoke concurrent `sources task` prerequisites.
                    // Only the metadata snapshot is protected by this outer lock.
                    drop(lock);
                    return run::wait_command(&mut command, "parent Turbo").map(run::exit_code);
                }
                Commands::Fetch { names } => commands::fetch(&project, &names)?,
                Commands::CheckRelease => commands::check_release(&project)?,
                Commands::Lock {
                    pins,
                    check,
                    registry,
                    ..
                } => {
                    if registry {
                        commands::lock_registry(&project, &pins, check)?;
                    } else {
                        commands::lock(&project, &pins, check)?;
                    }
                }
                Commands::Dev => workspace::dev(&project)?,
                Commands::Status { json } => return workspace::status(&project, json),
                Commands::Release {
                    clean,
                    offline,
                    registry,
                    ..
                } => workspace::release(&project, clean, offline, registry)?,
                _ => unreachable!(),
            }
        }
    }
    Ok(0)
}

fn main() {
    let code = match execute(Cli::parse()) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("error: {}", sources::git::redact(&format!("{error:#}")));
            1
        }
    };
    std::process::exit(code);
}
