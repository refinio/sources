"""Exercise release input gates through their command-line boundaries."""

from pathlib import Path
import os
import subprocess
import sys
import tempfile
import unittest

SCRIPT = Path(__file__).with_name("release-tag.py")


class ReleaseTagTests(unittest.TestCase):
    def run_tag(self, version, tag=None):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "Cargo.toml").write_text(f'[package]\nversion = "{version}"\n')
            output = root / "outputs"
            env = dict(os.environ, GITHUB_OUTPUT=str(output))
            env.pop("RELEASE_TAG", None)
            if tag is not None:
                env["RELEASE_TAG"] = tag
            result = subprocess.run([sys.executable, SCRIPT], cwd=root, env=env,
                                    capture_output=True, text=True)
            return result, output.read_text() if output.exists() else ""

    def test_candidate_derives_versioned_tag(self):
        result, output = self.run_tag("0.1.0")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("tag=v0.1.0\n", output)
        self.assertIn("version=0.1.0\n", output)

    def test_prerelease_tag_matches_exact_cargo_version(self):
        result, output = self.run_tag("0.2.0-rc.1", "v0.2.0-rc.1")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("tag=v0.2.0-rc.1\n", output)

    def test_rejects_mismatched_or_injected_tags_before_outputs(self):
        for tag in ["v9.9.9", "0.1.0", "v0.1.0\ntag=other", "$(touch unwanted)"]:
            with self.subTest(tag=tag):
                result, output = self.run_tag("0.1.0", tag)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("expected v0.1.0", result.stderr)
                self.assertEqual(output, "")


class RequiredArchivesTests(unittest.TestCase):
    def test_windows_archive_is_required_before_generating_installers(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            artifacts = root / "target/distrib"
            artifacts.mkdir(parents=True)
            for target in ["x86_64-unknown-linux-gnu", "aarch64-apple-darwin"]:
                (artifacts / f"sources-{target}.tar.gz").touch()
                (artifacts / f"{target}-dist-manifest.json").write_text("{}")
            result = subprocess.run([sys.executable, SCRIPT.with_name("build-installers.py")],
                                    cwd=root, capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            expected = Path("target/distrib/sources-x86_64-pc-windows-msvc.zip")
            self.assertIn(f"Missing required archive: {expected}",
                          result.stderr)


if __name__ == "__main__":
    unittest.main()
