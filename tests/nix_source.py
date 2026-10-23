#!/usr/bin/env python3
"""Real-Nix source snapshot regressions; no builds or profile operations."""

import json
import shutil
import subprocess
import tempfile
import tomllib
import unittest
from pathlib import Path
from urllib.parse import parse_qs, urlsplit


ROOT = Path(__file__).resolve().parents[1]


def run(*command):
    try:
        result = subprocess.run(
            command, cwd=ROOT, capture_output=True, text=True, timeout=120
        )
    except subprocess.TimeoutExpired as error:
        raise AssertionError(
            f"Command timed out: {command!r}\n"
            f"stdout: {error.stdout!r}\nstderr: {error.stderr!r}"
        ) from error
    if result.returncode:
        raise AssertionError(
            f"Command failed ({result.returncode}): {command!r}\n"
            f"stdout:\n{result.stdout}\nstderr:\n{result.stderr}"
        )
    return result.stdout.strip()


def snapshot(script):
    path = Path(run("bash", str(script)))
    if not path.is_absolute() or not path.is_dir():
        raise AssertionError(f"nix-source did not return a snapshot directory: {path}")
    return path


def metadata(source):
    return json.loads(
        run("nix", "flake", "metadata", "--json", "--no-write-lock-file", str(source))
    )


class NixSourceTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        if shutil.which("nix") is None:
            raise AssertionError("These regression tests require an installed Nix.")
        cls.source = snapshot(ROOT / "scripts/nix-source")

    def test_locked_path_uri_round_trip(self):
        first = metadata(self.source)
        uri = first["url"]
        self.assertEqual(first["locked"]["type"], "path", first)
        self.assertEqual(urlsplit(uri).scheme, "path", uri)
        # Exercise the real parser before checking the field: Nix 2.34.8 can
        # emit a path URL with submodules=1 that it then refuses to parse.
        second = metadata(uri)
        self.assertNotIn("submodules", first["locked"], first)
        self.assertNotIn("submodules", parse_qs(urlsplit(uri).query), uri)
        self.assertEqual(second["locked"], first["locked"])

    def test_packages_use_locked_git_dependency_without_vendor_source(self):
        for package in ("bas", "basg"):
            with self.subTest(package=package):
                source = Path(run("nix", "eval", "--raw", f"{self.source}#{package}.src"))
                self.assertFalse((source / ".git").exists())
                self.assertFalse((source / "vendor").exists())
                manifest = tomllib.loads((source / "Cargo.toml").read_text())
                dependency = manifest["dependencies"]["gds-uim"]
                self.assertEqual(dependency["git"], "https://github.com/4t145/gds-uim.git")
                self.assertTrue(dependency["optional"])
                self.assertNotIn("path", dependency)
                revision = dependency["rev"]
                self.assertRegex(revision, r"^[0-9a-f]{40}$")
                lock = tomllib.loads((source / "Cargo.lock").read_text())
                macros = {
                    entry["name"]: entry for entry in lock["package"]
                    if entry["name"] in ("gds-uim", "gds-uim-macros")
                }
                self.assertEqual(set(macros), {"gds-uim", "gds-uim-macros"})
                for entry in macros.values():
                    self.assertEqual(
                        entry["source"],
                        f"git+{dependency['git']}?rev={revision}#{revision}",
                    )
                self.assertEqual(macros["gds-uim"]["dependencies"], ["gds-uim-macros"])
                # Force package evaluation, including the locked Git output hash.
                self.assertTrue(
                    run("nix", "eval", "--raw", f"{self.source}#{package}.drvPath")
                    .endswith(".drv")
                )

    def test_snapshot_filter(self):
        # Only synthetic placeholders are read here, never a user's .env.
        with tempfile.TemporaryDirectory(prefix="bas-nix-source-") as directory:
            root = Path(directory)
            script = root / "scripts/nix-source"
            script.parent.mkdir()
            shutil.copyfile(ROOT / "scripts/nix-source", script)
            kept = [
                "src/untracked.rs",
                "src/service/target/tests.rs",
                "acp-test-agent/Cargo.toml",
                "acp-test-agent/src/lib.rs",
                "acp-test-agent/src/target/tests.rs",
                ".envrc",
            ]
            excluded = [
                ".git/config",
                "acp-test-agent/.git",
                ".delta/worktrees/clone/src/lib.rs",
                ".direnv/cache",
                ".env",
                ".env.private",
                "acp-test-agent/.env.private",
                "target/output",
                "acp-test-agent/target/output",
                "node_modules/package/index.js",
                "result/output",
                "__pycache__/module.pyc",
            ]
            for relative in kept + excluded:
                path = root / relative
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(f"synthetic placeholder: {relative}\n")

            source = snapshot(script)
            for relative in kept:
                with self.subTest(kept=relative):
                    self.assertEqual(
                        (source / relative).read_text(),
                        f"synthetic placeholder: {relative}\n",
                    )
            for relative in excluded:
                with self.subTest(excluded=relative):
                    self.assertFalse(
                        (source / relative).exists(),
                        f"Snapshot leaked excluded metadata/environment/output: {relative}",
                    )



if __name__ == "__main__":
    unittest.main(verbosity=2)
