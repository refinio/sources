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

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Task {
    task_id: String,
    hash: String,
    command: String,
    #[serde(default)]
    dependencies: Vec<String>,
    #[serde(default)]
    with: Vec<String>,
    #[serde(default)]
    environment_variables: Environment,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GlobalInputs {
    #[serde(default)]
    environment_variables: Environment,
}

#[derive(Clone, Default, Deserialize)]
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
    parent.args(arguments);
    let mut options = arguments.iter().take_while(|arg| *arg != "--");
    if options
        .clone()
        .any(|arg| matches!(arg.to_str(), Some("--help" | "-h" | "--version" | "-V")))
    {
        return Ok(parent);
    }
    let mut discovery = Vec::new();
    let mut task_arguments = false;
    for argument in arguments {
        if !task_arguments && argument == "--" {
            discovery.push(OsString::from("--dry=json"));
            task_arguments = true;
        }
        if !task_arguments
            && argument.to_str().is_some_and(|arg| {
                arg == "--dry"
                    || arg == "--dry-run"
                    || arg.starts_with("--dry=")
                    || arg.starts_with("--dry-run=")
            })
        {
            continue;
        }
        discovery.push(argument.clone());
    }
    if !task_arguments {
        discovery.push(OsString::from("--dry=json"));
    }
    let mut graph =
        Graph::new(options.any(|arg| arg.to_str().is_some_and(|arg| arg.starts_with("--dry"))));
    let root = std::fs::canonicalize(&project.root)?;
    graph.load_owner(&root)?;
    let inputs = {
        let _lock = crate::project_lock(&root, true)?;
        let candidates = configured_selectors(project)?;
        let env = candidates
            .keys()
            .map(|name| (name.clone(), String::new()))
            .collect();
        let metadata = metadata_command(&root, &discovery, &env, "parent Turbo")?;
        let names = metadata_names(&metadata);
        // Parent selection options can prune native prerequisites. Foreign return
        // queries use their owner's normal exact-task closure, a different batch.
        inputs_for(project, &names)?
    };
    graph.discover(
        inputs
            .values()
            .map(|input| input.producer.clone())
            .collect(),
    )?;
    graph.execute.extend(
        inputs
            .values()
            .filter(|input| input.content.is_some())
            .map(|input| input.producer.clone()),
    );
    graph.settle()?;
    let (env, groups) = graph.environment(&inputs)?;
    {
        let _lock = crate::project_lock(&root, true)?;
        refresh_and_verify(project, &groups, &env, graph.stderr)?;
    }
    parent.envs(env);
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

fn metadata_command(
    root: &Path,
    args: &[OsString],
    env: &BTreeMap<String, String>,
    name: &str,
) -> Result<Metadata> {
    let output = tempfile::tempfile()?;
    let mut command = command(root)?;
    command
        .args(args)
        .envs(env)
        .stdout(Stdio::from(output.try_clone()?));
    let status = crate::run::wait_command(&mut command, name)
        .context("cannot query Turbo; install pnpm and bootstrap the owning repository")?;
    ensure!(
        status.success(),
        "{name}: Turbo metadata query failed ({status}); fix the Turbo configuration and retry"
    );
    use std::io::{Read, Seek};
    let mut output = output;
    output.rewind()?;
    let mut bytes = Vec::new();
    output.read_to_end(&mut bytes)?;
    let metadata: Metadata = serde_json::from_slice(&bytes).with_context(|| {
        format!("{name}: invalid or unsupported Turbo dry-run JSON; expected schema version 1 with taskId, hash and command fields")
    })?;
    ensure!(
        metadata.version == "1",
        "{name}: unsupported Turbo metadata version {:?}; expected 1",
        metadata.version
    );
    Ok(metadata)
}

fn query_metadata(source: &SourceTasks, args: &[String]) -> Result<Metadata> {
    let args = std::iter::once(OsString::from("run"))
        .chain(args.iter().map(OsString::from))
        .chain(std::iter::once(OsString::from("--dry=json")))
        .collect::<Vec<_>>();
    metadata_command(&source.checkout, &args, &source.env, &source.name)
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

// Discovery is a graph snapshot, not execution. Native queries are batched by
// canonical owner/frontier; only the selected closure becomes graph entries.
type Key = (PathBuf, String);
type Inputs = BTreeMap<String, Input>;

#[derive(Clone)]
struct Input {
    producer: Key,
    content: Option<ContentTask>,
}

struct Node {
    inputs: Inputs,
    hash: Option<String>,
}

struct Owner {
    project: Option<Project>,
    root_name: String,
    tasks: BTreeMap<String, Task>,
}

impl Owner {
    fn ingest(&mut self, metadata: Metadata) {
        let normalize = |id: &str| {
            if !metadata.monorepo {
                format!("{}#{id}", self.root_name)
            } else if let Some(task) = id.strip_prefix("//#") {
                format!("{}#{task}", self.root_name)
            } else {
                id.to_owned()
            }
        };
        for mut task in metadata.tasks {
            task.task_id = normalize(&task.task_id);
            task.dependencies = task.dependencies.iter().map(|id| normalize(id)).collect();
            task.with = task.with.iter().map(|id| normalize(id)).collect();
            task.environment_variables.configured.extend(
                metadata
                    .global_cache_inputs
                    .environment_variables
                    .configured
                    .clone(),
            );
            self.tasks.insert(task.task_id.clone(), task);
        }
    }

    fn names(&self, id: &str) -> BTreeSet<String> {
        let mut pending = vec![id.to_owned()];
        let mut seen = BTreeSet::new();
        let mut names = BTreeSet::new();
        while let Some(id) = pending.pop() {
            if !seen.insert(id.clone()) {
                continue;
            }
            // Native selection options such as --only can omit prerequisites.
            if let Some(task) = self.tasks.get(&id) {
                names.extend(environment_names(&task.environment_variables));
                pending.extend(task.dependencies.clone());
                pending.extend(task.with.clone());
            }
        }
        names
    }
}

fn environment_names(env: &Environment) -> impl Iterator<Item = String> + '_ {
    env.configured
        .iter()
        .filter_map(|value| value.split_once('=').map(|(name, _)| name.to_owned()))
}

fn metadata_names(metadata: &Metadata) -> BTreeSet<String> {
    if metadata.tasks.is_empty() {
        return BTreeSet::new();
    }
    environment_names(&metadata.global_cache_inputs.environment_variables)
        .chain(
            metadata
                .tasks
                .iter()
                .flat_map(|task| environment_names(&task.environment_variables)),
        )
        .collect()
}

fn inputs_for(project: &Project, names: &BTreeSet<String>) -> Result<Inputs> {
    let candidates = configured_selectors(project)?;
    let selectors = names
        .iter()
        .filter_map(|name| candidates.get(name).cloned())
        .collect::<Vec<_>>();
    let groups = prepare(project, &selectors)?;
    let mut selected = BTreeMap::new();
    for source in groups {
        let root = std::fs::canonicalize(&source.checkout)?;
        for id in &source.ids {
            let (package, task) = id.split_once('#').expect("prepared identity");
            selected.insert(
                format!("{package}:{task}"),
                (root.clone(), id.clone(), source.content.get(id).cloned()),
            );
        }
    }
    names
        .iter()
        .filter_map(|name| candidates.get(name).map(|selector| (name, selector)))
        .map(|(name, selector)| {
            let (root, id, content) = &selected[selector];
            let (package, task) = id.split_once('#').expect("prepared identity");
            Ok((
                name.clone(),
                Input {
                    producer: (root.clone(), id.clone()),
                    content: if name == &sources::config::content_env_name(package, task)? {
                        content.clone()
                    } else {
                        None
                    },
                },
            ))
        })
        .collect()
}

fn selection(root: &Path, ids: Vec<String>, env: BTreeMap<String, String>) -> SourceTasks {
    SourceTasks {
        name: root.display().to_string(),
        checkout: root.to_owned(),
        args: ids.clone(),
        ids,
        env,
        content: BTreeMap::new(),
        digests: BTreeMap::new(),
    }
}

struct Graph {
    owners: BTreeMap<PathBuf, Owner>,
    nodes: BTreeMap<Key, Node>,
    execute: BTreeSet<Key>,
    stderr: bool,
}

impl Graph {
    fn new(stderr: bool) -> Self {
        Self {
            owners: BTreeMap::new(),
            nodes: BTreeMap::new(),
            execute: BTreeSet::new(),
            stderr,
        }
    }

    fn load_owner(&mut self, root: &Path) -> Result<()> {
        if self.owners.contains_key(root) {
            return Ok(());
        }
        let project = if root.join(sources::config::CONFIG).is_file() {
            let _lock = crate::project_lock(root, true)?;
            let project = Project::load(root.to_owned())?;
            project.validate()?;
            Some(project)
        } else {
            None
        };
        let manifest: serde_json::Value = sources::config::read_json(&root.join("package.json"))?;
        self.owners.insert(
            root.to_owned(),
            Owner {
                project,
                root_name: manifest["name"].as_str().unwrap_or("//").to_owned(),
                tasks: BTreeMap::new(),
            },
        );
        Ok(())
    }

    fn discover(&mut self, mut frontier: BTreeSet<Key>) -> Result<()> {
        while !frontier.is_empty() {
            let mut batches: BTreeMap<PathBuf, Vec<String>> = BTreeMap::new();
            for (root, id) in frontier {
                if !self.nodes.contains_key(&(root.clone(), id.clone())) {
                    batches.entry(root).or_default().push(id);
                }
            }
            frontier = BTreeSet::new();
            for (root, ids) in batches {
                self.load_owner(&root)?;
                let owner = self.owners.get_mut(&root).expect("loaded owner");
                let _lock = owner
                    .project
                    .as_ref()
                    .map(|_| crate::project_lock(&root, true))
                    .transpose()?;
                let missing = ids
                    .iter()
                    .filter(|id| !owner.tasks.contains_key(*id))
                    .cloned()
                    .collect::<Vec<_>>();
                if !missing.is_empty() {
                    let candidates = owner
                        .project
                        .as_ref()
                        .map(configured_selectors)
                        .transpose()?
                        .unwrap_or_default();
                    let mut source = selection(
                        &root,
                        missing,
                        candidates
                            .keys()
                            .map(|name| (name.clone(), String::new()))
                            .collect(),
                    );
                    let (_, metadata) = query(&mut source)?;
                    owner.ingest(metadata);
                }
                let mut names = BTreeMap::new();
                for id in &ids {
                    let task = &owner.tasks[id];
                    ensure!(
                        !task.command.trim().is_empty() && task.command != "<NONEXISTENT>",
                        "{}: selected Turbo task {id:?} has no executable package script",
                        root.display()
                    );
                    names.insert(id.clone(), owner.names(id));
                }
                let all_names = names
                    .values()
                    .flat_map(|names| names.iter().cloned())
                    .collect();
                let available = owner
                    .project
                    .as_ref()
                    .map(|project| inputs_for(project, &all_names))
                    .transpose()?
                    .unwrap_or_default();
                for id in ids {
                    let inputs: Inputs = available
                        .iter()
                        .filter(|(name, _)| names[&id].contains(*name))
                        .map(|(name, input)| (name.clone(), input.clone()))
                        .collect();
                    for input in inputs.values() {
                        if input.content.is_some() {
                            self.execute.insert(input.producer.clone());
                        }
                        if !self.nodes.contains_key(&input.producer) {
                            frontier.insert(input.producer.clone());
                        }
                    }
                    self.nodes
                        .insert((root.clone(), id), Node { inputs, hash: None });
                }
            }
        }
        self.check_cycles()
    }

    fn check_cycles(&self) -> Result<()> {
        fn visit(
            key: &Key,
            nodes: &BTreeMap<Key, Node>,
            path: &mut Vec<Key>,
            done: &mut BTreeSet<Key>,
        ) -> Result<()> {
            if done.contains(key) {
                return Ok(());
            }
            if let Some(start) = path.iter().position(|entry| entry == key) {
                let cycle = path[start..]
                    .iter()
                    .chain(std::iter::once(key))
                    .map(|(root, id)| format!("{}:{id}", root.display()))
                    .collect::<Vec<_>>()
                    .join(" -> ");
                anyhow::bail!(
                    "Turbo source task hash cycle: {cycle}; remove a cyclic SOURCES_* task env dependency in turbo.json"
                );
            }
            path.push(key.clone());
            for input in nodes[key].inputs.values() {
                visit(&input.producer, nodes, path, done)?;
            }
            path.pop();
            done.insert(key.clone());
            Ok(())
        }
        let mut done = BTreeSet::new();
        for key in self.nodes.keys() {
            visit(key, &self.nodes, &mut Vec::new(), &mut done)?;
        }
        Ok(())
    }

    fn environment(&self, inputs: &Inputs) -> Result<(BTreeMap<String, String>, Vec<SourceTasks>)> {
        let mut env = BTreeMap::new();
        let mut groups = Vec::new();
        for (name, input) in inputs {
            let (root, id) = &input.producer;
            let value = if let Some(content) = &input.content {
                let digest = crate::artifacts::digest(root, content)?;
                let mut source = selection(root, vec![id.clone()], BTreeMap::new());
                source.content.insert(id.clone(), content.clone());
                source.digests.insert(id.clone(), digest.clone());
                groups.push(source);
                digest
            } else {
                self.nodes[&input.producer]
                    .hash
                    .clone()
                    .expect("settled input")
            };
            env.insert(name.clone(), value);
        }
        Ok((env, groups))
    }

    fn settle(&mut self) -> Result<()> {
        while self.nodes.values().any(|node| node.hash.is_none()) {
            let mut batches: BTreeMap<PathBuf, Vec<String>> = BTreeMap::new();
            for ((root, id), node) in &self.nodes {
                if node.hash.is_none()
                    && node
                        .inputs
                        .values()
                        .all(|input| self.nodes[&input.producer].hash.is_some())
                {
                    batches.entry(root.clone()).or_default().push(id.clone());
                }
            }
            for (root, ids) in batches {
                let inputs: Inputs = ids
                    .iter()
                    .flat_map(|id| self.nodes[&(root.clone(), id.clone())].inputs.clone())
                    .collect();
                let (env, groups) = self.environment(&inputs)?;
                let mut source = selection(&root, ids.clone(), env);
                let hashes = {
                    let owner = &self.owners[&root];
                    let _lock = owner
                        .project
                        .as_ref()
                        .map(|_| crate::project_lock(&root, true))
                        .transpose()?;
                    if let Some(project) = &owner.project {
                        refresh_and_verify(project, &groups, &source.env, self.stderr)?;
                    }
                    query(&mut source)?.0
                };
                // Native scripts may call Sources in another owner and return to
                // a different task here. No project lock crosses execution.
                let args: BTreeMap<_, _> = source
                    .ids
                    .iter()
                    .cloned()
                    .zip(source.args.clone())
                    .collect();
                source
                    .ids
                    .retain(|id| self.execute.contains(&(root.clone(), id.clone())));
                if !source.ids.is_empty() {
                    source.args = source.ids.iter().map(|id| args[id].clone()).collect();
                    execute_upstream(&source, self.stderr)?;
                }
                for id in ids {
                    self.nodes
                        .get_mut(&(root.clone(), id.clone()))
                        .expect("discovered node")
                        .hash = Some(hashes[&id].clone());
                }
            }
        }
        Ok(())
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
    let groups = {
        let _lock = crate::project_lock(&project.root, true)?;
        prepare(project, selectors)?
    };
    let roots = groups
        .iter()
        .flat_map(|source| {
            source
                .ids
                .iter()
                .map(|id| Ok((std::fs::canonicalize(&source.checkout)?, id.clone())))
        })
        .collect::<Result<BTreeSet<_>>>()?;
    let mut graph = Graph::new(false);
    graph.discover(roots.clone())?;
    graph.execute.extend(roots);
    graph.settle()?;
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

fn execute_upstream(source: &SourceTasks, stderr: bool) -> Result<()> {
    let mut command = command(&source.checkout)?;
    command.arg("run").args(&source.args).envs(&source.env);
    if stderr {
        command.stdout(std::io::stderr());
    }
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
    stderr: bool,
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
        crate::pnpm::refresh_frozen(project, env, stderr)?;
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
