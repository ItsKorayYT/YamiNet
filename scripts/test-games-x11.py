#!/usr/bin/env python3
"""Exercise the real native game window using XTEST. Requires an existing X11 display,
xdotool and ImageMagick import. Uses isolated local-only browser/peer profiles.
Does not claim physical-keyboard, Windows or hardware display validation.
"""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time


def run(args):
    browser = args.browser.resolve()
    manifest = args.manifest.resolve()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    report = {"schema": "yami-games-x11-1", "input": "synthetic X11/XTEST", "checks": []}
    with tempfile.TemporaryDirectory(prefix="yami-games-x11-") as directory:
        profile = Path(directory)
        library = profile / "library"
        library.mkdir()
        (library / "library.tsv").write_text("YAMILIBRARY1\nhistory\t0\nhome\t\nsettings\t1\t1\t0\n")
        env = os.environ.copy()
        env.update(YAMINET_BROWSER_DATA_DIR=str(library), YAMINET_PEER_DIR=str(profile / "peers"),
                   YAMINET_BROWSER_DHT_NETWORK="local", YAMINET_BROWSER_RELAYS="0")
        with (output / "browser.log").open("w") as log:
            proc = subprocess.Popen([str(browser), "--game", str(manifest)], env=env, cwd=directory,
                                    stdout=log, stderr=log)
            def xdo(*values):
                return subprocess.check_output([str(args.xdotool), *map(str, values)], env=env,
                                               text=True, timeout=10).strip()
            def key(value):
                xdo("key", "--clearmodifiers", value)
            def title(expected):
                deadline = time.monotonic() + 10
                while time.monotonic() < deadline:
                    if proc.poll() is not None:
                        raise RuntimeError("browser exited during game validation")
                    if expected in xdo("getwindowname", window):
                        return
                    time.sleep(0.05)
                raise RuntimeError("title did not reach " + expected)
            def capture(name):
                path = output / (name + ".png")
                subprocess.run(["import", "-window", window, "-strip", str(path)],
                               env=env, check=True, timeout=10)
                return path.read_bytes()
            try:
                deadline = time.monotonic() + 15
                window = ""
                while time.monotonic() < deadline:
                    try:
                        window = xdo("search", "--onlyvisible", "--pid", proc.pid).splitlines()[0]
                        break
                    except (subprocess.CalledProcessError, IndexError):
                        time.sleep(0.05)
                if not window:
                    raise RuntimeError("native game window unavailable")
                xdo("windowfocus", "--sync", window)
                title("Prism Breakout")
                key("ctrl+r")  # deterministic restart/resume after focus during load
                time.sleep(0.5)
                capture("start-screen")
                key("space"); time.sleep(0.1)
                key("space"); time.sleep(0.3)
                xdo("keydown", "Left"); time.sleep(0.25); xdo("keyup", "Left")
                key("p"); time.sleep(0.4)
                first = capture("paused")
                time.sleep(0.25)
                assert first == capture("paused-stable"), "paused game changed"
                key("p"); time.sleep(0.4)
                assert first != capture("resumed"), "resume did not update window"
                report["checks"].append("CLI load, start, launch, paddle, pause freeze and resume")
                # Keep the game active long enough to sample actual presentation statistics.
                time.sleep(8)
                capture("gameplay")
                key("ctrl+t"); title("New tab")
                key("ctrl+l"); xdo("type", "--clearmodifiers", "local:about"); key("Return")
                title("About YamiNet")
                key("ctrl+w"); title("Prism Breakout")
                time.sleep(0.3)
                first = capture("background-paused")
                time.sleep(0.25)
                assert first == capture("background-stable"), "background game failed to pause"
                key("ctrl+w"); title("New tab")
                key("ctrl+l"); xdo("type", "local:welcome"); key("Return")
                title("Welcome to YamiNet")
                report["checks"].append("browser shortcuts, document browsing, background pause and close-tab isolation")
                # With one remaining tab, this is the sidebar's Open local game control.
                xdo("mousemove", "75", "245", "click", "1")
                xdo("type", "--clearmodifiers", "--delay", "1", str(manifest)); key("Return")
                title("Prism Breakout")
                time.sleep(0.5)
                key("space"); time.sleep(0.1); key("space")
                time.sleep(1)
                capture("native-game")
                xdo("mousemove", "1350", "35", "click", "1")  # Stop
                time.sleep(0.4)
                first = capture("stopped")
                time.sleep(0.25)
                assert first == capture("stopped-stable"), "stopped game changed"
                key("ctrl+r"); time.sleep(0.5)
                assert first != capture("restarted"), "stopped VM did not restart"
                xdo("windowsize", window, "900", "600"); time.sleep(0.5)
                capture("scaled-900x600")
                report["checks"].append("UI manifest launcher, stop, reload restart and viewport scaling")
                key("alt+F4")
                assert proc.wait(timeout=10) == 0, "unclean browser shutdown"
                report["checks"].append("clean native window shutdown with a game tab")
                report["result"] = "PASS"
            finally:
                if proc.poll() is None:
                    proc.terminate()
                    proc.wait(timeout=10)
    (output / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--browser", type=Path, required=True)
    parser.add_argument("--manifest", type=Path, default=Path("games/breakout/game.yami-game.json"))
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--xdotool", default=shutil.which("xdotool"))
    options = parser.parse_args()
    if not options.xdotool or not os.environ.get("DISPLAY"):
        parser.error("xdotool and DISPLAY are required (for example xvfb-run)")
    run(options)
