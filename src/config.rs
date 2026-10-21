use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};

pub const CONFIG: &str = ".sources.conf";
pub const LOCAL: &str = ".sources.local.conf";
pub const LOCK: &str = ".sources.lock";

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub url: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub pnpm: BTreeMap<String, PnpmPackage>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub packages: BTreeMap<String, TurboPackage>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PnpmPackage {
    pub path: PathBuf,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TurboPackage {
    pub tasks: Vec<String>,
}

/// Stable environment identity for one exact child Turbo package/task pair.
pub fn turbo_env_name(package: &str, task: &str) -> Result<String> {
    ensure!(
        !package.contains("__"),
        "Turbo package name {package:?} contains reserved delimiter '__'"
    );
    ensure!(
        !package.is_empty()
            && !package.starts_with('-')
            && !package.contains([':', '#'])
            && !package.chars().any(char::is_control),
        "invalid Turbo package name {package:?}: use an exact package name without ':', '#', controls, or a leading '-'"
    );
    ensure!(
        !task.is_empty() && !task.contains('#') && !task.chars().any(char::is_control),
        "invalid Turbo task name {task:?}: use a nonempty exact task name without '#' or controls"
    );
    let component = |value: &str| {
        value
            .split(|c: char| !c.is_ascii_alphanumeric())
            .filter(|part| !part.is_empty())
            .map(str::to_ascii_uppercase)
            .collect::<Vec<_>>()
            .join("_")
    };
    let package_component = component(package);
    let task_component = component(task);
    ensure!(
        !package_component.is_empty(),
        "Turbo package {package:?} has an empty normalized environment component"
    );
    ensure!(
        !task_component.is_empty(),
        "Turbo task {task:?} has an empty normalized environment component"
    );
    Ok(format!("SOURCES_{package_component}__{task_component}"))
}

impl Source {
    pub fn has_packages(&self) -> bool {
        !self.pnpm.is_empty()
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub version: u32,
    pub directory: PathBuf,
    pub sources: BTreeMap<String, Source>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Local {
    pub sources: BTreeMap<String, PathBuf>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Pins {
    pub version: u32,
    pub sources: BTreeMap<String, SourcePins>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SourcePins {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub pnpm: BTreeMap<String, String>,
}

impl Default for Pins {
    fn default() -> Self {
        Self {
            version: 2,
            sources: BTreeMap::new(),
        }
    }
}

#[derive(Debug)]
pub struct Project {
    pub root: PathBuf,
    pub config: Config,
    pub local: Local,
}

pub fn discover(start: &Path) -> Result<Option<PathBuf>> {
    let start = fs::canonicalize(start)
        .with_context(|| format!("cannot open directory {}", start.display()))?;
    ensure!(start.is_dir(), "{} is not a directory", start.display());
    Ok(start
        .ancestors()
        .find(|p| p.join(CONFIG).is_file())
        .map(Path::to_path_buf))
}

pub fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let bytes = fs::read(path).with_context(|| format!("cannot read {}", path.display()))?;
    serde_json::from_slice(&bytes).with_context(|| format!("invalid JSON in {}", path.display()))
}

pub fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let parent = path.parent().context("file has no parent directory")?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer_pretty(&mut temp, value)?;
    temp.write_all(b"\n")?;
    temp.as_file().sync_all()?;
    temp.persist(path)
        .with_context(|| format!("cannot replace {}", path.display()))?;
    Ok(())
}

/// Resolve existing aliases, while retaining nonexistent descendants for preflight checks.
pub fn resolved_path(path: &Path) -> Result<PathBuf> {
    resolve_path(path, false)
}

pub(crate) fn upstream_path(path: &Path) -> Result<PathBuf> {
    resolve_path(path, true)
}

fn resolve_path(path: &Path, tolerate_missing_target: bool) -> Result<PathBuf> {
    resolve_path_with_limit(path, tolerate_missing_target, 40)
}

fn resolve_path_with_limit(
    path: &Path,
    tolerate_missing_target: bool,
    remaining: usize,
) -> Result<PathBuf> {
    ensure!(
        remaining > 0,
        "too many symbolic links while resolving {}",
        path.display()
    );
    let absolute = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut result = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                result.pop();
            }
            Component::Prefix(_) | Component::RootDir => result.push(component.as_os_str()),
            _ => {
                result.push(component.as_os_str());
                match fs::symlink_metadata(&result) {
                    Ok(metadata) => match fs::canonicalize(&result) {
                        Ok(path) => result = path,
                        Err(error)
                            if tolerate_missing_target
                                && metadata.file_type().is_symlink()
                                && error.kind() == std::io::ErrorKind::NotFound =>
                        {
                            let target = fs::read_link(&result)?;
                            let target = if target.is_absolute() {
                                target
                            } else {
                                result.parent().context("link has no parent")?.join(target)
                            };
                            result = resolve_path_with_limit(&target, true, remaining - 1)?;
                        }
                        Err(error) => {
                            return Err(error)
                                .with_context(|| format!("cannot resolve {}", result.display()));
                        }
                    },
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => {
                        return Err(error)
                            .with_context(|| format!("cannot inspect {}", result.display()));
                    }
                }
            }
        }
    }
    Ok(result)
}

pub fn real_directory_if_present(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(meta) => ensure!(
            meta.is_dir() && !meta.file_type().is_symlink(),
            "{} must be a real directory, not a link; remove the collision and retry",
            path.display()
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error).with_context(|| format!("cannot inspect {}", path.display()));
        }
    }
    Ok(())
}

fn overlaps(a: &Path, b: &Path) -> bool {
    a.starts_with(b) || b.starts_with(a)
}

impl Project {
    pub fn load(root: PathBuf) -> Result<Self> {
        let mut config: Config = read_json(&root.join(CONFIG))?;
        ensure!(
            config.version == 1,
            "unsupported configuration version {}",
            config.version
        );
        let local = if root.join(LOCAL).exists() {
            read_json(&root.join(LOCAL))?
        } else {
            Local::default()
        };
        for source in config.sources.values_mut() {
            ensure!(
                !source.url.trim().is_empty(),
                "missing upstream URL; edit {CONFIG}"
            );
            let scp_style = source
                .url
                .split_once(':')
                .is_some_and(|(host, _)| host.len() > 1 && !host.contains(['/', '\\']));
            if !source.url.contains("://") && !scp_style && Path::new(&source.url).is_relative() {
                let path = upstream_path(&root.join(&source.url))?;
                source.url = url::Url::from_file_path(&path)
                    .map_err(|()| anyhow::anyhow!("cannot resolve configured local upstream path"))?
                    .to_string();
            }
        }
        Ok(Self {
            root,
            config,
            local,
        })
    }

    pub fn internal(&self) -> PathBuf {
        self.root.join(".sources")
    }
    pub fn links(&self) -> PathBuf {
        self.root.join(&self.config.directory)
    }
    pub fn managed(&self, name: &str) -> PathBuf {
        self.internal().join("checkouts").join(name)
    }
    pub fn dev(&self, name: &str) -> Result<PathBuf> {
        let value = self
            .local
            .sources
            .get(name)
            .with_context(|| format!("missing development mapping for {name}; edit {LOCAL}"))?;
        Ok(self.root.join(value))
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.config.directory.as_os_str().is_empty(),
            "workspace link directory must not be empty"
        );
        real_directory_if_present(&self.internal())?;
        real_directory_if_present(&self.links())?;
        let internal = resolved_path(&self.internal())?;
        let links = resolved_path(&self.links())?;
        ensure!(
            !overlaps(&internal, &links),
            "workspace link directory overlaps .sources managed state"
        );
        ensure!(
            !self.root.starts_with(&links),
            "workspace link directory must not contain the project root"
        );
        for filename in [CONFIG, LOCAL, LOCK] {
            let path = resolved_path(&self.root.join(filename))?;
            ensure!(
                !path.starts_with(&links),
                "workspace link directory overlaps {filename}"
            );
        }
        for name in self.local.sources.keys() {
            ensure!(
                self.config.sources.contains_key(name),
                "unknown source {name} in {LOCAL}"
            );
        }
        let link_parent = links
            .ancestors()
            .find(|path| path.is_dir())
            .context("workspace directory has no existing ancestor")?;
        let probes = [
            tempfile::tempdir_in(self.internal())?,
            tempfile::tempdir_in(link_parent)?,
        ];
        let mut packages = std::collections::BTreeSet::new();
        let mut turbo_packages = std::collections::BTreeSet::new();
        let mut turbo_hashes = BTreeMap::new();
        for (name, source) in &self.config.sources {
            ensure!(
                !name.is_empty()
                    && name != "."
                    && name != ".."
                    && !name.contains(['/', '\\', '\0'])
                    && !name.chars().any(char::is_control),
                "invalid source directory name {name:?}"
            );
            #[cfg(windows)]
            ensure!(
                !name.contains([':', '*', '?', '"', '<', '>', '|']) && !name.ends_with(['.', ' ']),
                "invalid Windows source name {name:?}"
            );
            for probe in &probes {
                fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(probe.path().join(name))
                    .with_context(|| {
                        format!(
                            "source name is invalid or names collide on this filesystem: {name}"
                        )
                    })?;
            }
            ensure!(
                !source.url.trim().is_empty(),
                "missing upstream URL for {name}"
            );
            for (package, entry) in &source.packages {
                ensure!(
                    turbo_packages.insert(package),
                    "duplicate Turbo package {package}: each package must belong to exactly one source"
                );
                ensure!(
                    !entry.tasks.is_empty(),
                    "{name}: package {package} must declare at least one Turbo task"
                );
                let mut tasks = std::collections::BTreeSet::new();
                for task in &entry.tasks {
                    ensure!(
                        tasks.insert(task),
                        "{name}: duplicate Turbo task {task:?} for package {package}"
                    );
                    let variable = turbo_env_name(package, task)?;
                    if let Some((previous_package, previous_task)) =
                        turbo_hashes.insert(variable.clone(), (package, task))
                    {
                        bail!(
                            "Turbo environment name collision {variable}: {previous_package}:{previous_task} and {package}:{task}; rename one configured package or task"
                        );
                    }
                }
            }
            for (package, mapping) in &source.pnpm {
                let valid_part = |part: &str| {
                    !part.is_empty()
                        && part.bytes().all(|b| {
                            b.is_ascii_lowercase() || b.is_ascii_digit() || b"-_.".contains(&b)
                        })
                        && !part.starts_with(['.', '_'])
                };
                let valid_name = if let Some(scoped) = package.strip_prefix('@') {
                    scoped
                        .split_once('/')
                        .is_some_and(|(scope, name)| valid_part(scope) && valid_part(name))
                } else {
                    valid_part(package)
                };
                ensure!(valid_name, "{name}: invalid pnpm package name {package:?}");
                ensure!(packages.insert(package), "duplicate pnpm package {package}");
                ensure!(
                    !mapping.path.as_os_str().is_empty()
                        && mapping.path.components().all(|part| {
                            matches!(part, Component::Normal(_) | Component::CurDir)
                        }),
                    "{name}: package path for {package} must stay within its source checkout"
                );
            }
            if let Some(dev) = self.local.sources.get(name) {
                let dev = resolved_path(&self.root.join(dev))?;
                ensure!(
                    !overlaps(&dev, &internal) && !overlaps(&dev, &links),
                    "development checkout for {name} overlaps managed paths"
                );
            }
        }
        Ok(())
    }

    pub fn pins(&self, complete: bool) -> Result<Pins> {
        let path = self.root.join(LOCK);
        let pins: Pins = if path.exists() {
            read_json(&path)?
        } else if complete && !self.config.sources.is_empty() {
            bail!("missing {LOCK}; run sources lock");
        } else {
            Pins::default()
        };
        ensure!(
            pins.version == 2,
            "unsupported lock version {}",
            pins.version
        );
        for (name, entry) in &pins.sources {
            ensure!(
                self.config.sources.contains_key(name),
                "obsolete lock entry {name}; edit {LOCK}"
            );
            if let Some(hash) = &entry.git {
                ensure!(
                    [40, 64].contains(&hash.len()) && hash.bytes().all(|c| c.is_ascii_hexdigit()),
                    "invalid full commit ID for {name} in {LOCK}"
                );
            }
            for (package, version) in &entry.pnpm {
                ensure!(
                    self.config.sources[name].pnpm.contains_key(package),
                    "obsolete package pin {package} for {name} in {LOCK}"
                );
                ensure!(
                    semver::Version::parse(version).is_ok(),
                    "{name}: expected exact package version for {package}, such as 1.2.3"
                );
            }
        }
        if complete {
            for name in self.config.sources.keys() {
                ensure!(
                    pins.sources
                        .get(name)
                        .and_then(|entry| entry.git.as_ref())
                        .is_some(),
                    "missing lock entry for {name}; run sources lock"
                );
            }
        }
        Ok(pins)
    }
}

pub fn setup(root: PathBuf) -> Result<()> {
    if !root.join(CONFIG).exists() {
        write_json(
            &root.join(CONFIG),
            &Config {
                version: 1,
                directory: "sources".into(),
                sources: BTreeMap::new(),
            },
        )?;
        println!("Created {CONFIG}; add source names and upstream URLs");
    }
    let project = Project::load(root)?;
    project.validate()?;
    for (filename, value) in [
        (LOCAL, serde_json::to_value(Local::default())?),
        (LOCK, serde_json::to_value(Pins::default())?),
    ] {
        if !project.root.join(filename).exists() {
            write_json(&project.root.join(filename), &value)?;
            println!("Created {filename}");
        }
    }
    let ignore_path = project.root.join(".gitignore");
    if let Ok(metadata) = fs::symlink_metadata(&ignore_path) {
        ensure!(
            metadata.is_file() && !metadata.file_type().is_symlink(),
            ".gitignore must be an ordinary file; remove the collision and rerun sources setup"
        );
    }
    let mut ignore = match fs::read_to_string(&ignore_path) {
        Ok(value) => value,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e.into()),
    };
    let links = resolved_path(&project.links())?;
    let mut entries = vec!["/.sources/".to_owned(), "/.sources.local.conf".to_owned()];
    if let Ok(relative) = links.strip_prefix(&project.root) {
        let path = relative.to_string_lossy();
        #[cfg(windows)]
        let path = path.replace('\\', "/");
        let escaped = path
            .replace('\\', "\\\\")
            .replace(' ', "\\ ")
            .replace('[', "\\[")
            .replace(']', "\\]")
            .replace('*', "\\*")
            .replace('?', "\\?");
        entries.push(format!("/{escaped}/"));
    }
    for entry in entries {
        if !ignore.lines().any(|line| line == entry) {
            if !ignore.is_empty() && !ignore.ends_with('\n') {
                ignore.push('\n');
            }
            ignore.push_str(&entry);
            ignore.push('\n');
        }
    }
    fs::write(ignore_path, ignore)?;
    println!("Setup complete; edit {CONFIG} and {LOCAL}, then run sources check");
    Ok(())
}
