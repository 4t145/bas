"""Developer installer boundaries, without a real build or profile modification."""

import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]


class InstallBasgDevTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        scripts = self.root / "scripts"
        scripts.mkdir()
        shutil.copyfile(ROOT / "scripts/install-basg-dev", scripts / "install-basg-dev")
        self.executable(scripts / "nix-source", "#!/bin/sh\nprintf '/fixture/source\\n'\n")
        self.binary = self.root / "custom target/debug/basg"
        self.executable(self.binary, "#!/bin/sh\nexit 0\n")
        self.package = self.root / "store/basg-dev"
        self.executable(self.package / "bin/basg-dev", "#!/bin/sh\nexit 0\n")
        self.log = self.root / "calls.jsonl"
        self.executable(self.root / "bin/nix", f"""#!{shutil.which('python3')}
import json, os, sys
args = sys.argv[1:]
with open(os.environ["CALL_LOG"], "a") as log:
    log.write(json.dumps(args) + "\\n")
stage = args[1] if args[0] == "profile" else args[0]
if os.environ.get("FAIL_STAGE") == stage:
    sys.exit("fixture failure: " + stage)
if args[:2] == ["profile", "list"]:
    print('{{"elements":{{}}}}')
elif args[0] == "develop":
    print(json.dumps({{
        "reason": "compiler-artifact", "target": {{"name": "basg", "kind": ["bin"]}},
        "executable": os.environ["BUILT_BINARY"],
    }}))
elif args[0] == "build":
    assert os.environ["BAS_DEV_BINARY"] == os.environ["BUILT_BINARY"]
    assert os.environ["BAS_DEV_SOURCE"] == "/fixture/source"
    print(os.environ["PACKAGE_OUTPUT"])
""")
        self.executable(
            self.root / "bin/systemctl",
            "#!/bin/sh\necho 'must not restart the backend' >&2\nexit 99\n",
        )
        self.executable(self.root / "bin/uname", "#!/bin/sh\necho Linux\n")
        self.env = {
            **os.environ,
            "HOME": str(self.root / "home"),
            "PATH": f"{self.root / 'bin'}:{os.environ['PATH']}",
            "CALL_LOG": str(self.log),
            "BUILT_BINARY": str(self.binary),
            "PACKAGE_OUTPUT": str(self.package),
        }

    def executable(self, path, text):
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)
        path.chmod(0o755)

    def run_installer(self, *arguments):
        return subprocess.run(
            ["bash", str(self.root / "scripts/install-basg-dev"), *arguments],
            env=self.env, capture_output=True, text=True, check=False,
        )

    def calls(self):
        return [json.loads(line) for line in self.log.read_text().splitlines()]

    def test_uses_cargo_artifact_and_only_installs_the_developer_entry(self):
        options = ["--offline", "--profile", str(self.root / "profile with spaces")]
        result = self.run_installer(*options)
        self.assertEqual(result.returncode, 0, result.stderr)
        calls = self.calls()
        build = next(call for call in calls if call[0] == "develop")
        self.assertEqual(build[build.index("--command") + 1:], [
            "env", "CARGO_INCREMENTAL=1", "cargo", "build", "--locked",
            "--features", "gui", "--bin", "basg",
            "--message-format=json-render-diagnostics",
        ])
        self.assertNotIn("--release", build)
        self.assertNotIn("--target-dir", build)
        self.assertEqual([call for call in calls if call[:2] == ["profile", "remove"]],
                         [["profile", "remove", *options, "basg-dev"]])
        self.assertEqual([call for call in calls if call[:2] == ["profile", "add"]],
                         [["profile", "add", *options, str(self.package)]])
        self.assertIn("release basg is unchanged", result.stdout)

    def test_failure_never_removes_the_previous_installation(self):
        for stage in ["list", "develop", "build"]:
            with self.subTest(stage=stage):
                self.env["FAIL_STAGE"] = stage
                if self.log.exists():
                    self.log.unlink()
                result = self.run_installer()
                self.assertNotEqual(result.returncode, 0)
                self.assertFalse(any(call[:2] in (
                    ["profile", "remove"], ["profile", "add"],
                ) for call in self.calls()))





if __name__ == "__main__":
    unittest.main()
