#!/usr/bin/env python3
"""Real-Nix source snapshot regressions; no builds or profile operations."""

import json
import shutil
import subprocess
import tempfile
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

    def test_initialized_vendor_source_is_preserved(self):
        vendor = self.source / "vendor/gds-uim"
        self.assertTrue(
            (vendor / "Cargo.toml").is_file(),
            "Initialize vendor/gds-uim before running: git submodule update --init --recursive",
        )
        self.assertTrue(
            any((vendor / "src").rglob("*.rs")),
            f"Snapshot is missing vendor Rust source: {vendor}",
        )
        self.assertFalse((self.source / ".git").exists())
        self.assertFalse((vendor / ".git").exists())

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
                "vendor/gds-uim/Cargo.toml",
                "vendor/gds-uim/src/lib.rs",
                "vendor/gds-uim/src/target/tests.rs",
                ".gitmodules",
                ".envrc",
            ]
            excluded = [
                ".git/config",
                "vendor/gds-uim/.git",
                ".delta/worktrees/clone/src/lib.rs",
                ".direnv/cache",
                ".env",
                ".env.private",
                "vendor/gds-uim/.env.private",
                "target/output",
                "acp-test-agent/target/output",
                "vendor/gds-uim/target/output",
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
