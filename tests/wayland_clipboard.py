#!/usr/bin/env python3
"""Check native clipboard MIME negotiation on an isolated headless Sway compositor."""

import argparse
import os
from pathlib import Path
import stat
import subprocess
import tempfile
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=Path("target/debug/basrw"))
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="basrw-wayland-") as temporary:
        root = Path(temporary)
        config = root / "sway.conf"
        config.write_text("xwayland disable\nseat seat0 fallback true\n", encoding="utf-8")
        env = os.environ.copy()
        for key in ("DISPLAY", "WAYLAND_DISPLAY", "SWAYSOCK", "NIRI_SOCKET"):
            env.pop(key, None)
        env.update(
            XDG_RUNTIME_DIR=str(root),
            WLR_BACKENDS="headless",
            WLR_RENDERER="pixman",
            WLR_LIBINPUT_NO_DEVICES="1",
            BASRW_TEST_ISOLATED_CLIPBOARD="1",
        )
        with (root / "sway.log").open("w+") as log:
            compositor = subprocess.Popen(
                ["sway", "--config", str(config)], env=env, stdout=log, stderr=log
            )
            try:
                deadline = time.monotonic() + 10
                socket = None
                while time.monotonic() < deadline and compositor.poll() is None:
                    socket = next(
                        (
                            path
                            for path in root.glob("wayland-*")
                            if stat.S_ISSOCK(path.stat().st_mode)
                        ),
                        None,
                    )
                    if socket:
                        break
                    time.sleep(0.05)
                if not socket:
                    log.seek(0)
                    raise RuntimeError("private compositor failed to start:\n" + log.read())
                env["WAYLAND_DISPLAY"] = socket.name
                subprocess.run(
                    [str(args.binary.resolve()), "--test-wayland-clipboard"],
                    env=env,
                    check=True,
                    timeout=20,
                )
            finally:
                compositor.terminate()
                try:
                    compositor.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    compositor.kill()
                    compositor.wait()


if __name__ == "__main__":
    main()
