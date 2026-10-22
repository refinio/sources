use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use sources::config::Project;
use sources::git::Git;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Clone)]
struct SourceTasks {
    name: String,
    checkout: PathBuf,
    ids: Vec<String>,
    args: Vec<String>,
    env: BTreeMap<String, String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Metadata {
    version: String,
    monorepo: bool,
    tasks: Vec<Task>,
    #[serde(default)]
    global_cache_inputs: GlobalInputs,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Task {
    task_id: String,
    hash: String,
    command: String,
    #[serde(default)]
    environment_variables: Environment,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GlobalInputs {
    #[serde(default)]
    environment_variables: Environment,
}

#[derive(Default, Deserialize)]
struct Environment {
    #[serde(default)]
    configured: Vec<String>,
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
    // Only snapshots computed for this checkout may enter its Turbo environment.
    // Ancestor hashes can describe another graph or an obsolete configuration.
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
    let mut resolver = Resolver::default();
    for source in &mut groups {
        for (id, hash) in resolver.resolve(source)? {
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
                env: BTreeMap::new(),
            })
        })
        .collect()
}

fn query_metadata(source: &SourceTasks, args: &[String]) -> Result<Metadata> {
    let output = tempfile::tempfile()?;
    let mut command = command(&source.checkout)?;
    command.args(["run"]).args(args).arg("--dry=json");
    command.envs(&source.env);
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

fn query(source: &mut SourceTasks) -> Result<(BTreeMap<String, String>, Metadata)> {
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
    Ok((hashes, metadata))
}

// Resolve one selected task at a time. Repository-level recursion would reject
// valid graphs that return to a different, independent task in the same checkout.
#[derive(Default)]
struct Resolver {
    visiting: Vec<(PathBuf, String)>,
    resolved: BTreeMap<(PathBuf, String), (SourceTasks, String)>,
}

impl Resolver {
    fn resolve(&mut self, source: &mut SourceTasks) -> Result<BTreeMap<String, String>> {
        let checkout = std::fs::canonicalize(&source.checkout)?;
        // Plain Turbo children cannot introduce source hash edges. Query their
        // uncached tasks together, then reuse them across all consuming graphs.
        if !source.checkout.join(sources::config::CONFIG).is_file() {
            let mut pending = source.clone();
            pending
                .ids
                .retain(|id| !self.resolved.contains_key(&(checkout.clone(), id.clone())));
            if !pending.ids.is_empty() {
                let (hashes, _) = query(&mut pending)?;
                for (id, arg) in pending.ids.iter().zip(&pending.args) {
                    let mut selected = pending.clone();
                    selected.ids = vec![id.clone()];
                    selected.args = vec![arg.clone()];
                    self.resolved.insert(
                        (checkout.clone(), id.clone()),
                        (selected, hashes[id].clone()),
                    );
                }
            }
        }
        let mut hashes = BTreeMap::new();
        let mut args = Vec::new();
        for id in source.ids.clone() {
            let key = (checkout.clone(), id.clone());
            if let Some(start) = self.visiting.iter().position(|entry| entry == &key) {
                let path = self.visiting[start..]
                    .iter()
                    .chain(std::iter::once(&key))
                    .map(|(root, id)| format!("{}:{id}", root.display()))
                    .collect::<Vec<_>>()
                    .join(" -> ");
                anyhow::bail!(
                    "Turbo source task hash cycle: {path}; remove a cyclic SOURCES_* task env dependency in turbo.json"
                );
            }
            if !self.resolved.contains_key(&key) {
                self.visiting.push(key.clone());
                let mut selected = source.clone();
                selected.ids = vec![id.clone()];
                selected.env.clear();
                let child = Project::load(selected.checkout.clone())?;
                sources::config::real_directory_if_present(&child.internal())?;
                std::fs::create_dir_all(child.internal())?;
                child.validate()?;
                // Supply candidate names only to the discovery query. Turbo expands
                // env wildcards/exclusions for its selected and transitive tasks.
                // These placeholder values never reach execution or exported hashes.
                let candidates = configured_selectors(&child)?;
                selected.env = candidates
                    .keys()
                    .map(|name| (name.clone(), String::new()))
                    .collect();
                let (_, metadata) = query(&mut selected)?;
                let consumed: BTreeSet<_> = metadata
                    .global_cache_inputs
                    .environment_variables
                    .configured
                    .iter()
                    .chain(
                        metadata
                            .tasks
                            .iter()
                            .flat_map(|task| &task.environment_variables.configured),
                    )
                    .filter_map(|value| value.split_once('=').map(|(name, _)| name.to_owned()))
                    .collect();
                selected.env.clear();
                let selectors = candidates
                    .into_iter()
                    .filter(|(name, _)| consumed.contains(name))
                    .map(|(_, selector)| selector)
                    .collect::<Vec<_>>();
                for mut dependency in prepare(&child, &selectors)? {
                    for (id, hash) in self.resolve(&mut dependency)? {
                        let (package, task) = id.split_once('#').expect("prepared identity");
                        selected
                            .env
                            .insert(sources::config::turbo_env_name(package, task)?, hash);
                    }
                }
                // Use exactly this snapshot in both the real query and execution.
                let mut selected_hashes = query(&mut selected)?.0;
                let hash = selected_hashes
                    .remove(&id)
                    .expect("validated selected hash");
                self.resolved.insert(key.clone(), (selected, hash));
                self.visiting.pop();
            }
            let (selected, hash) = &self.resolved[&key];
            source.env.extend(selected.env.clone());
            args.extend(selected.args.clone());
            hashes.insert(id, hash.clone());
        }
        source.args = args;
        Ok(hashes)
    }
}

fn configured_selectors(project: &Project) -> Result<BTreeMap<String, String>> {
    project
        .config
        .sources
        .values()
        .flat_map(|source| &source.packages)
        .flat_map(|(package, entry)| entry.tasks.iter().map(move |task| (package, task)))
        .map(|(package, task)| {
            Ok((
                sources::config::turbo_env_name(package, task)?,
                format!("{package}:{task}"),
            ))
        })
        .collect()
}

pub(crate) fn task(project: &Project, selectors: &[String]) -> Result<i32> {
    let mut groups = prepare(project, selectors)?;
    // Validate every source's selected tasks before any task is allowed to run.
    let mut resolver = Resolver::default();
    for source in &mut groups {
        resolver.resolve(source)?;
    }
    for source in &groups {
        let mut command = command(&source.checkout)?;
        command.arg("run").args(&source.args).envs(&source.env);
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
