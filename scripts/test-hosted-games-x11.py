#!/usr/bin/env python3
"""Publish on a separate node process, navigate a real yami:// address and click Play.
Requires X11/XTEST, xdotool and ImageMagick import. Uses isolated temporary profiles.
This is same-machine loopback validation, not independent-machine/internet evidence.
"""
import argparse
import json
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import tempfile
import time


def run(args):
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    repo = Path(__file__).resolve().parent.parent
    report = {"schema": "yami-hosted-games-x11-1", "network": "same-machine authenticated loopback",
              "input": "synthetic X11/XTEST", "checks": []}
    with tempfile.TemporaryDirectory(prefix="yami-hosted-demo-") as directory:
        root = Path(directory)
        site = root / "publisher" / "site"
        site.mkdir(parents=True)
        for name in ["home.ylang", "default.ysl"]:
            shutil.copyfile(repo / "sites/arcade" / name, site / name)
        subprocess.run([str(args.publisher.resolve()), str(repo / "games/breakout/game.yami-game.json"),
                        str(site), "prism"], check=True, capture_output=True, text=True)
        subprocess.run([str(args.node.resolve()), "site", "check", str(site)], check=True, capture_output=True, text=True)
        identity = root / "publisher" / "identity"
        result = subprocess.run([str(args.node.resolve()), "identity", "new", "--dir", str(identity)],
                                check=True, capture_output=True, text=True)
        node_id = re.search(r"yn1-[a-z2-7]{58}", result.stdout).group()
        player = root / "player"
        library = player / "library"
        library.mkdir(parents=True)
        (library / "library.tsv").write_text("YAMILIBRARY1\nhistory\t0\nhome\t\nsettings\t1\t1\t0\n")
        env = os.environ.copy()
        env["YAMINET_BROWSER_DATA_DIR"] = str(library)
        proc = None
        with (output / "node.log").open("w") as node_log, (output / "browser.log").open("w") as browser_log:
            host = subprocess.Popen([str(args.node.resolve()), "serve-dir", "127.0.0.1:0", str(site),
                                     "--identity", str(identity), "--tls", "--dht", "--dht-local",
                                     "--dht-dir", str(root / "publisher/state")], stdout=node_log, stderr=node_log)
            def wait(check, reason, timeout=15):
                until = time.monotonic() + timeout
                while time.monotonic() < until:
                    if host.poll() is not None or (proc is not None and proc.poll() is not None):
                        raise RuntimeError("process exited: " + reason)
                    if check():
                        return
                    time.sleep(0.05)
                raise RuntimeError("timed out: " + reason)
            def xdo(*values):
                return subprocess.check_output([str(args.xdotool), *map(str, values)], env=env,
                                               text=True, timeout=10).strip()
            def key(value):
                xdo("key", "--clearmodifiers", value)
            def capture(name):
                path = output / (name + ".png")
                subprocess.run(["import", "-window", window, "-strip", str(path)], env=env, check=True, timeout=10)
                return path.read_bytes()
            def title(value):
                wait(lambda: value in xdo("getwindowname", window), value)
            try:
                wait(lambda: "Open in Yami Browser:" in (output / "node.log").read_text(), "node startup")
                endpoint = re.search(r"on (127\.0\.0\.1:\d+) as", (output / "node.log").read_text()).group(1)
                url = f"yami://{node_id}/home"
                config = player / "browser.conf"
                config.write_text(f"yaminet-config 1\nrole browser\nnetwork loopback\nstate {player / 'state'}\nbootstrap {node_id}@{endpoint}\n")
                proc = subprocess.Popen([str(args.browser.resolve()), "--config", str(config), "--desktop"],
                                        cwd=player, env=env, stdout=browser_log, stderr=browser_log)
                windows = []
                def visible():
                    try:
                        windows[:] = xdo("search", "--onlyvisible", "--pid", proc.pid).splitlines()
                        return bool(windows)
                    except subprocess.CalledProcessError:
                        return False
                wait(visible, "native window")
                window = windows[0]
                xdo("windowfocus", "--sync", window)
                key("ctrl+l"); xdo("type", "--clearmodifiers", url); key("Return")
                title("YamiNet Arcade"); time.sleep(0.5)
                assert "Served bounded DHT RPC" in (output / "node.log").read_text()
                assert "Served game resource" not in (output / "node.log").read_text(), "page loading executed or fetched a game"
                capture("before-play")
                report.update(address=url, publisher_pid=host.pid, browser_pid=proc.pid)
                report["checks"].append("actual yami address, signed DHT resolution, inert page before consent")
                # Fixed geometry of the shipped 1440x900 demo page, at 1x Xvfb DPI.
                xdo("mousemove", "--window", window, args.play_x, args.play_y, "click", "1")
                title("Prism Breakout"); time.sleep(0.5)
                assert (output / "node.log").read_text().count("Served game resource prism") == 2
                capture("hosted-start")
                report["checks"].append("mouse Play fetches manifest and Wasm over pinned TLS into immersive game tab")
                key("space"); time.sleep(0.2); key("space"); time.sleep(0.5)
                xdo("keydown", "Right"); time.sleep(0.2); xdo("keyup", "Right")
                capture("hosted-playing")
                key("p"); time.sleep(0.5); first = capture("hosted-paused")
                time.sleep(0.3); assert first == capture("hosted-paused-stable")
                key("p"); time.sleep(0.6); assert first != capture("hosted-resumed")
                report["checks"].append("game controls, stable pause and resume")
                key("ctrl+t"); title("New tab")
                key("ctrl+l"); xdo("type", "local:about"); key("Return"); title("About YamiNet")
                key("ctrl+w"); title("Prism Breakout"); time.sleep(0.4)
                first = capture("background-paused"); time.sleep(0.3)
                assert first == capture("background-stable")
                key("ctrl+w"); title("YamiNet Arcade")
                assert (output / "node.log").read_text().count("Served game resource prism") == 2
                assert not any(p.suffix in [".wasm", ".json"] for p in player.rglob("*")), "player persisted executable files"
                report["checks"].append("responsive document tabs, background pause, preserved source page, no player executable files")
                key("alt+F4"); assert proc.wait(timeout=10) == 0
                host.send_signal(signal.SIGINT); assert host.wait(timeout=12) == 0
                report["checks"].append("clean browser and publisher process shutdown")
                report["result"] = "PASS"
            finally:
                for child in [proc, host]:
                    if child is not None and child.poll() is None:
                        child.terminate()
                        try:
                            child.wait(timeout=12)
                        except subprocess.TimeoutExpired:
                            child.kill(); child.wait(timeout=5)
                (output / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--browser", type=Path, required=True)
    parser.add_argument("--node", type=Path, required=True)
    parser.add_argument("--publisher", type=Path, required=True, help="built yami-games publish example")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--xdotool", default=shutil.which("xdotool"))
    parser.add_argument("--play-x", type=int, default=500)
    parser.add_argument("--play-y", type=int, default=156)
    options = parser.parse_args()
    if not options.xdotool or not os.environ.get("DISPLAY"):
        parser.error("xdotool and DISPLAY are required (for example xvfb-run)")
    run(options)
