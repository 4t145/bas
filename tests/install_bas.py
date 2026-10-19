"""Offline installer boundary tests; never invokes real Nix or systemd."""

import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]


class InstallBasTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        scripts = self.root / "scripts"
        scripts.mkdir()
        for name in ("install-bas", "nix-profile-install"):
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
    print(os.environ.get("PACKAGE_OUTPUT", "/fixture/bas"))
elif sys.argv[1:3] == ["profile", "list"]:
    if os.environ.get("PROFILE_LIST_ERROR"):
        print(os.environ["PROFILE_LIST_ERROR"], file=sys.stderr)
        sys.exit(1)
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

    def run_installer(self, *arguments, script="install-bas"):
        return subprocess.run(
            ["bash", str(self.root / "scripts" / script), *arguments],
            env=self.env,
            capture_output=True,
            text=True,
            check=False,
        )

    def service_fixture(self):
        self.service_log = self.root / "systemctl.jsonl"
        self.env["SYSTEMCTL_LOG"] = str(self.service_log)
        self.package = self.root / "store" / "new-bas"
        self.env["PACKAGE_OUTPUT"] = str(self.package)
        self.service = self.package / "share/systemd/user/bas.service"
        self.service.parent.mkdir(parents=True)
        self.service.write_text("[Service]\nExecStart=/new/bas\n[Install]\nWantedBy=default.target\n")
        self.unit_directory = self.config / "systemd/user"
        self.unit_link = self.unit_directory / "bas.service"
        self.enabled_link = self.unit_directory / "default.target.wants/bas.service"
        self.executable(
            self.root / "bin/systemctl",
            f"""#!{shutil.which('python3')}
import json, os, sys
from pathlib import Path

arguments = sys.argv[1:]
with open(os.environ["SYSTEMCTL_LOG"], "a") as log:
    log.write(json.dumps(arguments) + "\\n")
positionals = [argument for argument in arguments if not argument.startswith("-")]
command = positionals[0]
units = Path(os.environ["XDG_CONFIG_HOME"]) / "systemd/user"

def create_link(destination, target):
    if os.path.lexists(destination):
        if destination.is_symlink() and os.readlink(destination) == str(target):
            return
        if not destination.is_symlink() or "--force" not in arguments:
            print(f"Failed to create symlink {{destination}}: File exists", file=sys.stderr)
            sys.exit(1)
        destination.unlink()
    destination.parent.mkdir(parents=True, exist_ok=True)
    destination.symlink_to(target)

if command == "link":
    target = Path(positionals[1])
    create_link(units / target.name, target)
elif command == "enable":
    target = (units / positionals[1]).resolve(strict=True)
    create_link(units / "default.target.wants" / positionals[1], target)
elif command not in ("daemon-reload", "restart"):
    sys.exit("unexpected systemctl command")
""",
        )

    def previous_service_links(self):
        old_service = self.root / "store/old-bas/share/systemd/user/bas.service"
        old_service.parent.mkdir(parents=True, exist_ok=True)
        old_service.write_text("[Service]\nExecStart=/old/bas\n")
        for link in (self.unit_link, self.enabled_link):
            link.parent.mkdir(parents=True, exist_ok=True)
            link.symlink_to(old_service)
        return old_service

    def test_service_update_replaces_old_store_links_and_preserves_user_configuration(self):
        self.service_fixture()
        old_service = self.previous_service_links()
        preserved = {
            self.config / "bas/settings.json": b'{"daemon":{"agents":{"custom":{}}}}\n',
            self.unit_directory / "bas.service.d/override.conf": b"[Service]\nEnvironment=USER_OPTION=yes\n",
            self.unit_directory / "other.service": b"[Service]\nExecStart=/other\n",
        }
        for path, content in preserved.items():
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(content)
        unrelated_link = self.unit_directory / "default.target.wants/other.service"
        unrelated_link.symlink_to(self.unit_directory / "other.service")
        unrelated_target = os.readlink(unrelated_link)

        result = self.run_installer("--service")

        self.assertEqual(result.returncode, 0, result.stderr)
        for link in (self.unit_link, self.enabled_link):
            self.assertTrue(link.is_symlink())
            self.assertEqual(link.resolve(), self.service)
        self.assertEqual(old_service.read_text(), "[Service]\nExecStart=/old/bas\n")
        for path, content in preserved.items():
            self.assertEqual(path.read_bytes(), content)
        self.assertEqual(os.readlink(unrelated_link), unrelated_target)
        commands = [json.loads(line) for line in self.service_log.read_text().splitlines()]
        self.assertEqual(
            [[argument for argument in call if not argument.startswith("-")][0] for call in commands],
            ["link", "daemon-reload", "enable", "restart"],
        )



    def test_unreadable_profile_stops_before_removing_or_adding_packages(self):
        self.env["PROFILE_LIST_ERROR"] = "profile contains an invalid path URL"
        result = self.run_installer()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(self.env["PROFILE_LIST_ERROR"], result.stderr)
        calls = [json.loads(line) for line in self.log.read_text().splitlines()]
        self.assertFalse(any(call[:2] in (
            ["profile", "remove"], ["profile", "add"],
        ) for call in calls))
        self.assertFalse(self.config.exists())

    def test_install_only_bas_and_preserve_user_files(self):
        for existing in (False, True):
            with self.subTest(existing=existing):
                if existing:
                    directory = self.config / "bas"
                    directory.mkdir(parents=True)
                    for name in ("settings.json", "agents.json", "appearance.json"):
                        (directory / name).write_text(f"untouched {name}\n")
                before = {
                    path: path.read_bytes()
                    for path in self.home.rglob("*") if path.is_file()
                }
                result = self.run_installer("--profile", "/fixture/profile")
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(result.stderr, "")
                after = {
                    path: path.read_bytes()
                    for path in self.home.rglob("*") if path.is_file()
                }
                self.assertEqual(before, after)
                if not existing:
                    self.assertFalse(self.config.exists())
        calls = [json.loads(line) for line in self.log.read_text().splitlines()]
        self.assertTrue(any(call[:2] == ["profile", "add"] for call in calls))
        for call in calls:
            self.assertNotIn("codex", " ".join(call))
            if call[:2] == ["profile", "add"]:
                self.assertEqual(
                    call, ["profile", "add", "fixture-source#bas",
                           "--profile", "/fixture/profile"],
                )
            if call[:2] == ["profile", "remove"]:
                self.assertEqual(call[-1], "bas")




if __name__ == "__main__":
    unittest.main()
