use anyhow::{Context, Result, bail, ensure};
use sources::acquisition::{Acquisition, ensure_revision, fetch_tag_commit};
use sources::config::{self, LOCK, Project};
use sources::git::Git;
use std::collections::BTreeSet;

#[derive(Clone, Copy, clap::ValueEnum)]
pub enum DevHintRef {
    Branch,
    Tag,
    Commit,
}

pub fn dev_hints(
    project: &Project,
    name: Option<&str>,
    reference: Option<DevHintRef>,
    hint: Option<&str>,
) -> Result<()> {
    let mut config: config::Config = config::read_json(&project.root.join(config::CONFIG))?;
    if let Some(name) = name {
        ensure!(config.sources.contains_key(name), "unknown source {name}");
    }
    if let Some(hint) = hint {
        ensure!(
            !hint.trim().is_empty(),
            "development hint must not be empty"
        );
    }
    for (source_name, source) in &mut config.sources {
        if name.is_some_and(|name| name != source_name) {
            continue;
        }
        let value = if let Some(hint) = hint {
            hint.to_owned()
        } else {
            let checkout = crate::run::active_checkout(project, source_name)?;
            let repo = Git::at(&checkout);
            warn_changes(source_name, &repo)?;
            (match reference.context("choose --ref or --hint")? {
                DevHintRef::Branch => repo.branch(),
                DevHintRef::Tag => repo.exact_tag(),
                DevHintRef::Commit => repo.head()?.context("checkout has no commit"),
            })
            .with_context(|| format!("{source_name}: cannot capture development hint"))?
        };
        source.dev_hint = Some(value);
    }
    // Load the persisted config above to retain literal upstream paths, which Project normalizes.
    config::write_json(&project.root.join(config::CONFIG), &config)?;
    println!("Updated development hints in {}", config::CONFIG);
    Ok(())
}

pub fn warn_changes(name: &str, repo: &Git) -> Result<bool> {
    let changes = repo.changes()?;
    for change in &changes {
        eprintln!("warning: {name}: {} {}", change.status, change.path);
    }
    Ok(!changes.is_empty())
}

pub fn fetch(project: &Project, names: &[String]) -> Result<()> {
    let pins = project.pins(false)?;
    let selected: BTreeSet<_> = if names.is_empty() {
        project.config.sources.keys().cloned().collect()
    } else {
        names.iter().cloned().collect()
    };
    for name in &selected {
        ensure!(
            project.config.sources.contains_key(name),
            "unknown source {name}"
        );
        let path = project.managed(name);
        if path.exists() {
            let repo = Git::at(&path);
            repo.validate(&project.config.sources[name].url, true)?;
            warn_changes(name, &repo)?;
        }
    }
    for name in selected {
        let revision = ensure_revision(
            &project.managed(&name),
            &project.config.sources[&name].url,
            pins.sources
                .get(&name)
                .and_then(|entry| entry.git.as_deref()),
            Acquisition::Refresh,
            &project.internal(),
        )
        .with_context(|| {
            format!("{name}: fetch failed; rerun sources fetch {name} or sources release --clean")
        })?;
        println!("{name}: cached {}", revision.commit);
    }
    Ok(())
}

pub fn check_release(project: &Project) -> Result<()> {
    let pins = project.pins(true)?;
    for (name, source) in &project.config.sources {
        let path = project.managed(name);
        if path.exists() {
            let repo = Git::at(&path);
            repo.validate(&source.url, true)?;
            ensure!(
                !warn_changes(name, &repo)?,
                "{name}: managed checkout has source changes; preserve them elsewhere or run sources release --clean"
            );
        }
    }
    for (name, source) in &project.config.sources {
        let revision = ensure_revision(
            &project.managed(name),
            &source.url,
            pins.sources[name].git.as_deref(),
            Acquisition::EnsureCached,
            &project.internal(),
        )
        .with_context(|| {
            format!("{name}: cannot cache release inputs; rerun sources check-release")
        })?;
        println!("{name}: release inputs cached at {}", revision.commit);
    }
    Ok(())
}

pub fn lock(project: &Project, updates: &[String], check: bool) -> Result<()> {
    let mut pins = project.pins(false)?;
    let mut proposed = Vec::new();
    if updates.is_empty() {
        for (name, source) in &project.config.sources {
            let repo = Git::at(&project.dev(name)?);
            repo.validate(&source.url, false)
                .with_context(|| format!("{name}: invalid development checkout"))?;
            if warn_changes(name, &repo)? {
                eprintln!(
                    "warning: {name}: uncommitted changes are excluded from the locked commit"
                );
            }
            let commit = repo
                .head()?
                .with_context(|| format!("{name}: development checkout has no commit"))?;
            proposed.push((name.clone(), commit));
        }
    } else {
        let mut seen = BTreeSet::new();
        for update in updates {
            let (name, input) = update
                .rsplit_once(':')
                .context("pin updates must use <name>:<hash> or <name>:tag=<tag>")?;
            let source = project
                .config
                .sources
                .get(name)
                .with_context(|| format!("unknown source {name}"))?;
            ensure!(seen.insert(name), "duplicate pin update for {name}");
            if let Some(tag) = input.strip_prefix("tag=") {
                if check {
                    let commit = fetch_tag_commit(&project.managed(name), &source.url, tag)
                        .with_context(|| {
                            format!("{name}: tag pin validation failed; lockfile unchanged")
                        })?;
                    warn_changes(name, &Git::at(&project.managed(name)))?;
                    proposed.push((name.to_owned(), commit));
                    continue;
                }
                let mut commit = None;
                let mut errors = Vec::new();
                let mut candidates = vec![(project.managed(name), true)];
                if let Ok(dev) = project.dev(name) {
                    candidates.push((dev, false));
                }
                for (path, managed) in candidates {
                    if !path.exists() {
                        continue;
                    }
                    let repo = Git::at(&path);
                    if let Err(error) = repo.validate(&source.url, managed) {
                        errors.push(format!("{error:#}"));
                        continue;
                    }
                    if let Some(candidate) = repo.resolve_tag(tag)? {
                        if let Some(previous) = &commit {
                            ensure!(
                                previous == &candidate,
                                "{name}: ambiguous local tag {tag:?}; use --check to select the upstream tag"
                            );
                        }
                        if warn_changes(name, &repo)? {
                            eprintln!(
                                "warning: {name}: uncommitted changes are excluded from the locked commit"
                            );
                        }
                        commit = Some(candidate);
                    }
                }
                proposed.push((name.to_owned(), commit.with_context(|| format!(
                    "{name}: cannot resolve tag {tag:?} locally; use --check to fetch the exact upstream tag\n{}", errors.join("\n")
                ))?));
                continue;
            }
            ensure!(
                (4..=64).contains(&input.len()) && input.bytes().all(|c| c.is_ascii_hexdigit()),
                "{name}: expected a hexadecimal commit hash"
            );
            let mut resolved: Option<(String, std::path::PathBuf)> = None;
            let mut candidates = vec![(project.managed(name), true)];
            if let Ok(dev) = project.dev(name) {
                candidates.push((dev, false));
            }
            let mut errors = Vec::new();
            for (path, managed) in candidates {
                if !path.exists() {
                    continue;
                }
                let repo = Git::at(&path);
                if let Err(error) = repo.validate(&source.url, managed) {
                    errors.push(format!("{error:#}"));
                    continue;
                }
                match repo.resolve_object(input) {
                    Ok(object) => {
                        if warn_changes(name, &repo)? {
                            eprintln!(
                                "warning: {name}: uncommitted changes are excluded from the locked commit"
                            );
                        }
                        if let Some((previous, _)) = &resolved {
                            ensure!(
                                previous == &object,
                                "{name}: ambiguous abbreviated commit {input} resolves differently in local repositories; supply a full hash"
                            );
                        }
                        if resolved.is_none() {
                            resolved = Some((object, path));
                        }
                    }
                    Err(error) => {
                        if error
                            .downcast_ref::<sources::git::AmbiguousCommitId>()
                            .is_some()
                        {
                            return Err(error).with_context(|| {
                                format!("{name}: pin is ambiguous; lockfile unchanged")
                            });
                        }
                        errors.push(format!("{error:#}"));
                    }
                }
            }
            let commit = match resolved {
                Some((object, path)) => Git::at(&path).resolve_commit(&object)?,
                None if check && [40, 64].contains(&input.len()) => input.to_ascii_lowercase(),
                None => bail!(
                    "{name}: cannot resolve {input} locally; supply a full commit hash with --check if it needs fetching\n{}",
                    errors.join("\n")
                ),
            };
            proposed.push((name.to_owned(), commit));
        }
    }
    for (name, commit) in proposed {
        let commit = if check {
            ensure_revision(
                &project.managed(&name),
                &project.config.sources[&name].url,
                Some(&commit),
                Acquisition::EnsureCached,
                &project.internal(),
            )
            .with_context(|| format!("{name}: pin validation failed; lockfile unchanged"))?
            .commit
        } else {
            commit
        };
        pins.sources.entry(name).or_default().git = Some(commit);
    }
    config::write_json(&project.root.join(LOCK), &pins)?;
    println!("Updated {LOCK}");
    Ok(())
}

/// Update package pins independently, optionally downloading releases before writing.
pub fn lock_registry(project: &Project, updates: &[String], check: bool) -> Result<()> {
    let mut pins = project.pins(false)?;
    let mut seen = BTreeSet::new();
    let mut packages = Vec::new();
    for update in updates {
        let (package, version) = update
            .rsplit_once(':')
            .context("registry pins must use <package>:<version>")?;
        ensure!(seen.insert(package), "duplicate package pin for {package}");
        let (name, _) = project
            .config
            .sources
            .iter()
            .find(|(_, source)| source.pnpm.contains_key(package))
            .with_context(|| format!("unknown managed package {package}"))?;
        ensure!(
            semver::Version::parse(version).is_ok(),
            "expected exact package version for {package}, such as 1.2.3"
        );
        pins.sources
            .entry(name.clone())
            .or_default()
            .pnpm
            .insert(package.into(), version.into());
        packages.push(format!("{package}@{version}"));
    }
    if check {
        crate::pnpm::download(project, &packages)?;
    }
    config::write_json(&project.root.join(LOCK), &pins)?;
    println!("Updated {LOCK}");
    Ok(())
}
