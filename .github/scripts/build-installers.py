"""Generate installers from platform manifests and verify candidate checksums."""

import hashlib
import json
from pathlib import Path
import subprocess

artifacts = Path("target/distrib")
required = [
    "x86_64-unknown-linux-gnu",
    "aarch64-apple-darwin",
    "x86_64-pc-windows-msvc",
]
command = ["dist", "build", "--artifacts=global", "--installer=shell",
           "--installer=powershell", "--output-format=json"]
for target in required:
    suffix = ".zip" if target.endswith("windows-msvc") else ".tar.gz"
    archive = artifacts / f"sources-{target}{suffix}"
    if not archive.is_file():
        raise SystemExit(f"Missing required archive: {archive}")
    manifest = artifacts / f"{target}-dist-manifest.json"
    if not manifest.is_file():
        raise SystemExit(f"Missing platform build manifest: {manifest}")
    command.extend(["--target", target])

# cargo-dist merges *-dist-manifest.json files before generating global artifacts.
# Save stdout only after the build, so it cannot read its own unfinished manifest.
result = subprocess.run(command, check=True, stdout=subprocess.PIPE)
json.loads(result.stdout)
(artifacts / "global-dist-manifest.json").write_bytes(result.stdout)

# cargo-dist 0.33.0 migrates legacy env scripts even for unmanaged/no-PATH
# installs. Such scripts may be sourced by custom profiles it cannot update.
# Keep generation fail-closed if the pinned upstream template changes.
installer = artifacts / "sources-installer.sh"
script = installer.read_text(encoding="utf-8")
legacy_migration = '''        if [ "$NO_MODIFY_PATH" = "0" ] ||
            is_dist_env_script_sh "$_install_dir_expr" "$_legacy_env_script_path" ||
            is_dist_env_script_fish "$_install_dir_expr" "$_legacy_fish_env_script_path"
'''
if script.count(legacy_migration) != 1:
    raise SystemExit("Unexpected cargo-dist legacy env migration guard; review no-PATH installer behavior")
script = script.replace(legacy_migration, '        if [ "$NO_MODIFY_PATH" = "0" ]\n')
installer.write_text(script, encoding="utf-8")

checksums = {}
for line in (artifacts / "sha256.sum").read_text(encoding="utf-8").splitlines():
    if line.strip():
        digest, filename = line.split(maxsplit=1)
        checksums[filename.removeprefix("*")] = digest
archives = sorted(artifacts.glob("sources-*.tar.gz")) + sorted(artifacts.glob("sources-*.zip"))
for archive in archives:
    actual = hashlib.sha256(archive.read_bytes()).hexdigest()
    if checksums.get(archive.name) != actual:
        raise SystemExit(f"Aggregate sha256.sum is missing or incorrect for {archive.name}")
print(f"Verified aggregate checksums for {len(archives)} archives.", flush=True)
