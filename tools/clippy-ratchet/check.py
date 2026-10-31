"""Hold the workspace lint warning counts at or below a recorded baseline.

The check runs `cargo clippy` over the workspace (or reads its JSON output from
a file), counts the warnings per package and lint, and compares the counts with
`baseline.json` next to this script. A count above its baseline entry, or a lint
that has no entry, fails the check. Counts below the baseline are reported so
that the baseline can be lowered with `--update`.

Usage:
    python tools/clippy-ratchet/check.py [--input FILE] [--update]

`--input FILE` reads `cargo clippy --message-format=json` lines from FILE
instead of running cargo. The command used otherwise is in CLIPPY_ARGS; the
`MOLY_CARGO` environment variable names an alternative cargo command (split on
spaces). `--update` rewrites the baseline with the current counts and refuses
when any count went up; without a baseline file it writes the first one.
"""

from __future__ import annotations

import argparse
import collections
import json
import os
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
BASELINE = HERE / "baseline.json"
CLIPPY_ARGS = [
    "clippy",
    "--workspace",
    "--locked",
    "--keep-going",
    "--message-format=json",
    "--",
    "--cap-lints",
    "warn",
]

Counts = dict[str, dict[str, int]]


def package_name(package_id: str) -> str:
    """Name of a package from a cargo package id.

    Path ids look like `path+file:///dir/name#0.1.0` when the package name
    equals the directory name, and `path+file:///dir/other#name@0.1.0` when it
    does not.
    """
    fragment = package_id.rsplit("#", 1)[-1]
    if "@" in fragment:
        return fragment.split("@", 1)[0]
    return package_id.split("#", 1)[0].rstrip("/").rsplit("/", 1)[-1]


def count(lines) -> tuple[Counts, int]:
    """Warnings per package and lint, and the number of errors.

    A diagnostic reported twice at the same place (for example once per target
    that compiles the same file) counts once.
    """
    seen: set[tuple] = set()
    counts: Counts = collections.defaultdict(lambda: collections.defaultdict(int))
    errors = 0
    for line in lines:
        line = line.strip()
        if not line.startswith("{"):
            continue
        try:
            record = json.loads(line)
        except json.JSONDecodeError:
            continue
        if record.get("reason") != "compiler-message":
            continue
        message = record["message"]
        level = message.get("level")
        if level in ("error", "error: internal compiler error"):
            errors += 1
            continue
        if level != "warning":
            continue
        code = (message.get("code") or {}).get("code")
        if not code:
            continue
        spans = [s for s in message.get("spans", []) if s.get("is_primary")]
        place = tuple((s["file_name"], s["line_start"], s["column_start"]) for s in spans)
        package = package_name(record["package_id"])
        key = (package, code, place, message.get("message"))
        if key in seen:
            continue
        seen.add(key)
        counts[package][code] += 1
    return {p: dict(sorted(c.items())) for p, c in sorted(counts.items())}, errors


def compare(baseline: Counts, current: Counts) -> tuple[list[str], list[str]]:
    """Lines describing increases and decreases against the baseline."""
    up, down = [], []
    for package in sorted(set(baseline) | set(current)):
        before = baseline.get(package, {})
        after = current.get(package, {})
        for lint in sorted(set(before) | set(after)):
            b, a = before.get(lint, 0), after.get(lint, 0)
            if a > b:
                up.append(f"{package} {lint}: {b} -> {a}")
            elif a < b:
                down.append(f"{package} {lint}: {b} -> {a}")
    return up, down


def run_clippy() -> list[str]:
    cargo = os.environ.get("MOLY_CARGO", "cargo").split()
    done = subprocess.run(
        cargo + CLIPPY_ARGS,
        cwd=ROOT,
        stdout=subprocess.PIPE,
        text=True,
        encoding="utf-8",
        errors="replace",
    )
    return done.stdout.splitlines()


def total(counts: Counts) -> int:
    return sum(sum(c.values()) for c in counts.values())


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--input", type=Path, help="read clippy JSON lines from this file")
    parser.add_argument("--update", action="store_true", help="rewrite the baseline")
    parser.add_argument("--baseline", type=Path, default=BASELINE, help=argparse.SUPPRESS)
    args = parser.parse_args(argv)

    if args.input:
        lines = args.input.read_text(encoding="utf-8", errors="replace").splitlines()
    else:
        lines = run_clippy()
    current, errors = count(lines)
    if errors:
        print(f"clippy reported {errors} errors; the counts are not comparable", file=sys.stderr)
        return 1
    if not current:
        print("no warnings were read; check that clippy ran", file=sys.stderr)

    baseline: Counts = {}
    if args.baseline.exists():
        baseline = json.loads(args.baseline.read_text(encoding="utf-8"))
    up, down = compare(baseline, current)

    for line in up:
        print(f"increased  {line}")
    for line in down:
        print(f"decreased  {line}")
    print(f"{total(current)} warnings, baseline {total(baseline)}")

    if up and not (args.update and not args.baseline.exists()):
        print("lint warnings went up; fix the new warnings", file=sys.stderr)
        return 1
    if args.update:
        args.baseline.write_text(json.dumps(current, indent=2) + "\n", encoding="utf-8", newline="\n")
        print(f"wrote {args.baseline.name}")
    elif down:
        print("counts went down; run with --update to lower the baseline")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
