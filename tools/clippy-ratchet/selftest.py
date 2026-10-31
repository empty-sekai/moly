"""Self-test for check.py on synthetic clippy output; runs without cargo."""

from __future__ import annotations

import json
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import check  # noqa: E402


def message(package_id: str, code: str | None, level: str = "warning", line: int = 1, file: str = "src/lib.rs"):
    return json.dumps(
        {
            "reason": "compiler-message",
            "package_id": package_id,
            "target": {"kind": ["lib"], "name": "x"},
            "message": {
                "level": level,
                "message": f"{code} here",
                "code": {"code": code} if code else None,
                "spans": [{"file_name": file, "line_start": line, "column_start": 1, "is_primary": True}],
            },
        }
    )


PLAIN = "path+file:///w/crates/alpha#0.1.0"
NAMED = "path+file:///w/crates/dir#beta@0.1.0"


def run(lines: list[str], baseline: dict | None, *extra: str) -> tuple[int, dict | None]:
    with tempfile.TemporaryDirectory() as tmp:
        tmp = Path(tmp)
        source = tmp / "clippy.jsonl"
        source.write_text("\n".join(lines) + "\n", encoding="utf-8")
        target = tmp / "baseline.json"
        if baseline is not None:
            target.write_text(json.dumps(baseline), encoding="utf-8")
        code = check.main(["--input", str(source), "--baseline", str(target), *extra])
        written = json.loads(target.read_text(encoding="utf-8")) if target.exists() else None
        return code, written


def main() -> int:
    failures = 0

    def expect(name: str, ok: bool) -> None:
        nonlocal failures
        print(("pass " if ok else "FAIL ") + name)
        failures += not ok

    expect("plain package id", check.package_name(PLAIN) == "alpha")
    expect("named package id", check.package_name(NAMED) == "beta")

    lines = [
        message(PLAIN, "clippy::needless_borrow", line=1),
        message(PLAIN, "clippy::needless_borrow", line=2),
        message(PLAIN, "clippy::needless_borrow", line=2),  # same place twice
        message(NAMED, "dead_code"),
        message(NAMED, None),  # a note without a lint code
        "not json",
    ]
    counts, errors = check.count(lines)
    expect(
        "counts dedupe and group",
        errors == 0 and counts == {"alpha": {"clippy::needless_borrow": 2}, "beta": {"dead_code": 1}},
    )

    same = {"alpha": {"clippy::needless_borrow": 2}, "beta": {"dead_code": 1}}
    code, _ = run(lines, same)
    expect("equal counts pass", code == 0)

    code, _ = run(lines, {"alpha": {"clippy::needless_borrow": 1}, "beta": {"dead_code": 1}})
    expect("an increase fails", code == 1)

    code, _ = run(lines, {"alpha": {"clippy::needless_borrow": 2}})
    expect("a lint without a baseline entry fails", code == 1)

    code, written = run(lines, {"alpha": {"clippy::needless_borrow": 2}}, "--update")
    expect("update refuses on an increase", code == 1 and written == {"alpha": {"clippy::needless_borrow": 2}})

    higher = {"alpha": {"clippy::needless_borrow": 5, "unused_mut": 1}, "beta": {"dead_code": 1}}
    code, written = run(lines, higher)
    expect("a decrease passes without update", code == 0 and written == higher)
    code, written = run(lines, higher, "--update")
    expect("update lowers the baseline", code == 0 and written == same)

    code, written = run(lines, None, "--update")
    expect("update writes the first baseline", code == 0 and written == same)
    code, written = run(lines, None)
    expect("a missing baseline fails without update", code == 1 and written is None)

    code, _ = run(lines + [message(PLAIN, "E0425", level="error")], same)
    expect("compiler errors fail", code == 1)

    print(f"{failures} failed")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
