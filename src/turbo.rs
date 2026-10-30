use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use sources::config::{ContentTask, Project};
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
    content: BTreeMap<String, ContentTask>,
    digests: BTreeMap<String, String>,
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
    crate::pnpm::clear_sources_env(&mut command);
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
    let mut resolver = Resolver::new(project)?;
    let mut env = BTreeMap::new();
    for source in &mut groups {
        env.extend(resolver.resolve(source)?);
    }
    refresh_and_verify(project, &groups, &env)?;
    parent.envs(env);
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
                content: ids_content(project, name, &ids),
                digests: BTreeMap::new(),
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

// Discover task edges independently, then refresh/hash/execute each selected
// group with its combined environment. Cycle identity remains checkout + task.
#[derive(Default)]
struct Resolver {
    visiting: Vec<(PathBuf, String)>,
    resolved: BTreeMap<(PathBuf, String), (SourceTasks, String)>,
    executed: BTreeSet<(PathBuf, String)>,
    caller: PathBuf,
}

impl Resolver {
    fn new(project: &Project) -> Result<Self> {
        Ok(Self {
            caller: std::fs::canonicalize(&project.root)?,
            ..Self::default()
        })
    }

    // Plain repositories cannot consume managed source env. Their validation
    // query is already the final hash snapshot, so keep it for later resolution.
    fn remember_plain(
        &mut self,
        source: &SourceTasks,
        hashes: &BTreeMap<String, String>,
    ) -> Result<()> {
        let checkout = std::fs::canonicalize(&source.checkout)?;
        for (id, arg) in source.ids.iter().zip(&source.args) {
            let mut selected = source.clone();
            selected.ids = vec![id.clone()];
            selected.args = vec![arg.clone()];
            self.resolved.insert(
                (checkout.clone(), id.clone()),
                (selected, hashes[id].clone()),
            );
        }
        Ok(())
    }

    fn resolve(&mut self, source: &mut SourceTasks) -> Result<BTreeMap<String, String>> {
        let checkout = std::fs::canonicalize(&source.checkout)?;
        ensure!(
            source.content.is_empty() || checkout != self.caller,
            "cannot prepare content task in ancestor {} holding the caller lock; stage it before invoking Sources",
            checkout.display()
        );
        let configured = source.checkout.join(sources::config::CONFIG).is_file();
        if !configured {
            let mut pending = source.clone();
            pending
                .ids
                .retain(|id| !self.resolved.contains_key(&(checkout.clone(), id.clone())));
            if !pending.ids.is_empty() {
                let (hashes, _) = query(&mut pending)?;
                self.remember_plain(&pending, &hashes)?;
            }
        }
        let child = if configured {
            let child = Project::load(source.checkout.clone())?;
            sources::config::real_directory_if_present(&child.internal())?;
            std::fs::create_dir_all(child.internal())?;
            child.validate()?;
            Some(child)
        } else {
            None
        };
        let candidates = child
            .as_ref()
            .map(configured_selectors)
            .transpose()?
            .unwrap_or_default();
        let mut ready = BTreeMap::new();
        let mut pending = false;
        // Discover dependencies task by task, keeping cycle identity at task level.
        // Do not mark every task in the group as visiting: an independent task in
        // this checkout may legitimately be needed through another repository.
        for id in &source.ids {
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
            if let Some((selected, _)) = self.resolved.get(&key) {
                ready.insert(id.clone(), selected.clone());
                continue;
            }
            pending = true;
            self.visiting.push(key);
            let mut selected = source.clone();
            selected.ids = vec![id.clone()];
            // Let native Turbo expand wildcards/exclusions for this exact task
            // and its prerequisites. Placeholders never reach execution.
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
                .iter()
                .filter(|(name, _)| consumed.contains(*name))
                .map(|(_, selector)| selector.clone())
                .collect::<Vec<_>>();
            for mut dependency in prepare(
                child.as_ref().expect("configured pending child"),
                &selectors,
            )? {
                selected.env.extend(self.resolve(&mut dependency)?);
            }
            self.visiting.pop();
            ready.insert(id.clone(), selected);
        }
        if pending {
            let child = child.as_ref().expect("configured pending child");
            let mut combined: BTreeMap<_, _> = ready
                .values()
                .flat_map(|selected| selected.env.clone())
                .collect();
            // Include cached tasks' dependencies too: an install must verify all
            // payloads consumed by the selected group, not only newly found edges.
            let selectors = candidates
                .iter()
                .filter(|(name, _)| combined.contains_key(*name))
                .map(|(_, selector)| selector.clone())
                .collect::<Vec<_>>();
            let mut dependencies = prepare(child, &selectors)?;
            for dependency in &mut dependencies {
                combined.extend(self.resolve(dependency)?);
            }
            if dependencies
                .iter()
                .any(|dependency| !dependency.content.is_empty())
            {
                ensure!(
                    checkout != self.caller,
                    "cannot prepare content in ancestor {} holding the caller lock; stage that task before invoking Sources",
                    checkout.display()
                );
                let _lock = crate::project_lock(&child.root, false)?;
                refresh_and_verify(child, &dependencies, &combined)?;
            }
            // One refresh with the complete group snapshot precedes all final
            // hashes. A shared lifecycle generator can now prepare every input.
            let mut snapshot = source.clone();
            snapshot.env = combined.clone();
            let (hashes, _) = query(&mut snapshot)?;
            for (id, arg) in snapshot.ids.iter().zip(&snapshot.args) {
                let key = (checkout.clone(), id.clone());
                if self.executed.contains(&key) {
                    ensure!(
                        self.resolved[&key].1 == hashes[id],
                        "prepared task {id} inputs changed during grouped refresh; stage nonoverlapping inputs before retrying"
                    );
                }
                let mut selected = ready.remove(id).expect("discovered selected task");
                selected.args = vec![arg.clone()];
                // Retain only this task's consumed edges in the memo. Returning
                // to this task must not invent dependencies on unrelated tasks.
                for (name, value) in &mut selected.env {
                    *value = combined[name].clone();
                }
                self.resolved.insert(key, (selected, hashes[id].clone()));
            }
        }
        source.env.clear();
        source.args.clear();
        for id in &source.ids {
            let (selected, _) = &self.resolved[&(checkout.clone(), id.clone())];
            source.env.extend(selected.env.clone());
            source.args.extend(selected.args.clone());
        }
        let mut execution = source.clone();
        execution.ids = source
            .ids
            .iter()
            .filter(|id| {
                source.content.contains_key(*id)
                    && !self.executed.contains(&(checkout.clone(), (*id).clone()))
            })
            .cloned()
            .collect();
        if !execution.ids.is_empty() {
            execution.args = execution
                .ids
                .iter()
                .flat_map(|id| {
                    self.resolved[&(checkout.clone(), id.clone())]
                        .0
                        .args
                        .clone()
                })
                .collect();
            execute_upstream(&execution)?;
            self.executed.extend(
                execution
                    .ids
                    .iter()
                    .map(|id| (checkout.clone(), id.clone())),
            );
        }
        let mut hashes = BTreeMap::new();
        for id in &source.ids {
            let (package, task) = id.split_once('#').expect("prepared identity");
            hashes.insert(
                sources::config::turbo_env_name(package, task)?,
                self.resolved[&(checkout.clone(), id.clone())].1.clone(),
            );
            if let Some(content) = source.content.get(id) {
                let digest = crate::artifacts::digest(&source.checkout, content)?;
                source.digests.insert(id.clone(), digest.clone());
                hashes.insert(sources::config::content_env_name(package, task)?, digest);
            }
        }
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
        .chain(
            project
                .config
                .sources
                .values()
                .flat_map(|source| &source.packages)
                .flat_map(|(package, entry)| entry.content.keys().map(move |task| (package, task)))
                .map(|(package, task)| {
                    Ok((
                        sources::config::content_env_name(package, task)?,
                        format!("{package}:{task}"),
                    ))
                }),
        )
        .collect()
}

pub(crate) fn task(project: &Project, selectors: &[String]) -> Result<i32> {
    let mut groups = prepare(project, selectors)?;
    // Validation cannot depend on content appearing in the caller's config:
    // any selected child can consume content declared only in its own project.
    let mut resolver = Resolver::new(project)?;
    for source in &mut groups {
        // Direct calls execute their own tasks once, after all validation.
        source.content.clear();
        let (hashes, _) = query(source)?;
        if !source.checkout.join(sources::config::CONFIG).is_file() {
            resolver.remember_plain(source, &hashes)?;
        }
    }
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

fn ids_content(project: &Project, owner: &str, ids: &[String]) -> BTreeMap<String, ContentTask> {
    ids.iter()
        .filter_map(|id| {
            let (package, task) = id.split_once('#').expect("prepared identity");
            project.config.sources[owner].packages[package]
                .content
                .get(task)
                .map(|content| (id.clone(), content.clone()))
        })
        .collect()
}

#[derive(Debug)]
pub(crate) struct PreparationFailure(pub i32);

impl std::fmt::Display for PreparationFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "upstream Turbo failed with exit status {}", self.0)
    }
}
impl std::error::Error for PreparationFailure {}

fn execute_upstream(source: &SourceTasks) -> Result<()> {
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
        return Err(PreparationFailure(crate::run::exit_code(status)).into());
    }
    Ok(())
}

fn refresh_and_verify(
    project: &Project,
    groups: &[SourceTasks],
    env: &BTreeMap<String, String>,
) -> Result<()> {
    let mut expected = Vec::new();
    for source in groups {
        for content in source.content.values() {
            for artifact in &content.artifacts {
                if let Some(installed) = &artifact.installed {
                    expected.push((
                        source.checkout.clone(),
                        artifact.path.clone(),
                        installed.clone(),
                        crate::artifacts::snapshot(&source.checkout, &artifact.path)?,
                    ));
                }
            }
        }
    }
    if !expected.is_empty() {
        crate::pnpm::refresh_frozen(project, env)?;
    }
    // Later upstream tasks or install lifecycles must not change an earlier
    // snapshot, including paths consumed directly through checkout links.
    for source in groups {
        for (id, content) in &source.content {
            ensure!(
                crate::artifacts::digest(&source.checkout, content)? == source.digests[id],
                "required artifacts for {id} changed during content preparation; stage deterministic, nonoverlapping outputs before retrying"
            );
        }
    }
    for (_, path, installed, before) in expected {
        ensure!(
            crate::artifacts::snapshot(&project.root, &installed)? == before,
            "installed consumed artifact {} differs from prepared {}; fix the file-package reference, package files/exports or explicit staging task before retrying",
            installed.display(),
            path.display()
        );
    }
    Ok(())
}
