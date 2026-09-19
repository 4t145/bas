"""Local launcher checks; uses a placeholder key and never sends a model prompt."""

import json
import os
from pathlib import Path
import re
import signal
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "scripts"))
import local  # noqa: E402


class ConfigurationTests(unittest.TestCase):
    def test_dotenv_is_data_and_environment_takes_precedence(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            literal = "$(touch should-not-exist)${ANOTHER_VARIABLE}"
            (root / ".env").write_text(f"DEEPSEEK_API_KEY='{literal}'\n")
            with patch.object(local, "ROOT", root), patch.dict(os.environ, {}, clear=True):
                self.assertEqual(local.load_key(), literal)
                self.assertFalse((root / "should-not-exist").exists())
                with patch.dict(os.environ, {"DEEPSEEK_API_KEY": "placeholder-env-key"}):
                    self.assertEqual(local.load_key(), "placeholder-env-key")
                (root / ".env").write_text("DEEPSEEK_API_KEY=\n")
                with self.assertRaises(RuntimeError):
                    local.load_key()

    def test_pi_configuration_references_the_environment_without_copying_credentials(self):
        with tempfile.TemporaryDirectory() as directory:
            with patch.dict(os.environ, {"DEEPSEEK_API_KEY": "placeholder-secret-not-for-files"}):
                agent_dir = local.configure_pi(Path(directory))
            config = json.loads((agent_dir / "models.json").read_text())
            self.assertEqual(config["providers"]["deepseek"]["apiKey"], "DEEPSEEK_API_KEY")
            for file in agent_dir.iterdir():
                self.assertNotIn("placeholder-secret-not-for-files", file.read_text())
            self.assertFalse((agent_dir / "auth.json").exists())


def descendants(pid):
    # Tokio may launch a backend from any worker thread.
    children = set()
    for children_file in Path(f"/proc/{pid}/task").glob("*/children"):
        try:
            children.update(int(value) for value in children_file.read_text().split())
        except FileNotFoundError:
            pass
    return children | {child for parent in children for child in descendants(parent)}


class LifecycleTests(unittest.TestCase):
    def test_real_pi_initializes_without_a_prompt_and_shutdown_reaps_children(self):
        env = os.environ | {"DEEPSEEK_API_KEY": "placeholder-for-offline-startup-only"}
        with tempfile.TemporaryFile(mode="w+") as log:
            process = subprocess.Popen(
                [sys.executable, str(ROOT / "scripts/local.py"), "daemon", "--port", "0", "--no-build"],
                env=env, stdout=log, stderr=subprocess.STDOUT, start_new_session=True,
            )
            child_pids = set()
            try:
                deadline = time.monotonic() + 30
                while time.monotonic() < deadline:
                    log.seek(0)
                    output = log.read()
                    match = re.search(r"basrd ready: (127\.0\.0\.1:\d+)", output)
                    if match:
                        break
                    self.assertIsNone(process.poll(), output)
                    time.sleep(0.1)
                else:
                    self.fail("local daemon did not become ready")
                run_dir = Path(re.search(r"Logs and test files: (.+)", output)[1])
                server = match[1]
                result = local.cli(env, server, "session", "create", "--agent", "pi",
                                   "--cwd", str(run_dir / "work"), "--title", "offline-startup")
                self.assertRegex(result, r"status\s+idle")
                child_pids = descendants(process.pid)
                self.assertGreaterEqual(len(child_pids), 3, "expected basrd, pi-acp and pi")
                for file in (run_dir / "pi-agent").glob("*.json"):
                    self.assertNotIn(env["DEEPSEEK_API_KEY"], file.read_text())
            finally:
                process.send_signal(signal.SIGTERM)
                try:
                    process.wait(timeout=25)
                except subprocess.TimeoutExpired:
                    local.stop(process)
                    self.fail("launcher did not stop after SIGTERM")
            self.assertEqual(process.returncode, 130)
            self.assertFalse({pid for pid in child_pids if Path(f"/proc/{pid}").exists()},
                             "backend processes survived launcher shutdown")


if __name__ == "__main__":
    unittest.main()
