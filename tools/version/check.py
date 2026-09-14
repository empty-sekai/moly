"""Check workspace version metadata without resolving or building dependencies."""

from pathlib import Path
import re
import sys
import tomllib


def check():
    root = Path(__file__).resolve().parents[2]
    workspace = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))["workspace"]
    version = workspace["package"]["version"]
    if not re.fullmatch(r"(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?", version):
        raise ValueError("workspace.package.version must be a semantic version")
    lock = tomllib.loads((root / "Cargo.lock").read_text(encoding="utf-8"))
    local_packages = {item["name"]: item["version"] for item in lock["package"] if "source" not in item}
    members = []
    for member in workspace["members"]:
        package = tomllib.loads((root / member / "Cargo.toml").read_text(encoding="utf-8"))["package"]
        name = package["name"]
        if package["version"] != {"workspace": True}:
            raise ValueError(f"{name}: inherit workspace.package.version with version.workspace = true")
        if local_packages.get(name) != version:
            raise ValueError(f"{name}: Cargo.lock does not match workspace version {version}")
        members.append(name)
    print(f"moly {version}: all {len(members)} workspace crates and Cargo.lock agree")


if __name__ == "__main__":
    try:
        check()
    except (KeyError, OSError, ValueError) as error:
        print(f"Version check failed: {error}", file=sys.stderr)
        sys.exit(1)
