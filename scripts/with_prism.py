#!/usr/bin/env python3
"""Run a command against a Stoplight Prism mock of the vendored spec.

    python3 scripts/with_prism.py [--spec spec/freeastro.json] [--port 4010] -- COMMAND...

Starts `npx @stoplight/prism-cli@PRISM_VERSION mock SPEC` (Node.js required), waits until it
listens, runs COMMAND with PRISM_URL set, then stops Prism and exits with COMMAND's status.
Prism's log goes to prism.log (or --log) for debugging request validation failures.
Standard library only and identical in every FreeAstroAPI SDK repository.
"""

from __future__ import annotations

import argparse
import os
import shutil
import signal
import subprocess
import time
import urllib.error
import urllib.request

PRISM_VERSION = "5.16.0"


def wait_until_listening(url: str, process: subprocess.Popen[bytes], timeout: float) -> None:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise SystemExit(f"Prism exited early with status {process.returncode}; see the Prism log")
        try:
            urllib.request.urlopen(url, timeout=2)
            return
        except urllib.error.HTTPError:
            return  # Any HTTP answer (401 without an API key) means Prism is up.
        except OSError:
            time.sleep(0.5)
    raise SystemExit(f"Prism did not start listening on {url} within {timeout:.0f}s")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--spec", default="spec/freeastro.json")
    parser.add_argument("--port", type=int, default=4010)
    parser.add_argument("--log", default="prism.log")
    parser.add_argument("--timeout", type=float, default=180.0)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command[1:] if args.command[:1] == ["--"] else args.command
    if not command:
        parser.error("missing COMMAND")
    npx = shutil.which("npx")
    if not npx:
        raise SystemExit("npx (Node.js) is required to run Prism")

    url = f"http://127.0.0.1:{args.port}"
    prism_cmd = [
        npx,
        "--yes",
        f"@stoplight/prism-cli@{PRISM_VERSION}",
        "mock",
        args.spec,
        "--host",
        "127.0.0.1",
        "--port",
        str(args.port),
    ]
    with open(args.log, "wb") as log:
        prism = subprocess.Popen(prism_cmd, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
        try:
            wait_until_listening(url + "/", prism, args.timeout)
            print(f"Prism {PRISM_VERSION} is mocking {args.spec} on {url}", flush=True)
            return subprocess.call(command, env={**os.environ, "PRISM_URL": url})
        finally:
            try:
                os.killpg(prism.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
            try:
                prism.wait(timeout=10)
            except subprocess.TimeoutExpired:
                os.killpg(prism.pid, signal.SIGKILL)


if __name__ == "__main__":
    raise SystemExit(main())
