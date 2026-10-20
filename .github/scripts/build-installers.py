"""Generate installers from platform manifests and verify candidate checksums."""

import hashlib
import json
from pathlib import Path
import subprocess

artifacts = Path("target/distrib")
required = [
    "x86_64-unknown-linux-gnu",
    "x86_64-apple-darwin",
    "aarch64-apple-darwin",
]
command = ["dist", "build", "--artifacts=global", "--installer=shell", "--output-format=json"]
targets = list(required)
windows = artifacts / "sources-x86_64-pc-windows-msvc.zip"
if windows.is_file():
    targets.append("x86_64-pc-windows-msvc")
    command.append("--installer=powershell")
else:
    print("Windows archive unavailable; omitting Windows and its installer.", flush=True)
for target in targets:
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
