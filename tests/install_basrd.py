"""Offline installer boundary tests; never invokes real Nix or systemd."""

import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]


class InstallBasrdTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        scripts = self.root / "scripts"
        scripts.mkdir()
        for name in ("install-basrd", "nix-profile-install"):
            shutil.copy2(ROOT / "scripts" / name, scripts / name)
        self.executable(scripts / "nix-source", "#!/bin/sh\nprintf 'fixture-source\\n'\n")
        binaries = self.root / "bin"
        binaries.mkdir()
        self.log = self.root / "calls.jsonl"
        self.executable(
            binaries / "nix",
            f"""#!{shutil.which('python3')}
import json, os, sys
with open(os.environ["CALL_LOG"], "a") as log:
    log.write(json.dumps(sys.argv[1:]) + "\\n")
if sys.argv[1] == "eval":
    print("/fixture/basrd")
elif sys.argv[1:3] == ["profile", "list"]:
    print('{{"elements": {{"codex-acp": {{"storePaths": ["/old/codex"]}}}}}}')
""",
        )
        self.executable(
            binaries / "systemctl",
            "#!/bin/sh\necho 'unexpected systemctl invocation' >&2\nexit 99\n",
        )
        self.executable(binaries / "uname", "#!/bin/sh\necho Linux\n")
        self.home = self.root / "home"
        self.home.mkdir()
        self.config = self.home / ".config"
        self.env = {
            **os.environ,
            "HOME": str(self.home),
            "XDG_CONFIG_HOME": str(self.config),
            "PATH": f"{binaries}:{os.environ['PATH']}",
            "CALL_LOG": str(self.log),
        }

    def executable(self, path, text):
        path.write_text(text)
        path.chmod(0o755)

    def run_installer(self, *arguments):
        return subprocess.run(
            ["bash", str(self.root / "scripts/install-basrd"), *arguments],
            env=self.env,
            capture_output=True,
            text=True,
            check=False,
        )

    def test_removed_flag_fails_before_any_side_effect(self):
        for arguments in (
            ("--with-codex",),
            ("--service", "--with-codex"),
            ("--profile", "/unused/profile", "--with-codex"),
            ("--with-codex=true",),
        ):
            with self.subTest(arguments=arguments):
                result = self.run_installer(*arguments)
                self.assertEqual(result.returncode, 2, result.stderr)
                self.assertIn("docs/install.md", result.stderr)
                self.assertIn("independently", result.stderr)
                self.assertFalse(self.log.exists())
                self.assertFalse(self.config.exists())

    def test_install_only_basr_and_preserve_user_files(self):
        for existing in (False, True):
            with self.subTest(existing=existing):
                if existing:
                    directory = self.config / "basr"
                    directory.mkdir(parents=True)
                    for name in ("settings.json", "agents.json", "appearance.json"):
                        (directory / name).write_text(f"untouched {name}\n")
                before = {
                    path: path.read_bytes()
                    for path in self.home.rglob("*") if path.is_file()
                }
                result = self.run_installer("--profile", "/fixture/profile")
                self.assertEqual(result.returncode, 0, result.stderr)
                after = {
                    path: path.read_bytes()
                    for path in self.home.rglob("*") if path.is_file()
                }
                self.assertEqual(before, after)
                if not existing:
                    self.assertFalse(self.config.exists())
        calls = [json.loads(line) for line in self.log.read_text().splitlines()]
        self.assertTrue(any(call[:2] == ["profile", "install"] for call in calls))
        for call in calls:
            self.assertNotIn("codex", " ".join(call))
            if call[:2] == ["profile", "install"]:
                self.assertEqual(
                    call, ["profile", "install", "fixture-source#basrd",
                           "--profile", "/fixture/profile"],
                )
            if call[:2] == ["profile", "remove"]:
                self.assertEqual(call[-1], "basrd")

    def test_help_has_no_install_side_effect(self):
        result = self.run_installer("--help")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotIn("--with-codex", result.stdout)
        self.assertIn("independently", result.stdout)
        self.assertFalse(self.log.exists())
        self.assertFalse(self.config.exists())


if __name__ == "__main__":
    unittest.main()
