#!/usr/bin/env python3
"""Check that program settings are read through moly-config.

Every setting a program reads is a key of the moly-config key table
(`crates/base/moly-config/src/lib.rs`), and its environment variable is read
there. This check scans the Rust sources under `crates/` for direct reads of
`MOLY_*` variables:

    std::env::var("MOLY_X")    std::env::var_os("MOLY_X")
    option_env!("MOLY_X")      env!("MOLY_X")

A read outside `crates/base/moly-config/` must be listed in `reads.toml` next
to this file, with the reason it is not a configuration key (a test corpus, a
build-time setting, a variable that decides where the configuration file is).
A listed read that no longer exists is reported too, so the list stays exact.

Usage:
    python tools/config/check.py [--root <workspace>] [--reads <toml>]

Exit codes:
    0  every direct read is listed and every listed read exists
    1  an unlisted read or a stale entry (one line each)
    2  bad usage or an invalid reads.toml
"""

from __future__ import annotations

import argparse
import pathlib
import re
import sys
import tomllib

READ = re.compile(r'(?:\benv::var(?:_os)?|\boption_env!|\benv!)\(\s*"(MOLY_[A-Z0-9_]+)"')
OWNER = "crates/base/moly-config/"


def scan(root: pathlib.Path) -> dict[tuple[str, str], int]:
    """Direct reads per (file, variable), outside the moly-config crate."""
    found: dict[tuple[str, str], int] = {}
    crates = root / "crates"
    if not crates.is_dir():
        raise SystemExit(f"error: {crates} is not a directory")
    for path in sorted(crates.rglob("*.rs")):
        name = path.relative_to(root).as_posix()
        if name.startswith(OWNER):
            continue
        for match in READ.finditer(path.read_text(encoding="utf-8")):
            key = (name, match.group(1))
            found[key] = found.get(key, 0) + 1
    return found


def load(path: pathlib.Path) -> dict[tuple[str, str], str]:
    try:
        raw = tomllib.loads(path.read_text(encoding="utf-8"))
    except (OSError, tomllib.TOMLDecodeError) as error:
        print(f"error: {path}: {error}", file=sys.stderr)
        raise SystemExit(2) from error
    listed: dict[tuple[str, str], str] = {}
    for index, entry in enumerate(raw.get("read", [])):
        file, variable, reason = entry.get("file"), entry.get("variable"), entry.get("reason")
        if not all(isinstance(value, str) and value for value in (file, variable, reason)):
            print(f"error: {path}: read #{index + 1} needs file, variable and reason", file=sys.stderr)
            raise SystemExit(2)
        if (file, variable) in listed:
            print(f"error: {path}: {file} {variable} is listed twice", file=sys.stderr)
            raise SystemExit(2)
        listed[(file, variable)] = reason
    return listed


def check(found: dict[tuple[str, str], int], listed: dict[tuple[str, str], str]) -> list[str]:
    problems = [
        f"{file}: reads {variable} directly; read it through a moly-config key, "
        f"or list it in tools/config/reads.toml with its reason"
        for (file, variable) in sorted(found)
        if (file, variable) not in listed
    ]
    problems += [
        f"{file}: tools/config/reads.toml lists {variable}, which the file no longer reads"
        for (file, variable) in sorted(listed)
        if (file, variable) not in found
    ]
    return problems


def main(argv: list[str] | None = None) -> int:
    here = pathlib.Path(__file__).resolve().parent
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--root", type=pathlib.Path, default=here.parent.parent)
    parser.add_argument("--reads", type=pathlib.Path, default=here / "reads.toml")
    args = parser.parse_args(argv)
    found = scan(args.root)
    problems = check(found, load(args.reads))
    for problem in problems:
        print(problem)
    if problems:
        return 1
    print(f"ok: {sum(found.values())} direct reads in {len({f for f, _ in found})} files, all listed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
