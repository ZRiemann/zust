"""Project-owned libraries and explicit examples; Forge owns Cargo execution."""
from pathlib import Path

from zeta_forge.build_cli import Product, Project
from zeta_forge.config import load_repo_config
from zeta_forge.rust_engine import RustEngine, RustTarget


def project(script: Path) -> Project:
    root = script.resolve().parent
    actions = ("doctor", "check", "build", "rebuild", "test", "clean", "fetch")
    engine = RustEngine(
        root, load_repo_config(script),
        {
            "zust": RustTarget("zust", root, artifact_name="libz.rlib", features=("log",)),
            "zust-log": RustTarget("zust-log", root, artifact_name="libzust_log.rlib"),
            "logging": RustTarget("zust", root, features=("log",), example="logging"),
        },
        root / "build" / "cargo",
    )
    return Project(
        "zust",
        (Product("zust", "rust", "ZetaX z::xxx facade (log enabled)", actions),
         Product("zust-log", "rust", "Independent logging component", actions),
         Product("logging", "rust", "Logging example (explicit selection)", actions + ("run", "dev"))),
        {"rust": engine}, ("zust",),
        action_defaults={"check": ("zust", "zust-log"), "test": ("zust", "zust-log")},
    )
