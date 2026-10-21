"""Exercise the executable from the actual cargo-dist archive on its native OS."""

import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import zipfile

archive = Path(sys.argv[1]).resolve()
checksum = Path(str(archive) + ".sha256").read_text(encoding="utf-8").split()[0]
if hashlib.sha256(archive.read_bytes()).hexdigest() != checksum:
    sys.exit(f"Package checksum mismatch: {archive}")
with tempfile.TemporaryDirectory(prefix="sources-package-smoke-") as directory:
    root = Path(directory)
    if archive.suffix == ".zip":
        with zipfile.ZipFile(archive) as package:
            package.extractall(root)
        name = "sources.exe"
    else:
        with tarfile.open(archive) as package:
            package.extractall(root, filter="data")
        name = "sources"
    licenses = list(root.rglob("LICENSE"))
    if len(licenses) != 1 or licenses[0].read_bytes() != Path("LICENSE").read_bytes():
        sys.exit("Package must contain exactly one copy of the repository LICENSE")
    binaries = [p for p in root.rglob(name) if p.is_file()]
    if len(binaries) != 1:
        sys.exit(f"Expected one packaged executable, found {binaries}")
    binary = binaries[0]
    subprocess.run([binary, "--version"], check=True)
    subprocess.run([binary, "--help"], check=True)
    project = root / "project"
    project.mkdir()
    subprocess.run([binary, "-C", project, "setup"], check=True)
    output = subprocess.check_output([binary, "-C", project, "status", "--json"])
    status = json.loads(output)
    if status["version"] != 2 or status["mode"] != "unconfigured":
        sys.exit(f"Unexpected fresh project status: {status}")
print(f"Packaged executable smoke passed: {archive.name}")
