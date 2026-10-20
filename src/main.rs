use anyhow::{Context, Result, ensure};
use clap::{Parser, Subcommand};
use sources::config::{self, CONFIG};
use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

mod commands;
mod run;
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
    /// Point workspace links at validated development checkouts
    Dev,
    /// Inspect actual workspace links, commits, and source changes locally
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Activate locked commits in managed checkouts
    Release {
        #[arg(long, conflicts_with = "offline")]
        clean: bool,
        #[arg(long)]
        offline: bool,
    },
    /// Capture development HEADs or update selected source pins
    Lock {
        pins: Vec<String>,
        #[arg(long)]
        check: bool,
    },
}

fn project_lock(root: &Path) -> Result<File> {
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
    lock.try_lock().map_err(|error| anyhow::anyhow!("project {} is locked or unavailable: {error}; retry after the other sources command finishes", root.display()))?;
    Ok(lock)
}

fn execute(cli: Cli) -> Result<i32> {
    let start = cli.directory.unwrap_or(std::env::current_dir()?);
    let root = match config::discover(&start)? {
        Some(root) => root,
        None if matches!(cli.command, Commands::Setup) => std::fs::canonicalize(start)?,
        None => anyhow::bail!("no {CONFIG} found; run sources setup in the project root"),
    };
    let _lock = project_lock(&root).context("cannot acquire project lock")?;
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
                Commands::Fetch { names } => commands::fetch(&project, &names)?,
                Commands::CheckRelease => commands::check_release(&project)?,
                Commands::Lock { pins, check } => commands::lock(&project, &pins, check)?,
                Commands::Dev => workspace::dev(&project)?,
                Commands::Status { json } => return workspace::status(&project, json),
                Commands::Release { clean, offline } => {
                    workspace::release(&project, clean, offline)?
                }
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
