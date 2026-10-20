"""Require a manually selected release tag to match the Cargo package version."""

import os
from pathlib import Path
import tomllib

version = tomllib.loads(Path("Cargo.toml").read_text(encoding="utf-8"))["package"]["version"]
tag = f"v{version}"
requested = os.environ.get("RELEASE_TAG", "")
if requested and requested != tag:
    raise SystemExit(f"Release tag {requested!r} does not match Cargo version; expected {tag}")
print(tag)
if output := os.environ.get("GITHUB_OUTPUT"):
    with open(output, "a", encoding="utf-8") as stream:
        stream.write(f"tag={tag}\nversion={version}\n")
