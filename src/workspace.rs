use anyhow::{Context, Result, ensure};
use serde::Serialize;
use sources::acquisition::activation_issues;
use sources::acquisition::{Acquisition, activate, ensure_revision};
use sources::config::{self, Project};
use sources::git::Git;
use sources::git::{Change, redact};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

fn preflight_links(project: &Project) -> Result<()> {
    config::real_directory_if_present(&project.links())?;
    for name in project.config.sources.keys() {
        let link = project.links().join(name);
        match fs::symlink_metadata(&link) {
            Ok(metadata) => ensure!(
                metadata.file_type().is_symlink(),
                "workspace path {} is an ordinary file or directory; move it elsewhere and retry",
                link.display()
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| format!("cannot inspect {}", link.display()));
            }
        }
    }
    Ok(())
}

#[cfg(unix)]
fn link_directory(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
fn link_directory(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_dir(target, link)
}

fn unlink(path: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::FileTypeExt;
        if fs::symlink_metadata(path)?.file_type().is_symlink_dir() {
            // Windows directory links need RemoveDirectory, even when dangling.
            // This is nonrecursive and removes the link object, never its target.
            return fs::remove_dir(path);
        }
    }
    fs::remove_file(path)
}

fn activate_links(project: &Project, targets: &BTreeMap<String, PathBuf>) -> Result<()> {
    fs::create_dir_all(project.links())?;
    for entry in fs::read_dir(project.links())? {
        let entry = entry?;
        if entry.file_type()?.is_symlink()
            && !project
                .config
                .sources
                .keys()
                .any(|name| entry.file_name() == name.as_str())
        {
            unlink(&entry.path()).with_context(|| {
                format!(
                    "cannot remove stale workspace link {}",
                    entry.path().display()
                )
            })?;
        }
    }
    for name in project
        .config
        .sources
        .keys()
        .filter(|name| !targets.contains_key(*name))
    {
        let link = project.links().join(name);
        if fs::symlink_metadata(&link).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
            unlink(&link)
                .with_context(|| format!("cannot remove workspace link {}", link.display()))?;
        }
    }
    for (name, target) in targets {
        let link = project.links().join(name);
        let target = target.canonicalize().with_context(|| {
            format!("{name}: checkout unavailable; rerun sources dev or sources release")
        })?;
        if fs::read_link(&link).is_ok() {
            if link.canonicalize().is_ok_and(|actual| actual == target) {
                continue;
            }
            unlink(&link)
                .with_context(|| format!("cannot replace workspace link {}", link.display()))?;
        }
        link_directory(&target, &link).with_context(|| format!("cannot link {} to {}; check directory permissions and rerun the command; on Windows enable Developer Mode or run with symlink privileges", link.display(), target.display()))?;
    }
    Ok(())
}

pub fn dev(project: &Project) -> Result<()> {
    preflight_links(project)?;
    crate::pnpm::validate_packages(project, false)?;
    let pnpm = crate::pnpm::Plan::prepare(project, false, false)?;
    let mut targets = BTreeMap::new();
    for (name, source) in &project.config.sources {
        let path = project.dev(name)?;
        let repo = Git::at(&path);
        repo.validate(&source.url, false)
            .with_context(|| format!("{name}: invalid development checkout"))?;
        crate::commands::warn_changes(name, &repo)?;
        targets.insert(name.clone(), path);
        let managed = project.managed(name);
        if managed.exists() {
            let repo = Git::at(&managed);
            match repo
                .validate(&source.url, true)
                .and_then(|()| crate::commands::warn_changes(name, &repo).map(|_| ()))
            {
                Ok(()) => {}
                Err(error) => eprintln!(
                    "warning: {name}: managed cache needs repair: {error:#}; run sources release --clean"
                ),
            }
        }
    }
    activate_links(project, &targets)?;
    if let Some(plan) = pnpm {
        plan.apply()?;
    }
    println!("Development mode activated");
    Ok(())
}

fn remove_managed(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
            fs::remove_dir_all(path)?
        }
        Ok(_) => unlink(path)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

pub fn release(project: &Project, clean: bool, offline: bool, registry: bool) -> Result<()> {
    let selection = if registry { " --registry" } else { " --git" };
    let retry = format!(
        "sources release{selection}{}",
        if clean {
            " --clean"
        } else if offline {
            " --offline"
        } else {
            ""
        }
    );
    let clean_retry = format!("sources release{selection} --clean");
    let pins = project.pins(false)?;
    for (name, source) in &project.config.sources {
        if !(registry && source.has_packages()) {
            ensure!(
                pins.sources
                    .get(name)
                    .and_then(|entry| entry.git.as_ref())
                    .is_some(),
                "missing lock entry for {name}; run sources lock --git"
            );
        }
    }
    preflight_links(project)?;
    let pnpm = crate::pnpm::Plan::prepare(project, registry, offline)?;
    let checkouts = project.internal().join("checkouts");
    if !clean {
        config::real_directory_if_present(&checkouts)?;
    }
    for (name, source) in &project.config.sources {
        if registry && source.has_packages() && !clean {
            continue;
        }
        let path = project.managed(name);
        if fs::symlink_metadata(&path).is_ok() {
            let repo = Git::at(&path);
            let inspected = repo
                .validate(&source.url, true)
                .and_then(|()| crate::commands::warn_changes(name, &repo));
            if clean {
                if inspected.unwrap_or(true) {
                    eprintln!(
                        "warning: {name}: clean recreation discards all managed contents, including edits"
                    );
                }
            } else {
                ensure!(
                    !inspected.with_context(|| format!(
                        "{name}: invalid managed checkout; run {clean_retry}"
                    ))?,
                    "{name}: managed checkout has source changes; preserve them elsewhere or run {clean_retry}"
                );
            }
        }
    }
    if clean {
        remove_managed(&checkouts).context("cannot remove disposable managed checkouts")?;
        for entry in fs::read_dir(project.internal())? {
            let entry = entry?;
            if entry.file_name() != "project.lock" {
                remove_managed(&entry.path())?;
            }
        }
    }
    let mut revisions = BTreeMap::new();
    for (name, source) in &project.config.sources {
        if registry && source.has_packages() {
            continue;
        }
        let policy = if offline {
            Acquisition::Offline
        } else {
            Acquisition::EnsureCached
        };
        let revision = ensure_revision(
            &project.managed(name),
            &source.url,
            pins.sources[name].git.as_deref(),
            policy,
            &project.internal(),
        )
        .with_context(|| {
            format!("{name}: release preparation failed; rerun {retry} after sources fetch {name}, or use {clean_retry}")
        })?;
        revisions.insert(name.clone(), revision);
    }
    sources::release_graph::validate(
        project,
        &revisions,
        if offline {
            Acquisition::Offline
        } else {
            Acquisition::EnsureCached
        },
        registry,
    )?;
    let mut targets = BTreeMap::new();
    for (name, revision) in &revisions {
        let path = project.managed(name);
        activate(&path, revision).with_context(|| {
            format!("{name}: activation failed; rerun {retry} or {clean_retry}")
        })?;
        targets.insert(name.clone(), path);
    }
    if !registry {
        crate::pnpm::validate_packages(project, true)?;
    }
    activate_links(project, &targets)?;
    let installed_pnpm = pnpm.is_some();
    if let Some(plan) = pnpm {
        plan.apply()?;
    }
    if offline {
        if installed_pnpm {
            println!(
                "Release mode activated using cached Git inputs and pnpm offline installation"
            );
        } else {
            println!("Release mode activated using cached inputs without network access");
        }
    } else {
        println!("Release mode activated");
    }
    if !clean && !project.config.sources.is_empty() {
        eprintln!("warning: retained generated outputs may require rebuilding");
    }
    Ok(())
}

#[derive(Serialize)]
struct SourceStatus {
    upstream: String,
    release_kind: String,
    release_packages: BTreeMap<String, String>,
    pnpm_selection: Option<String>,
    workspace_path: PathBuf,
    link_target: Option<PathBuf>,
    development_path: Option<PathBuf>,
    active_mode: String,
    active_commit: Option<String>,
    locked_commit: Option<String>,
    cached_commit_available: bool,
    active_matches_locked: Option<bool>,
    dirty: bool,
    changes: Vec<Change>,
    submodule_issues: Vec<String>,
    errors: Vec<String>,
}

#[derive(Serialize)]
struct Status {
    version: u32,
    project_root: PathBuf,
    mode: String,
    sources: BTreeMap<String, SourceStatus>,
}

pub fn status(project: &Project, json: bool) -> Result<i32> {
    let pins = project.pins(false)?;
    let mut rows = BTreeMap::new();
    let mut active_links = 0;
    for (name, source) in &project.config.sources {
        let path = project.links().join(name);
        let dev = project
            .local
            .sources
            .get(name)
            .map(|path| project.root.join(path));
        let mut row = SourceStatus {
            upstream: redact(&source.url),
            release_kind: "git".into(),
            release_packages: pins
                .sources
                .get(name)
                .map(|entry| entry.pnpm.clone())
                .unwrap_or_default(),
            pnpm_selection: None,
            workspace_path: path.clone(),
            link_target: None,
            development_path: dev.clone(),
            active_mode: "missing".into(),
            active_commit: None,
            locked_commit: pins.sources.get(name).and_then(|entry| entry.git.clone()),
            cached_commit_available: false,
            active_matches_locked: None,
            dirty: false,
            changes: Vec::new(),
            submodule_issues: Vec::new(),
            errors: Vec::new(),
        };
        let managed = project.managed(name);
        if fs::symlink_metadata(&managed).is_ok() {
            let git = Git::at(&managed);
            match git.validate(&source.url, true) {
                Ok(()) => {
                    if let Some(pin) = &row.locked_commit {
                        match ensure_revision(
                            &managed,
                            &source.url,
                            Some(pin),
                            Acquisition::Offline,
                            &project.internal(),
                        ) {
                            Ok(_) => row.cached_commit_available = true,
                            Err(error) => row.errors.push(redact(&format!(
                                "locked inputs are not fully cached: {error:#}; run sources fetch"
                            ))),
                        }
                    }
                }
                Err(error) => row.errors.push(redact(&format!(
                    "managed cache needs repair: {error:#}; run sources release --clean"
                ))),
            }
        }
        if source.has_packages() {
            match crate::pnpm::source_mode(project, name) {
                Ok(selection) => {
                    row.pnpm_selection = Some(selection.clone());
                    if selection == "registry" {
                        row.release_kind = "pnpm".into();
                        active_links += 1;
                        row.active_mode = "release".into();
                        if fs::symlink_metadata(&path).is_ok() {
                            row.errors.push("published package selection still has a workspace entry; run sources release --registry".into());
                        }
                        rows.insert(name.clone(), row);
                        continue;
                    }
                    if selection == "mixed" {
                        row.errors.push(
                            "pnpm references are inconsistent; run sources dev or sources release"
                                .into(),
                        );
                    }
                }
                Err(error) => row.errors.push(redact(&format!(
                    "{error:#}; run sources dev or sources release"
                ))),
            }
        }
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                active_links += 1;
                let inspect = (|| -> Result<()> {
                    let raw_target = fs::read_link(&path)?;
                    let target = if raw_target.is_absolute() { raw_target } else { project.links().join(raw_target) };
                    row.link_target = Some(target.clone());
                    let target = config::resolved_path(&target)?;
                    let managed = config::resolved_path(&project.managed(name))?;
                    let dev_matches = dev.as_ref().map(|path| config::resolved_path(path)).transpose()?.is_some_and(|dev| dev == target);
                    let managed_mode = target == managed;
                    row.active_mode = if managed_mode { "release" } else if dev_matches { "dev" } else { "unexpected" }.into();
                    ensure!(managed_mode || dev_matches, "unexpected workspace link target; run sources dev or sources release");
                    ensure!(path.exists(), "broken workspace link; run sources dev or sources release --clean");
                    let git = Git::at(&target);
                    git.validate(&source.url, managed_mode)?;
                    row.active_commit = git.head()?;
                    row.changes = git.changes()?;
                    row.dirty = !row.changes.is_empty();
                    row.active_matches_locked = row.active_commit.as_ref().zip(row.locked_commit.as_ref()).map(|(a,b)| a == b);
                    row.submodule_issues = activation_issues(&target)?;
                    if managed_mode {
                        ensure!(row.active_commit.is_some(), "managed checkout is not activated; run sources release");
                        ensure!(row.submodule_issues.is_empty(), "incomplete submodule activation; run sources release");
                    }
                    Ok(())
                })();
                if let Err(error) = inspect { row.errors.push(redact(&format!("{error:#}"))); }
            },
            Ok(_) => row.errors.push("workspace path is an ordinary file or directory; move it elsewhere and run sources dev or sources release".into()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
            Err(error) => row.errors.push(format!("cannot inspect workspace link: {error}")),
        }
        if source.has_packages() {
            if (row.active_mode == "dev" || row.active_mode == "release")
                && row.pnpm_selection.as_deref() != Some("file")
            {
                row.errors.push("checkout links and pnpm references disagree; run sources dev or sources release".into());
                row.active_mode = "mixed".into();
            }
            if row.pnpm_selection.as_deref() == Some("file")
                && row.active_mode != "dev"
                && row.active_mode != "release"
            {
                row.errors.push(
                    "pnpm file references have no valid checkout link; run sources dev or sources release --git".into(),
                );
            }
        }
        rows.insert(name.clone(), row);
    }
    let mode = if rows.is_empty() || active_links == 0 {
        "unconfigured"
    } else if rows.values().all(|row| row.active_mode == "dev") {
        "dev"
    } else if rows.values().all(|row| row.active_mode == "release") {
        "release"
    } else {
        "mixed"
    };
    if active_links > 0 {
        for row in rows.values_mut().filter(|row| row.active_mode == "missing") {
            row.errors.push("missing workspace link during partial activation; run sources dev or sources release".into());
        }
    }
    let state = Status {
        version: 2,
        project_root: project.root.clone(),
        mode: mode.into(),
        sources: rows,
    };
    if json {
        println!("{}", serde_json::to_string_pretty(&state)?);
    } else {
        println!("mode: {}", state.mode);
        for (name, row) in &state.sources {
            println!(
                "{name}: {}{}",
                row.active_mode,
                if row.dirty { " (source changes)" } else { "" }
            );
            if row.pnpm_selection.is_some() {
                println!(
                    "  pnpm references: {}; installed dependencies are not inspected",
                    row.pnpm_selection.as_deref().unwrap_or("unavailable")
                );
                for (package, version) in &row.release_packages {
                    println!("  package release: {package}@{version}");
                }
            }
            println!(
                "  upstream: {}\n  workspace: {}",
                row.upstream,
                row.workspace_path.display()
            );
            println!(
                "  target: {}",
                row.link_target
                    .as_ref()
                    .map_or_else(|| "missing".into(), |p| p.display().to_string())
            );
            if let Some(dev) = &row.development_path {
                println!("  development: {}", dev.display());
            }
            if row.pnpm_selection.as_deref() != Some("registry") {
                println!(
                    "  active: {}\n  locked: {}",
                    row.active_commit.as_deref().unwrap_or("unavailable"),
                    row.locked_commit.as_deref().unwrap_or("unpinned")
                );
                println!("  locked commit cached: {}", row.cached_commit_available);
            } else if let Some(commit) = &row.locked_commit {
                println!("  locked Git commit: {commit}");
            }
            if let Some(matches) = row.active_matches_locked {
                println!("  active matches lock: {matches}");
            }
            for issue in &row.submodule_issues {
                println!("  submodule: {issue}");
            }
        }
    }
    let mut failed = false;
    for (name, row) in &state.sources {
        for change in &row.changes {
            eprintln!("warning: {name}: {} {}", change.status, change.path);
        }
        for issue in &row.submodule_issues {
            eprintln!("warning: {name}: {issue}");
        }
        for error in &row.errors {
            eprintln!("error: {name}: {error}");
            failed = true;
        }
    }
    Ok(i32::from(failed))
}
