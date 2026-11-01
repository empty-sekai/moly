#!/usr/bin/env python3
"""Check that workspace crates only depend on lower (or allowed) layers.

Reads the cargo metadata JSON of the workspace (``cargo metadata --format-version 1
--no-deps``; nothing is compiled) and the rules in ``layers.toml`` next to this
file, then checks every workspace member's dependency edges against:

  * layer order (``[order]``): a dependency must point to a crate in a strictly
    lower layer, unless the pair is listed under ``[allow].edges``;
  * the ``domain`` layer: its crates must not depend on one another, except for
    the edges listed under ``[domain_exceptions].allowed``;
  * the server layer: ``[game_layers]`` must never be used by it;
  * dependency-free crates (``[dependency_free]``, e.g. ``moly-law``): no
    ``[dependencies]`` at all, dev-dependencies are reported too.

Every edge (normal, dev, build, optional and target-specific) is checked with
the same rule and labelled in the output. An optional or target-specific edge
may point upwards only when listed under ``[allow].game_edges``; a dev edge
only when listed under ``[allow].dev_edges``.

Usage:
    python tools/layers/check.py [--metadata <json>] [--layers <toml>]
                                 [--cargo <cargo command>] [-v]

The default cargo command is ``$MOLY_CARGO`` when that variable is set, else
``cargo``. ``--metadata`` skips running cargo altogether.

Exit codes:
    0  all rules hold (one summary line is printed)
    1  at least one rule violation (one ``from -> to: reason`` line each)
    2  bad usage, unreadable metadata, or invalid layers.toml
"""

from __future__ import annotations

import argparse
import json
import os
import shlex
import subprocess
import sys
import tomllib

HERE = os.path.dirname(os.path.abspath(__file__))
DEFAULT_LAYERS = os.path.join(HERE, "layers.toml")
DOMAIN = "domain"
NORMAL = "normal"


class ConfigError(Exception):
    pass


def warn(message):
    print(f"warning: {message}", file=sys.stderr)


# --------------------------------------------------------------------------
# configuration


def _table(raw, section, path):
    value = raw.get(section, {})
    if not isinstance(value, dict):
        raise ConfigError(f"[{section}] in {path} must be a table")
    return value


def _strings(table, key, section):
    items = table.get(key, [])
    if not isinstance(items, list) or not all(isinstance(i, str) for i in items):
        raise ConfigError(f"[{section}].{key} must be a list of strings")
    return items


def _edge_set(table, key, section):
    """Parse 'from -> to' entries into canonical single-spaced strings."""
    result = set()
    for item in _strings(table, key, section):
        if "->" not in item:
            raise ConfigError(f"[{section}].{key} entry {item!r} must look like 'a -> b'")
        result.add(" ".join(item.split()))
    return result


def load_config(path):
    with open(path, "rb") as fh:
        raw = tomllib.load(fh)

    layers = _table(raw, "layers", path)
    if DOMAIN not in layers:
        raise ConfigError("[layers] must define the 'domain' layer")
    for name, spec in layers.items():
        if not isinstance(spec, dict):
            raise ConfigError(f"[layers.{name}] must be a table")

    order = raw.get("order")
    if not isinstance(order, list) or not all(isinstance(n, str) for n in order):
        raise ConfigError("[order] must list the layer names from lowest to highest")
    for name in order:
        if name not in layers:
            raise ConfigError(f"[order] names unknown layer {name!r}")
    if len(set(order)) != len(order):
        raise ConfigError("[order] lists a layer twice")
    missing = sorted(set(layers) - set(order))
    if missing:
        raise ConfigError(f"layers missing from [order]: {', '.join(missing)}")

    files = _table(raw, "files", path)
    if not files:
        raise ConfigError("[files] must map Cargo.toml paths to layers")
    for manifest, layer in files.items():
        if layer not in layers:
            raise ConfigError(f"file {manifest!r} names unknown layer {layer!r}")

    crates = _table(raw, "crates", path)
    for crate, layer in crates.items():
        if layer not in layers:
            raise ConfigError(f"crate {crate!r} names unknown layer {layer!r}")

    allow = _table(raw, "allow", path)
    exceptions = _table(raw, "domain_exceptions", path)
    # Crates are known through a [files] path or a [crates] entry.
    known = {os.path.basename(os.path.dirname(m.replace("\\", "/")))
             for m in files} | set(crates)
    for crate in _strings(layers.get("domain", {}), "crates", f"layers.{DOMAIN}"):
        if crate not in known:
            warn(f"domain crate {crate!r} has no [files] path or [crates] entry")
    dependency_free = _strings(raw, "dependency_free", "dependency_free")
    for crate in dependency_free:
        if crate not in known:
            warn(f"dependency-free crate {crate!r} has no [files] path or [crates] entry")
    for crate in dependency_free:
        if crate not in crates and not crate.startswith("moly-"):
            warn(f"dependency-free crate {crate!r}: expected a directory crates/*/{crate} in [files]")

    return {
        "rank": {name: i for i, name in enumerate(order)},
        "domain": DOMAIN,
        "files": dict(files),
        "crates": dict(crates),
        "deny_unassigned": bool(raw.get("deny_unassigned", False)),
        "game_layers": set(_strings(raw, "game_layers", "root")),
        "edges": _edge_set(allow, "edges", "allow"),
        "game_edges": _edge_set(allow, "game_edges", "allow"),
        "dev_edges": _edge_set(allow, "dev_edges", "allow"),
        "domain_exceptions": _edge_set(exceptions, "allowed", "domain_exceptions"),
        "dependency_free": dependency_free,
    }


# --------------------------------------------------------------------------
# metadata


_SHELLS = {"bash", "bash.exe", "sh", "sh.exe"}


def _prepare_shell(argv):
    """Make 'bash <script> ...' run the right bash on Windows.

    A bare ``bash`` on Windows may resolve to the Windows Subsystem for Linux
    stub, which cannot see Windows paths. When the wrapper script is a Windows
    path, use Git for Windows' bash if one is installed, and hand the script to
    it in MSYS form (/c/...). Everything else is passed through untouched.
    """
    if os.name != "nt" or len(argv) < 2 or os.environ.get("MSYSTEM"):
        return argv
    if os.path.basename(argv[0]).lower() not in _SHELLS:
        return argv
    script = argv[1]
    if not os.path.isfile(script):
        return argv
    argv = list(argv)
    if os.path.dirname(argv[0]) == "":
        for base in (os.environ.get("ProgramFiles", r"C:\Program Files"),
                     os.environ.get("ProgramFiles(x86)", r"C:\Program Files (x86)")):
            candidate = os.path.join(base, "Git", "bin", "bash.exe")
            if os.path.isfile(candidate):
                argv[0] = candidate
                break
    drive, rest = os.path.splitdrive(os.path.abspath(script))
    drive = drive.rstrip(":").lower()
    if drive:
        argv[1] = f"/{drive}{rest.replace(os.sep, '/')}"
    return argv


def run_cargo(cargo):
    argv = _prepare_shell(shlex.split(cargo, posix=(os.name != "nt"))) + [
        "metadata",
        "--format-version",
        "1",
        "--no-deps",
    ]
    try:
        proc = subprocess.run(argv, stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=False)
    except FileNotFoundError:
        raise ConfigError(f"cargo command not found: {argv[0]!r}")
    if proc.returncode != 0:
        err = proc.stderr.decode("utf-8", "replace").strip()
        raise ConfigError(f"cargo metadata failed (exit {proc.returncode}): {err}")
    return proc.stdout


def load_metadata(args):
    if args.metadata:
        with open(args.metadata, "rb") as fh:
            return json.loads(fh.read().decode("utf-8-sig"))
    cargo = args.cargo or os.environ.get("MOLY_CARGO") or "cargo"
    return json.loads(run_cargo(cargo).decode("utf-8-sig"))


def _posix(path):
    return path.replace("\\", "/").rstrip("/")


def collect(data, name_aliases=None):
    """Return (members, edges) from 'cargo metadata --no-deps' output.

    members: sorted list of workspace member crate names.
    edges: dict (from, to) -> sorted labels drawn from
    {"normal", "dev", "build", "optional", "target: <cfg>"}.

    A dependency points at a workspace member when its ``path`` matches the
    member's manifest directory. When the producer records no path, a
    dependency without a registry source is matched by crate name, then
    through ``name_aliases`` (dependency name -> workspace crate name).
    """
    packages = data.get("packages", [])
    ids = set(data.get("workspace_members") or [p.get("id") for p in packages])
    members = {p["name"]: p for p in packages if p.get("id") in ids}
    by_path = {}
    for pkg in members.values():
        manifest = pkg.get("manifest_path")
        if manifest:
            by_path[os.path.dirname(_posix(manifest))] = pkg["name"]

    edges = {}
    for name, pkg in members.items():
        for dep in pkg.get("dependencies", []):
            target = None
            if dep.get("path"):
                target = by_path.get(_posix(dep["path"]))
            if target is None and not dep.get("source"):
                target = dep.get("name") if dep.get("name") in members else None
            if target is None:
                target = (name_aliases or {}).get(dep.get("name"))
            if target not in members:
                continue
            labels = set()
            if dep.get("kind") == "dev":
                labels.add("dev")
            elif dep.get("kind") == "build":
                labels.add("build")
            else:
                labels.add(NORMAL)
            if dep.get("target"):
                labels.add(f"target: {dep['target']}")
            if dep.get("optional"):
                labels.add("optional")
            edges.setdefault((name, target), set()).update(labels)
    return sorted(members), {k: sorted(v) for k, v in edges.items()}


def layers_by_crate(data, members, cfg):
    """Map crate name -> layer, from [crates] (by name) and [files] (by path)."""
    root = _posix(data.get("workspace_root") or "").lower()
    assigned = {}
    for pkg in data.get("packages", []):
        manifest = pkg.get("manifest_path")
        if not manifest or pkg.get("name") not in members:
            continue
        rel = os.path.dirname(_posix(manifest))
        if root and rel.lower().startswith(root + "/"):
            rel = rel[len(root) + 1:]
        layer = cfg["files"].get(f"{rel}/Cargo.toml")
        if layer:
            assigned[pkg["name"]] = layer
    assigned.update(cfg["crates"])
    return assigned


# --------------------------------------------------------------------------
# rules


def _is_upward_skip(labels, key, cfg):
    """True when 'key' may point to the same layer or upwards."""
    if key in cfg["edges"]:
        return True
    if "dev" in labels and key in cfg["dev_edges"]:
        return True
    if any(l == "optional" or l.startswith("target:") for l in labels):
        return key in cfg["game_edges"]
    return False


def check(members, edges, assigned, cfg):
    """Return a list of (from, to, reason) violations."""
    violations = []
    rank = cfg["rank"]
    reported = set()

    def no_layer(crate):
        if crate not in reported:
            reported.add(crate)
            violations.append((crate, "(none)", "crate has no layer in layers.toml"))

    for crate in members:
        if crate not in assigned and cfg["deny_unassigned"]:
            no_layer(crate)
    for crate in assigned:
        if crate not in members:
            warn(f"layers.toml assigns layer {assigned[crate]!r} to {crate!r}, "
                 "which is not a workspace member")

    for (src, dst), labels in sorted(edges.items()):
        if src == dst:
            continue
        tag = f" [{', '.join(labels)}]"
        key = f"{src} -> {dst}"
        if key in cfg["edges"]:
            continue
        lsrc, ldst = assigned.get(src), assigned.get(dst)
        if lsrc is None or ldst is None:
            if cfg["deny_unassigned"]:
                no_layer(src if lsrc is None else dst)
            continue

        if lsrc == cfg["domain"] and ldst == cfg["domain"]:
            if key not in cfg["domain_exceptions"]:
                violations.append((src, dst, f"domain crates must not depend on each other{tag}"))
            continue

        if lsrc == "server" and ldst in cfg["game_layers"]:
            violations.append((src, dst, f"server layer must not use game layer {ldst!r}{tag}"))
            continue

        if _is_upward_skip(labels, key, cfg):
            continue
        if rank[ldst] >= rank[lsrc]:
            if rank[ldst] == rank[lsrc]:
                reason = f"same-layer edge in {lsrc!r} is not allowed"
            else:
                reason = f"upward dependency {lsrc!r} -> {ldst!r}"
            violations.append((src, dst, f"{reason}{tag}"))

    for crate in cfg["dependency_free"]:
        if crate not in members:
            # Loading the configuration already warned about this crate.
            continue
        for (src, dst), labels in sorted(edges.items()):
            if src != crate:
                continue
            # The reason below replaces a layer-order finding on this edge.
            violations[:] = [v for v in violations if (v[0], v[1]) != (src, dst)]
            if NORMAL in labels:
                also = " + ".join(k for k in ("dev", "build") if k in labels)
                note = f" (also declared as {also})" if also else ""
                violations.append((src, dst, f"[dependencies] must be empty{note}"))
            elif "dev" in labels:
                violations.append((src, dst, "[dev-dependencies] on a dependency-free crate"))
    return violations


# --------------------------------------------------------------------------
# entry point


def main(argv=None):
    parser = argparse.ArgumentParser(
        description="Check workspace crate layer rules from layers.toml."
    )
    parser.add_argument(
        "--metadata",
        metavar="JSON",
        help="use a saved 'cargo metadata --format-version 1 --no-deps' output",
    )
    parser.add_argument(
        "--layers",
        default=DEFAULT_LAYERS,
        metavar="TOML",
        help="rules file (default: layers.toml next to this script)",
    )
    parser.add_argument(
        "--cargo",
        default=None,
        metavar="CMD",
        help="cargo command to run (default: $MOLY_CARGO, else 'cargo')",
    )
    parser.add_argument("-v", "--verbose", action="store_true", help="print every checked edge")
    args = parser.parse_args(argv)

    try:
        cfg = load_config(args.layers)
        data = load_metadata(args)
        members, edges = collect(data, cfg["crates"])
        assigned = layers_by_crate(data, members, cfg)
    except (ConfigError, OSError, ValueError, tomllib.TOMLDecodeError) as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 2

    if not members:
        print("error: metadata lists no workspace members", file=sys.stderr)
        return 2

    if args.verbose:
        for (src, dst), labels in sorted(edges.items()):
            print(
                f"edge {src}[{assigned.get(src)}] -> {dst}[{assigned.get(dst)}] "
                f"({', '.join(labels)})"
            )

    violations = check(members, edges, assigned, cfg)
    if violations:
        for src, dst, reason in violations:
            print(f"{src} -> {dst}: {reason}")
        print(f"{len(violations)} violation(s) in {len(members)} crates / {len(edges)} edges")
        return 1

    print(f"ok: {len(members)} crates, {len(edges)} dependency edges, layers respected")
    return 0


if __name__ == "__main__":
    sys.exit(main())
