"""Runtime-only transfer of pi credentials; never interpolate them into commands."""

import json
import os
import tempfile

from dotenv import dotenv_values


def configure_pi() -> bool:
    source = os.environ.get("BASR_TEST_ENV_FILE")
    values = dotenv_values(source, interpolate=False) if source else {}
    key = os.environ.get("DEEPSEEK_API_KEY") or values.get("DEEPSEEK_API_KEY")
    if not key:
        return False
    target = "/home/basr/.pi/agent/auth.json"
    # NamedTemporaryFile is private (0600) and removed even if transfer fails.
    with tempfile.NamedTemporaryFile(mode="w+", suffix=".json", encoding="utf-8") as auth:
        json.dump({"deepseek": {"type": "api_key", "key": key}}, auth)
        auth.flush()
        machine.copy_from_host(auth.name, target)
    machine.succeed(f"chown basr:basr {target}; chmod 600 {target}")
    print("Pi credentials copied into the guest for user basr.")
    return True
