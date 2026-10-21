use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use serde_json::Value;
use sources::config::{self, Project};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::str::FromStr;
use yaml_edit::{Mapping, YamlFile};

const WORKSPACE: &str = "pnpm-workspace.yaml";

pub struct Plan {
    root: PathBuf,
    workspace: PathBuf,
    contents: String,
    packages: BTreeSet<String>,
    repair: String,
    install: bool,
    offline: bool,
}

fn ordinary_file(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("cannot read pnpm file {}", path.display()))?;
    ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink(),
        "pnpm file {} must be an ordinary file, not a symlink",
        path.display()
    );
    Ok(())
}

fn workspace(path: &Path) -> Result<(YamlFile, Mapping)> {
    ordinary_file(path)?;
    let yaml = YamlFile::from_str(&fs::read_to_string(path)?)
        .with_context(|| format!("invalid {}", path.display()))?;
    let documents: Vec<_> = yaml.documents().collect();
    ensure!(
        documents.len() == 1,
        "{WORKSPACE} must contain one YAML document"
    );
    let root = documents[0]
        .as_mapping()
        .with_context(|| format!("{WORKSPACE} must contain a mapping"))?;
    Ok((yaml, root))
}

fn dev_reference(project: &Project, source: &str, package_path: &Path) -> Result<String> {
    let path = project.links().join(source).join(package_path);
    #[cfg(unix)]
    ensure!(
        !path.as_os_str().as_encoded_bytes().contains(&b'\\'),
        "pnpm local paths cannot contain backslashes on Unix: {}; use a source link directory and package path without backslashes",
        path.display()
    );
    let path = path.strip_prefix(&project.root).unwrap_or(&path);
    let text = path
        .to_str()
        .context("pnpm package paths must be valid Unicode")?
        .replace('\\', "/");
    Ok(if path.is_absolute() {
        format!("file:{text}")
    } else {
        format!("file:./{text}")
    })
}

fn overlaps_package(selector: &str, package: &str) -> bool {
    let target = selector.rsplit('>').next().unwrap_or(selector).trim();
    target == package
        || target
            .strip_prefix(package)
            .is_some_and(|tail| tail.starts_with('@'))
}

fn validate_overrides(mapping: &Mapping, packages: &BTreeSet<String>) -> Result<()> {
    for key in mapping.keys() {
        let key = key
            .as_scalar()
            .context("pnpm override keys must be strings")?
            .as_string();
        for package in packages {
            ensure!(
                !overlaps_package(&key, package) || key == *package,
                "pnpm override {key:?} conflicts with managed package {package}; remove it and use the managed {WORKSPACE} override"
            );
        }
    }
    Ok(())
}

fn validate_root_manifest(project: &Project) -> Result<()> {
    let path = project.root.join("package.json");
    ordinary_file(&path)?;
    let manifest: Value = config::read_json(&path)?;
    ensure!(
        manifest.pointer("/pnpm/overrides").is_none(),
        "package.json pnpm.overrides shadows managed YAML overrides in pnpm 10; move its entries to {WORKSPACE} overrides and remove package.json pnpm.overrides"
    );
    Ok(())
}

#[derive(Deserialize)]
struct Member {
    name: Option<String>,
}

pub(crate) fn pnpm_command(root: &Path) -> Command {
    #[cfg(windows)]
    let program = {
        let search_path = std::env::var_os("PATH").unwrap_or_default();
        std::env::split_paths(&search_path)
            .find_map(|directory| {
                ["pnpm.exe", "pnpm.cmd"]
                    .into_iter()
                    .map(|name| directory.join(name))
                    .find(|path| path.is_file())
            })
            .unwrap_or_else(|| PathBuf::from("pnpm.cmd"))
    };
    #[cfg(not(windows))]
    let program = "pnpm";
    let mut command = Command::new(program);
    command.current_dir(root);
    command
}

fn check_version(root: &Path) -> Result<()> {
    let output = pnpm_command(root)
        .arg("--version")
        .output()
        .context("cannot start pnpm; install pnpm 10.15.0 or newer and rerun the command")?;
    ensure!(
        output.status.success(),
        "cannot determine pnpm version: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let version = semver::Version::parse(String::from_utf8_lossy(&output.stdout).trim())
        .context("pnpm --version did not return a semantic version")?;
    ensure!(
        version >= semver::Version::new(10, 15, 0),
        "pnpm {version} is unsupported; install pnpm 10.15.0 or newer for managed YAML overrides"
    );
    Ok(())
}

fn check_members(root: &Path, packages: &BTreeSet<String>) -> Result<()> {
    let output = pnpm_command(root)
        .args(["-r", "list", "--depth", "-1", "--json"])
        .output()
        .context("cannot start pnpm; install pnpm and rerun sources dev or sources release")?;
    ensure!(
        output.status.success(),
        "cannot discover pnpm workspace packages: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let members: Vec<Member> = serde_json::from_slice(&output.stdout)
        .context("pnpm workspace discovery did not return valid JSON")?;
    for name in members.into_iter().filter_map(|member| member.name) {
        ensure!(
            !packages.contains(&name),
            "managed pnpm package {name} is also a workspace member; exclude sources/** (or the configured source directory) from {WORKSPACE} packages and rerun the command"
        );
    }
    Ok(())
}

impl Plan {
    pub fn prepare(project: &Project, registry: bool, offline: bool) -> Result<Option<Self>> {
        let pins = project.pins(false)?;
        let mut references = BTreeMap::new();
        for (name, source) in &project.config.sources {
            for (package, mapping) in &source.pnpm {
                references.insert(
                    package.clone(),
                    if registry {
                        pins.sources.get(name).and_then(|entry| entry.pnpm.get(package))
                            .with_context(|| format!("missing registry pin for {package}; run sources lock --registry {package}:<version>"))?.clone()
                    } else {
                        dev_reference(project, name, &mapping.path)?
                    },
                );
            }
        }
        if references.is_empty() {
            return Ok(None);
        }
        validate_root_manifest(project)?;
        check_version(&project.root)?;
        let packages: BTreeSet<_> = references.keys().cloned().collect();
        let path = project.root.join(WORKSPACE);
        let (yaml, root) = workspace(&path)?;
        let overrides = match root.get("overrides") {
            Some(node) => node
                .as_mapping()
                .cloned()
                .context("pnpm overrides must be an explicit YAML mapping")?,
            None => {
                root.set("overrides", Mapping::new());
                root.get_mapping("overrides")
                    .context("cannot create pnpm overrides mapping")?
            }
        };
        validate_overrides(&overrides, &packages)?;
        // Reusing file references must not refresh the dependency lock merely
        // because the stable link now points to another checkout.
        let same_references = references.iter().all(|(package, reference)| {
            overrides
                .get(package.as_str())
                .is_some_and(|value| value.yaml_eq(reference))
        });
        let lock_matches = workspace(&project.root.join("pnpm-lock.yaml"))
            .ok()
            .and_then(|(_, root)| root.get_mapping("overrides"))
            .is_some_and(|overrides| {
                references.iter().all(|(package, reference)| {
                    overrides
                        .get(package.as_str())
                        .is_some_and(|value| value.yaml_eq(reference))
                })
            });
        let install = registry || !same_references || !lock_matches;
        for (package, reference) in references {
            overrides.set(package.as_str(), reference.as_str());
        }
        // Validate the existing workspace now, and recheck after the caller has
        // activated links: broad workspace globs can expose new local members.
        check_members(&project.root, &packages)?;
        Ok(Some(Self {
            root: project.root.clone(),
            workspace: path,
            contents: yaml.to_string(),
            packages,
            repair: if registry {
                "sources release --registry"
            } else {
                "pnpm install --no-frozen-lockfile, then sources dev or sources release --git"
            }
            .into(),
            install,
            offline,
        }))
    }

    pub fn apply(self) -> Result<()> {
        check_members(&self.root, &self.packages)?;
        if !self.install {
            return Ok(());
        }
        ordinary_file(&self.workspace)?;
        let mut temporary = tempfile::NamedTempFile::new_in(&self.root)?;
        temporary.write_all(self.contents.as_bytes())?;
        temporary.as_file().sync_all()?;
        temporary
            .persist(&self.workspace)
            .context("cannot update pnpm workspace overrides")?;
        let mut command = pnpm_command(&self.root);
        command.args(["install", "--no-frozen-lockfile"]);
        if self.offline {
            command.arg("--offline");
        }
        let repair = &self.repair;
        let status = crate::run::wait_command(&mut command, "pnpm install")
            .with_context(|| format!("cannot complete pnpm install; rerun {repair}"))?;
        ensure!(
            status.success(),
            "pnpm install failed ({status}); selection files may already have changed; fix the pnpm error and rerun {repair}"
        );
        Ok(())
    }
}

pub fn source_mode(project: &Project, name: &str) -> Result<String> {
    let pins = project.pins(false)?;
    let source = project
        .config
        .sources
        .get(name)
        .context("unknown pnpm source")?;
    validate_root_manifest(project)?;
    let (_, root) = workspace(&project.root.join(WORKSPACE))?;
    let overrides = match root.get("overrides") {
        Some(node) => Some(
            node.as_mapping()
                .cloned()
                .context("pnpm overrides must be an explicit YAML mapping")?,
        ),
        None => None,
    };
    if let Some(overrides) = &overrides {
        validate_overrides(overrides, &source.pnpm.keys().cloned().collect())?;
    }
    let mut modes = BTreeSet::new();
    for (package, mapping) in &source.pnpm {
        let reference = overrides
            .as_ref()
            .and_then(|overrides| overrides.get(package.as_str()));
        modes.insert(match reference {
            None => "unconfigured",
            Some(value)
                if pins
                    .sources
                    .get(name)
                    .and_then(|entry| entry.pnpm.get(package))
                    .is_some_and(|version| value.yaml_eq(version)) =>
            {
                "registry"
            }
            Some(value) if value.yaml_eq(&dev_reference(project, name, &mapping.path)?) => "file",
            Some(_) => "mixed",
        });
    }
    Ok(if modes.len() == 1 {
        *modes.first().unwrap()
    } else {
        "mixed"
    }
    .into())
}

/// Validate package paths in the checkout selected by this operation.
pub fn validate_packages(project: &Project, managed: bool) -> Result<()> {
    for (name, source) in &project.config.sources {
        if source.pnpm.is_empty() {
            continue;
        }
        let checkout = if managed {
            project.managed(name)
        } else {
            project.dev(name)?
        };
        for (package, mapping) in &source.pnpm {
            let package_root =
                fs::canonicalize(checkout.join(&mapping.path)).with_context(|| {
                    format!(
                        "{name}: missing pnpm package path {}",
                        mapping.path.display()
                    )
                })?;
            ensure!(
                package_root.starts_with(fs::canonicalize(&checkout)?),
                "{name}: pnpm package path escapes the selected checkout"
            );
            let path = package_root.join("package.json");
            let manifest: Value = config::read_json(&path)?;
            ensure!(
                manifest["name"].as_str() == Some(package),
                "{name}: {} must declare package name {package}",
                path.display()
            );
        }
    }
    Ok(())
}

pub fn download(project: &Project, packages: &[String]) -> Result<()> {
    check_version(&project.root)?;
    let status = crate::run::wait_command(
        pnpm_command(&project.root)
            .args(["store", "add"])
            .args(packages),
        "pnpm store add",
    )?;
    ensure!(
        status.success(),
        "registry download failed; lockfile unchanged; rerun sources lock --registry --check with the selected versions"
    );
    Ok(())
}
