"""Install a checksum-pinned cargo-dist in a task-owned directory, never globally."""

import hashlib
import os
from pathlib import Path
import platform
import sys
import tarfile
import urllib.request
import zipfile

VERSION = "0.33.0"
# Official release asset digests from github.com/axodotdev/cargo-dist, v0.33.0.
ASSETS = {
    ("Linux", "x86_64"): (
        "x86_64-unknown-linux-gnu.tar.xz",
        "4b3f0a5f0ebbdb798f6db649d01b32ba1518376b6f7a0502b7d92b75cc2c8293",
    ),
    ("Darwin", "arm64"): (
        "aarch64-apple-darwin.tar.xz",
        "7b3cbe25511de01d74c0f5fcb7909edabd379bea9cfa284d93af5a3cdfa3247c",
    ),
    ("Windows", "AMD64"): (
        "x86_64-pc-windows-msvc.zip",
        "9a36d70795e14326a5ec4bf17aee085df00ab85a322739291ea5d4b1b5f693cd",
    ),
}

suffix, expected = ASSETS[(platform.system(), platform.machine())]
root = Path(sys.argv[1]).resolve()
root.mkdir(parents=True, exist_ok=True)
archive = root / f"cargo-dist-{suffix}"
url = f"https://github.com/axodotdev/cargo-dist/releases/download/v{VERSION}/{archive.name}"
with urllib.request.urlopen(url, timeout=120) as response:
    archive.write_bytes(response.read())
if hashlib.sha256(archive.read_bytes()).hexdigest() != expected:
    sys.exit(f"cargo-dist archive checksum mismatch: {archive}")
if archive.suffix == ".zip":
    with zipfile.ZipFile(archive) as package:
        package.extractall(root)
else:
    with tarfile.open(archive) as package:
        package.extractall(root, filter="data")
name = "dist.exe" if os.name == "nt" else "dist"
binaries = list(root.rglob(name))
if len(binaries) != 1:
    sys.exit(f"Expected exactly one {name}, found {binaries}")
binary = binaries[0]
print(binary)
if "GITHUB_PATH" in os.environ:
    with open(os.environ["GITHUB_PATH"], "a", encoding="utf-8") as output:
        output.write(str(binary.parent) + "\n")
