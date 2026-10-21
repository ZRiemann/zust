#!/usr/bin/env python3
"""Locate Forge, then invoke the project-owned build definition."""

from __future__ import annotations

import os
import sys
from pathlib import Path


def main() -> int:
    root = Path(__file__).resolve().parent
    workspace = Path(
        os.path.expandvars(os.path.expanduser(os.environ.get("ZETAX_ROOT", str(root.parent))))
    ).resolve()
    common = workspace / "zeta_forge" / "common"
    if not (common / "zeta_forge" / "build_cli.py").is_file():
        print(f"Forge build library is missing: {common}", file=sys.stderr)
        return 1
    sys.path.insert(0, str(common))
    from zeta_forge.build_cli import cli
    from builder.zust import project

    return cli(project(Path(__file__)))


if __name__ == "__main__":
    raise SystemExit(main())
