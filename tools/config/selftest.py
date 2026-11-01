#!/usr/bin/env python3
"""Self-test of tools/config/check.py on a synthetic workspace."""

from __future__ import annotations

import pathlib
import sys
import tempfile

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import check  # noqa: E402

READS = '''
[[read]]
file = "crates/a/x/src/lib.rs"
variable = "MOLY_LISTED"
reason = "test corpus"
'''


def run(source: str, reads: str = READS) -> int:
    with tempfile.TemporaryDirectory() as tmp:
        root = pathlib.Path(tmp)
        src = root / "crates" / "a" / "x" / "src"
        src.mkdir(parents=True)
        (src / "lib.rs").write_text(source, encoding="utf-8")
        owner = root / "crates" / "base" / "moly-config" / "src"
        owner.mkdir(parents=True)
        (owner / "lib.rs").write_text('std::env::var("MOLY_OWNED")', encoding="utf-8")
        (root / "reads.toml").write_text(reads, encoding="utf-8")
        return check.main(["--root", str(root), "--reads", str(root / "reads.toml")])


def main() -> int:
    cases = [
        ("a listed read passes", 'let _ = std::env::var("MOLY_LISTED");', 0),
        ("an unlisted read fails", 'let _ = std::env::var("MOLY_LISTED"); std::env::var_os("MOLY_NEW");', 1),
        ("a build-time read fails", 'std::env::var("MOLY_LISTED"); option_env!("MOLY_BUILD");', 1),
        ("a stale entry fails", "fn nothing() {}", 1),
    ]
    failed = 0
    for name, source, expected in cases:
        got = run(source)
        status = "ok" if got == expected else "FAILED"
        failed += got != expected
        print(f"{status}: {name} (exit {got}, expected {expected})")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
