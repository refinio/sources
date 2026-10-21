use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use sources::config::Project;
use sources::git::Git;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

struct SourceTasks {
    name: String,
    checkout: PathBuf,
    ids: Vec<String>,
    args: Vec<String>,
}

#[derive(Deserialize)]
struct Metadata {
    version: String,
    monorepo: bool,
    tasks: Vec<Task>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Task {
    task_id: String,
    hash: String,
    command: String,
}

fn command(root: &Path) -> Result<Command> {
    let bin = root.join("node_modules/.bin");
    let names: &[&str] = if cfg!(windows) {
        &["turbo.exe", "turbo.cmd"]
    } else {
        &["turbo"]
    };
    ensure!(
        names.iter().any(|name| bin.join(name).is_file()),
        "missing locally installed Turbo at {}; bootstrap this repository's pnpm dependencies before retrying",
        root.display()
    );
    let mut command = crate::pnpm::pnpm_command(root);
    command.args(["exec", "turbo"]);
    // Snapshot queries and nested task execution must hash the same environment.
    // Never feed the parent's bridge hashes back into a child's task hashes.
    for (key, _) in std::env::vars_os() {
        let prefix = key.as_encoded_bytes().get(..b"SOURCES_".len());
        let managed = prefix.is_some_and(|prefix| {
            if cfg!(windows) {
                prefix.eq_ignore_ascii_case(b"SOURCES_")
            } else {
                prefix == b"SOURCES_"
            }
        });
        if managed {
            command.env_remove(key);
        }
    }
    Ok(command)
}

pub(crate) fn prepare_parent(project: &Project, arguments: &[OsString]) -> Result<Command> {
    let mut parent = command(&project.root)?;
    let selectors = project
        .config
        .sources
        .values()
        .flat_map(|source| &source.packages)
        .flat_map(|(package, entry)| {
            entry
                .tasks
                .iter()
                .map(move |task| format!("{package}:{task}"))
        })
        .collect::<Vec<_>>();
    let mut groups = prepare(project, &selectors)?;
    for source in &mut groups {
        for (id, hash) in query(source)? {
            let (package, task) = id.split_once('#').expect("prepared package/task identity");
            parent.env(sources::config::turbo_env_name(package, task)?, hash);
        }
    }
    parent.args(arguments);
    Ok(parent)
}

fn prepare(project: &Project, selectors: &[String]) -> Result<Vec<SourceTasks>> {
    let mut groups: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    let mut seen = BTreeSet::new();
    for selector in selectors {
        let (package, task) = selector
            .split_once(':')
            .with_context(|| format!("expected package:task selector, got {selector:?}"))?;
        let (owner, entry) = project
            .config
            .sources
            .iter()
            .find_map(|(name, source)| source.packages.get(package).map(|entry| (name, entry)))
            .with_context(|| {
                format!("unconfigured Turbo package {package:?}; edit .sources.conf packages")
            })?;
        ensure!(
            entry.tasks.iter().any(|allowed| allowed == task),
            "unconfigured Turbo task {selector:?}; edit its .sources.conf tasks allowlist"
        );
        if seen.insert(selector) {
            groups
                .entry(owner)
                .or_default()
                .push(format!("{package}#{task}"));
        }
    }
    groups
        .into_iter()
        .map(|(name, ids)| {
            let checkout = crate::run::active_checkout(project, name)?;
            crate::run::warn_changes(&Git::at(&checkout), name)?;
            command(&checkout)?;
            Ok(SourceTasks {
                name: name.to_owned(),
                checkout,
                args: ids.clone(),
                ids,
            })
        })
        .collect()
}

fn query_metadata(source: &SourceTasks, args: &[String]) -> Result<Metadata> {
    let output = tempfile::tempfile()?;
    let mut command = command(&source.checkout)?;
    command.args(["run"]).args(args).arg("--dry=json");
    command.stdout(Stdio::from(output.try_clone()?));
    let status = crate::run::wait_command(&mut command, &source.name)
        .context("cannot query child Turbo; install pnpm and bootstrap the child repository")?;
    ensure!(
        status.success(),
        "{}: Turbo metadata query failed ({status}); fix the child Turbo configuration and retry",
        source.name
    );
    use std::io::{Read, Seek};
    let mut output = output;
    output.rewind()?;
    let mut bytes = Vec::new();
    output.read_to_end(&mut bytes)?;
    let metadata: Metadata = serde_json::from_slice(&bytes).with_context(|| {
        format!("{}: invalid or unsupported Turbo dry-run JSON; expected schema version 1 with taskId, hash and command fields", source.name)
    })?;
    ensure!(
        metadata.version == "1",
        "{}: unsupported Turbo metadata version {:?}; expected 1",
        source.name,
        metadata.version
    );
    Ok(metadata)
}

fn query(source: &mut SourceTasks) -> Result<BTreeMap<String, String>> {
    let manifest: serde_json::Value =
        sources::config::read_json(&source.checkout.join("package.json"))
            .with_context(|| format!("{}: cannot read the child package manifest", source.name))?;
    let root_name = manifest.get("name").and_then(serde_json::Value::as_str);
    let mut args = source
        .ids
        .iter()
        .map(|id| {
            let (package, task) = id.split_once('#').expect("prepared package/task identity");
            if Some(package) == root_name {
                format!("//#{task}")
            } else {
                id.clone()
            }
        })
        .collect::<Vec<_>>();
    let mut metadata = query_metadata(source, &args)?;
    if !metadata.monorepo {
        args = source.ids.iter().map(|id| {
            let (package, task) = id.split_once('#').expect("prepared package/task identity");
            ensure!(Some(package) == root_name,
                "{}: single-package Turbo checkout does not contain configured package {package:?}; match package.json name", source.name);
            ensure!(!task.starts_with('-'),
                "{}: single-package Turbo cannot select option-prefixed task {task:?}; rename the child task", source.name);
            Ok(task.to_owned())
        }).collect::<Result<Vec<_>>>()?;
        metadata = query_metadata(source, &args)?;
    }
    let mut hashes = BTreeMap::new();
    for (id, selected_id) in source.ids.iter().zip(&args) {
        let matches: Vec<_> = metadata
            .tasks
            .iter()
            .filter(|task| &task.task_id == selected_id)
            .collect();
        ensure!(
            matches.len() == 1,
            "{}: Turbo metadata must contain exactly one selected task {id:?}; verify the package and task exist",
            source.name
        );
        let task = matches[0];
        ensure!(
            !task.command.trim().is_empty() && task.command != "<NONEXISTENT>",
            "{}: selected Turbo task {id:?} has no executable package script; configure the child task before retrying",
            source.name
        );
        ensure!(
            !task.hash.is_empty(),
            "{}: selected Turbo task {id:?} has an empty hash",
            source.name
        );
        hashes.insert(id.clone(), task.hash.clone());
    }
    source.args = args;
    Ok(hashes)
}

pub(crate) fn task(project: &Project, selectors: &[String]) -> Result<i32> {
    let mut groups = prepare(project, selectors)?;
    // Validate every source's selected tasks before any task is allowed to run.
    for source in &mut groups {
        query(source)?;
    }
    for source in &groups {
        let mut command = command(&source.checkout)?;
        command.arg("run").args(&source.args);
        let status = crate::run::wait_command(&mut command, &source.name)?;
        if let Err(error) = crate::run::warn_changes(&Git::at(&source.checkout), &source.name) {
            eprintln!(
                "warning: {}: cannot inspect source after command: {error:#}",
                source.name
            );
        }
        if !status.success() {
            return Ok(crate::run::exit_code(status));
        }
    }
    Ok(0)
}
