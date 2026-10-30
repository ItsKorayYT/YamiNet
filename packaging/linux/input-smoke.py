#!/usr/bin/env python3
"""Synthetic X11 keyboard events through the actual winit event adapter; no physical-input claim."""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from verify import runtime_env


def smoke(browser, xdotool):
    report = {'schema': 'yaminet-x11-input-smoke-1', 'input': 'synthetic X11/XTEST; physical keyboard and IME NOT RUN', 'checks': []}
    with tempfile.TemporaryDirectory(prefix='Yami X11 input profile ') as tmp:
        env = runtime_env(); env['YAMINET_BROWSER_DATA_DIR'] = str(Path(tmp) / 'library')
        log = open(Path(tmp) / 'browser.log', 'w', encoding='utf-8')
        proc = subprocess.Popen([str(browser.resolve())], cwd=tmp, env=env, stdout=log, stderr=log)
        def xdo(*args):
            return subprocess.check_output([str(xdotool), *args], env=env, text=True, timeout=10).strip()
        try:
            deadline = time.monotonic() + 10
            window = ''
            while time.monotonic() < deadline:
                try: window = xdo('search', '--onlyvisible', '--pid', str(proc.pid)).splitlines()[0]; break
                except (subprocess.CalledProcessError, IndexError): time.sleep(0.05)
            if not window or proc.poll() is not None: raise RuntimeError('native window unavailable')
            xdo('windowfocus', '--sync', window)
            def key(value): xdo('key', '--clearmodifiers', value)
            def title(expected):
                deadline = time.monotonic() + 8
                while time.monotonic() < deadline:
                    if expected in xdo('getwindowname', window): return
                    if proc.poll() is not None: break
                    time.sleep(0.05)
                raise RuntimeError('native input did not produce expected navigation/title')
            def address(value):
                key('ctrl+l'); xdo('type', '--clearmodifiers', '--delay', '5', value); key('Return')
            key('Escape')
            address('local:welcome'); title('Welcome to YamiNet')
            address('local:about'); title('About YamiNet')
            report['checks'].append('address-selection-and-typing')
            key('alt+Left'); title('Welcome to YamiNet')
            key('alt+Right'); title('About YamiNet')
            report['checks'].append('back-forward')
            key('ctrl+t'); title('New tab')
            key('ctrl+w'); title('About YamiNet')
            report['checks'].append('new-close-tab')
            key('F5'); title('About YamiNet')
            key('ctrl+f'); xdo('type', '--delay', '5', 'YamiNet'); key('Escape')
            key('ctrl+l'); xdo('type', '--delay', '5', 'local:welcome'); key('Return'); title('Welcome to YamiNet')
            report['checks'].append('find-escape-address-focus')
            key('alt+F4')
            if proc.wait(timeout=10) != 0: raise RuntimeError('browser did not shut down cleanly')
            report['checks'].append('native-keyboard-clean-shutdown')
            report['result'] = 'PASS'
        finally:
            if proc.poll() is None: proc.terminate(); proc.wait(timeout=10)
            log.close()
    return report


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--browser', type=Path, required=True)
    p.add_argument('--xdotool', type=Path, default=shutil.which('xdotool'))
    p.add_argument('--report', type=Path, required=True)
    args = p.parse_args()
    try:
        if args.xdotool is None: raise RuntimeError('xdotool is required')
        result = smoke(args.browser, args.xdotool)
    except Exception as error:
        args.report.write_text(json.dumps({'schema': 'yaminet-x11-input-smoke-1', 'result': 'FAIL', 'error_class': type(error).__name__}), encoding='utf-8')
        raise
    args.report.write_text(json.dumps(result, indent=2), encoding='utf-8')
    print(json.dumps(result))
