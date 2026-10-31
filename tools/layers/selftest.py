#!/usr/bin/env python3
"""Self-test for tools/layers/check.py using synthetic cargo metadata.

Builds 'cargo metadata --format-version 1 --no-deps' style JSON in a temporary
directory and runs check.py against the real layers.toml through --metadata:

  1. the documented layering with only downward edges -> exit 0
  2. an upward cross-layer dependency                  -> exit 1
  3. two domain crates depending on each other         -> exit 1
  4. a dependency in moly-law (normal and dev)         -> exit 1

Usage:
    python tools/layers/selftest.py

Exit codes: 0 when every case behaves as expected, 1 otherwise.
"""

import json
import os
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
CHECK = os.path.join(HERE, "check.py")
LAYERS = os.path.join(HERE, "layers.toml")

# crate -> Cargo.toml directory, relative to the synthetic workspace root
CRATES = {
    "moly-law": "crates/base/moly-law",
    "moly-store": "crates/base/moly-store",
    "moly-plugin": "crates/base/moly-plugin",
    "moly-text": "crates/base/moly-text",
    "moly-assets": "crates/engine/moly-assets",
    "moly-perf": "crates/engine/moly-perf",
    "moly-protocol": "crates/net/moly-protocol",
    "moly-layout": "crates/net/moly-layout",
    "moly-link": "crates/net/moly-link",
    "moly-server": "crates/server/moly-server",
    "moly-core": "crates/game/moly-core",
    "moly-render": "crates/game/moly-render",
    "moly-audio": "crates/game/moly-audio",
    "moly-ui": "crates/game/moly-ui",
    "moly-masters": "crates/game/moly-masters",
    "moly-world": "crates/game/moly-world",
    "moly-features": "crates/game/moly-features",
    "moly-devtools": "crates/game/moly-devtools",
    "moly-game": "crates/game/moly-game",
    "moly-app": "crates/app/moly-app",
    "moly-stage": "crates/app/moly-stage",
    "moly-android": "crates/app/moly-android",
    "moly-launcher": "crates/app/moly-launcher",
    "moly-server-host": "crates/app/moly-server-host",
    "moly-convert": "crates/game/moly-convert",
    "moly-selftest-domain": "crates/game/moly-selftest-domain",
}

# Edges: (from, to) or (from, to, "dev"). All of these point to a lower layer.
COMPLIANT = [
    ("moly-assets", "moly-law"),
    ("moly-layout", "moly-law"),
    ("moly-layout", "moly-protocol"),
    ("moly-link", "moly-protocol"),
    ("moly-masters", "moly-assets"),
    ("moly-masters", "moly-law"),
    ("moly-masters", "moly-protocol"),
    ("moly-core", "moly-assets"),
    ("moly-core", "moly-law"),
    ("moly-core", "moly-store"),
    ("moly-render", "moly-core"),
    ("moly-render", "moly-layout"),
    ("moly-render", "moly-link"),
    ("moly-render", "moly-masters"),
    ("moly-world", "moly-render"),
    ("moly-world", "moly-text"),
    ("moly-world", "moly-ui"),
    ("moly-features", "moly-world"),
    ("moly-devtools", "moly-features"),
    ("moly-game", "moly-devtools"),
    ("moly-game", "moly-features"),
    ("moly-app", "moly-game"),
    ("moly-app", "moly-link"),
    ("moly-stage", "moly-world"),
    ("moly-android", "moly-app"),
    ("moly-launcher", "moly-store"),
    ("moly-server-host", "moly-server"),
    ("moly-server", "moly-protocol"),
    ("moly-server", "moly-masters"),
    # domain crates below the tool/assembly/app tiers
    ("moly-convert", "moly-store"),
    ("moly-convert", "moly-text"),
    ("moly-selftest-domain", "moly-core"),
    ("moly-devtools", "moly-convert", "dev"),
    # dev-dependency to a different layer is still checked
    ("moly-features", "moly-server", "dev"),
]


def metadata(extra_edges=()):
    root = "C:/synthetic/moly"
    edges = {}
    for src, dst, *kind in list(COMPLIANT) + list(extra_edges):
        edges.setdefault(src, []).append((dst, kind[0] if kind else None))

    packages = []
    for name, rel in CRATES.items():
        deps = []
        for dst, kind in edges.get(name, []):
            deps.append(
                {
                    "name": dst,
                    "source": None,
                    "req": "*",
                    "kind": kind,
                    "rename": None,
                    "optional": False,
                    "uses_default_features": True,
                    "features": [],
                    "target": None,
                    "registry": None,
                    "path": f"{root}/{CRATES[dst]}",
                }
            )
        # A registry dependency that happens to share an external name.
        deps.append(
            {
                "name": "serde",
                "source": "registry+https://github.com/rust-lang/crates.io-index",
                "req": "^1",
                "kind": None,
                "rename": None,
                "optional": False,
                "uses_default_features": True,
                "features": [],
                "target": "cfg(target_arch = \"wasm32\")",
                "registry": None,
            }
        )
        packages.append(
            {
                "name": name,
                "version": "0.0.0",
                "id": f"path+file:///{root}/{rel}#0.0.0",
                "dependencies": deps,
                "manifest_path": f"{root.replace('/', chr(92))}\\{rel.replace('/', chr(92))}\\Cargo.toml",
                "targets": [],
            }
        )
    return {
        "packages": packages,
        "workspace_members": [p["id"] for p in packages],
        "workspace_default_members": [p["id"] for p in packages],
        "workspace_root": root,
        "target_directory": f"{root}/target",
        "version": 1,
        "resolve": None,
        "metadata": None,
    }


CASES = [
    ("compliant layering", [], 0, None),
    (
        "upward cross-layer dependency",
        [("moly-store", "moly-core")],
        1,
        "moly-store -> moly-core: upward dependency 'base' -> 'game-core'",
    ),
    (
        "domain crates depend on each other",
        [("moly-convert", "moly-selftest-domain")],
        1,
        "moly-convert -> moly-selftest-domain: domain crates must not depend on each other",
    ),
    (
        "moly-law has a normal dependency",
        [("moly-law", "moly-plugin")],
        1,
        "moly-law -> moly-plugin: [dependencies] must be empty",
    ),
    (
        "moly-law has a dev-dependency",
        [("moly-law", "moly-store", "dev")],
        1,
        "moly-law -> moly-store: [dev-dependencies] on a dependency-free crate",
    ),
]


def layers_copy(tmpdir):
    """The real rules with one synthetic crate pinned to the domain layer, so
    that two domain crates exist for case 3."""
    with open(LAYERS, encoding="utf-8") as fh:
        text = fh.read()
    marker = "[crates]\n"
    assert text.count(marker) == 1, "layers.toml must hold one [crates] table"
    text = text.replace(marker, marker + '"moly-selftest-domain" = "domain"\n')
    path = os.path.join(tmpdir, "layers.toml")
    with open(path, "w", encoding="utf-8", newline="\n") as fh:
        fh.write(text)
    return path


def run_case(tmpdir, index, name, extra, want_code, want_text):
    path = os.path.join(tmpdir, f"case{index}.json")
    with open(path, "w", encoding="utf-8", newline="\n") as fh:
        json.dump(metadata(extra), fh, indent=1)
    proc = subprocess.run(
        [sys.executable, CHECK, "--metadata", path, "--layers", layers_copy(tmpdir)],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        encoding="utf-8",
    )
    ok = proc.returncode == want_code
    if want_text is not None:
        ok = ok and want_text in proc.stdout
    status = "ok  " if ok else "FAIL"
    print(f"{status} {name}: exit {proc.returncode} (want {want_code})")
    if not ok:
        for line in (proc.stdout + proc.stderr).splitlines():
            print(f"     | {line}")
    return ok


def main():
    failures = 0
    with tempfile.TemporaryDirectory(prefix="moly-layers-selftest-") as tmpdir:
        for index, (name, extra, want_code, want_text) in enumerate(CASES, 1):
            if not run_case(tmpdir, index, name, extra, want_code, want_text):
                failures += 1
    if failures:
        print(f"{failures} case(s) failed")
        return 1
    print(f"all {len(CASES)} cases passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
