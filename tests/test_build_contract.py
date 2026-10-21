"""Read-only CLI, version and dependency-boundary checks for zust."""
from __future__ import annotations

import hashlib
import json
import subprocess
import tomllib
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def invoke(*arguments: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run([str(ROOT / "zbuild.py"), *arguments], cwd=ROOT, text=True, capture_output=True, check=False)


def source_snapshot() -> dict[str, str]:
    return {
        str(path.relative_to(ROOT)): hashlib.sha256(path.read_bytes()).hexdigest()
        for path in ROOT.rglob("*")
        if path.is_file() and not ({".git", "build", "target", "__pycache__"} & set(path.relative_to(ROOT).parts))
    }


class BuildContractTests(unittest.TestCase):
    def test_read_only_cli_and_unsupported_library_actions(self) -> None:
        before = source_snapshot()
        for args in [("--help",), ("list",), ("metadata",), ("build", "--dry-run"), ("test", "--dry-run"), ("clean", "--dry-run")]:
            result = invoke(*args)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        for args in [("run",), ("dev",), ("run", "zust"), ("run", "zust-log"), ("dev", "zust"), ("dev", "zust-log"), ("install", "zust-log"), ("install", "logging")]:
            self.assertEqual(invoke(*args).returncode, 2)
        self.assertEqual(before, source_snapshot())

    def test_explicit_example_metadata_and_read_only_actions(self) -> None:
        before = source_snapshot()
        outputs_before = {
            str(path): (path.stat().st_size, path.stat().st_mtime_ns)
            for path in (ROOT / "build").rglob("*") if path.is_file()
        }
        for profile in ("debug", "release"):
            result = invoke("metadata", "logging", "--profile", profile)
            self.assertEqual(result.returncode, 0, result.stderr)
            step = json.loads(result.stdout)["steps"][0]
            self.assertEqual(step["examples"], {"logging": "logging"})
            self.assertEqual(step["features"], {"logging": ["log"]})
            self.assertEqual(step["run_directories"], {"logging": str(ROOT)})
            self.assertEqual(Path(step["artifacts"]["logging"]).parent, ROOT / "build/cargo" / profile / "examples")
            for action in ("doctor", "check", "build", "rebuild", "test", "run", "dev", "clean", "fetch"):
                with self.subTest(action=action, profile=profile):
                    args = (action, "logging", "--profile", profile, "--dry-run")
                    if action in {"run", "dev"}:
                        args += ("--", "configuration with spaces.json", "--secret=not-logged")
                    result = invoke(*args)
                    self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                    self.assertIn('"logging": "logging"', result.stdout)
                    self.assertNotIn("not-logged", result.stdout + result.stderr)
                    if action in {"clean", "rebuild"}:
                        self.assertIn('"affected": ["zust", "zust-log", "logging"]', result.stdout)
                        self.assertIn("affected_artifacts", result.stdout)
        self.assertEqual(before, source_snapshot())
        self.assertEqual(outputs_before, {
            str(path): (path.stat().st_size, path.stat().st_mtime_ns)
            for path in (ROOT / "build").rglob("*") if path.is_file()
        })
        self.assertIn("logging", invoke("list").stdout)

    def test_metadata_defaults_and_versions(self) -> None:
        result = invoke("metadata", "zust", "zust-log")
        self.assertEqual(result.returncode, 0, result.stderr)
        metadata = json.loads(result.stdout)
        step = metadata["steps"][0]
        self.assertEqual(step["features"], {"zust": ["log"], "zust-log": []})
        self.assertEqual(Path(step["artifacts"]["zust"]).name, "libz.rlib")
        self.assertEqual(Path(step["artifacts"]["zust-log"]).name, "libzust_log.rlib")
        manifest = tomllib.loads((ROOT / "Cargo.toml").read_text())
        self.assertEqual(manifest["workspace"]["package"]["version"], (ROOT / "VERSION").read_text().strip())
        self.assertEqual(manifest["lib"]["name"], "z")
        result = invoke("test", "--dry-run")
        self.assertIn('"targets": ["zust", "zust-log"]', result.stdout)
        self.assertIn('"targets": ["zust", "zust-log"]', invoke("check", "--dry-run").stdout)
        self.assertIn('"targets": ["zust"]', invoke("build", "--dry-run").stdout)

    def test_facade_without_log_has_no_component_dependency(self) -> None:
        result = subprocess.run(["cargo", "tree", "--locked", "--offline", "-p", "zust", "--no-default-features", "--edges", "normal", "--prefix", "none"], cwd=ROOT, text=True, capture_output=True, check=False)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(result.stdout.strip().splitlines()), 1)
        self.assertTrue(result.stdout.startswith("zust v0.1.0"))


if __name__ == "__main__":
    unittest.main()
