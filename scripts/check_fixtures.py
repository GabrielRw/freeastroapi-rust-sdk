#!/usr/bin/env python3
"""Prove the generator copes with spec shapes the production spec does not have yet.

    python3 scripts/check_fixtures.py [SPEC ...]

For each fixture spec (default: tests/fixtures/*-spec.json) this builds a throwaway copy of the
crate under target/fixture-sdk/<name>/ with the hand-written sources, generates code and
operation tests from the fixture, and runs `cargo clippy -D warnings` and `cargo test` on it
(sharing ./target so dependencies compile once). The fixtures cover typed error responses
(`ErrorResponse` on 4xx/5xx), SVG, PNG, markdown, PDF and JSON-or-image responses, OpenAPI 3.1
type arrays, `const` discriminated unions, recursive schemas and Rust keyword names.
"""

from __future__ import annotations

import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
HAND_WRITTEN = ["lib.rs", "client.rs", "error.rs", "body.rs"]


def run(command: list[str], cwd: Path = ROOT) -> None:
    print("+", " ".join(command), flush=True)
    subprocess.run(command, cwd=cwd, check=True)


def check(spec: Path) -> None:
    name = spec.name.removesuffix(".json")
    crate = ROOT / "target" / "fixture-sdk" / name
    if crate.exists():
        shutil.rmtree(crate)
    (crate / "src").mkdir(parents=True)
    (crate / "spec").mkdir()
    manifest = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
    # A distinct package name keeps cargo's artefacts apart in the shared target directory;
    # the library keeps its name so the copied tests still `use freeastroapi::...`.
    package = "freeastroapi-fixture-" + name.replace("_", "-")
    manifest = manifest.replace('name = "freeastroapi"', f'name = "{package}"', 1)
    manifest = manifest.replace("[features]", '[lib]\nname = "freeastroapi"\n\n[features]', 1)
    (crate / "Cargo.toml").write_text(manifest + "\n[workspace]\n", encoding="utf-8")
    for file in HAND_WRITTEN:
        shutil.copy(ROOT / "src" / file, crate / "src" / file)
    shutil.copytree(ROOT / "tests" / "common", crate / "tests" / "common")
    shutil.copy(ROOT / "tests" / "coverage.rs", crate / "tests" / "coverage.rs")
    shutil.copy(spec, crate / "spec" / "freeastro.json")
    shutil.copy(ROOT / "rustfmt.toml", crate / "rustfmt.toml")
    run(
        [
            sys.executable,
            str(ROOT / "scripts" / "generate.py"),
            "--spec", str(spec),
            "--out-dir", str(crate / "src" / "generated"),
            "--tests-file", str(crate / "tests" / "operations.rs"),
            "--contract-file", str(crate / "tests" / "contract.rs"),
        ]
    )
    manifest_path = str(crate / "Cargo.toml")
    target = str(ROOT / "target")
    run(["cargo", "clippy", "--manifest-path", manifest_path, "--target-dir", target, "--lib", "--tests", "--", "-D", "warnings"])
    run(["cargo", "test", "--manifest-path", manifest_path, "--target-dir", target, "--tests"])


def main(argv: list[str]) -> int:
    specs = [Path(p).resolve() for p in argv] or sorted((ROOT / "tests" / "fixtures").glob("*-spec.json"))
    for spec in specs:
        check(spec)
    print(f"fixture specs OK: {', '.join(s.name for s in specs)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
