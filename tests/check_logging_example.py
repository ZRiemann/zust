"""Explicit, bounded zbuild example smoke test; excluded from default discovery."""
from __future__ import annotations

import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

# Ordered message/level contract, independent of timestamps and caller locations.
INFO_MESSAGES = [
    "[example:startup] service started",
    "[example:scalar] i8=-8",
    "[example:scalar] i16=-1600",
    "[example:scalar] i32=-32000",
    "[example:scalar] i64=-64000",
    "[example:scalar] i128=-123456789012345678901234567890",
    "[example:scalar] isize=-42",
    "[example:scalar] u8=8 hex=0x08",
    "[example:scalar] u16=1600",
    "[example:scalar] u32=32000",
    "[example:scalar] u64=64000",
    "[example:scalar] u128=123456789012345678901234567890",
    "[example:scalar] usize=42",
    "[example:scalar] f32=1.25 precision=1.250",
    "[example:scalar] f64=123.456789 precision=123.46",
    "[example:scalar] bool=true",
    "[example:scalar] char=量 debug='量'",
    '[example:composite] &str=borrowed text debug="borrowed text"',
    '[example:composite] String=owned text debug="owned text"',
    "[example:composite] array=[1, 2, 3]",
    "[example:composite] slice=[2, 3]",
    '[example:composite] tuple=("order", 3, true)',
    "[example:composite] Option::Some=Some(42)",
    "[example:composite] Option::None=None",
    "[example:composite] Result::Ok=Ok(42)",
    '[example:composite] Result::Err=Err("demo rejection")',
    '[example:custom] Debug=Order { id: 1001, symbol: "DEMO", quantity: 3, price: 123.45 }',
    "[example:custom] Display=Order #1001: 3 x DEMO @ 123.45",
    "[example:custom] OrderLog=order_id=1001 symbol=DEMO quantity=3 price=123.45",
]
LEVEL_RECORDS = [
    ("trace", "[example:trace] demo trace record"),
    ("debug", "[example:debug] demo debug record"),
    ("info", "[example:info] demo info record"),
    ("warning", "[example:warn] demo warning; no actual failure"),
    ("error", "[example:error] demo error; no actual failure"),
    ("critical", "[example:critical] demo critical record; no actual failure"),
]
READY = "[example:ready] service ready"
ALL_RECORDS = [("info", message) for message in INFO_MESSAGES] + LEVEL_RECORDS + [("info", READY)]
IMPORTANT_RECORDS = LEVEL_RECORDS[3:] + [("info", READY)]


class LoggingExampleTests(unittest.TestCase):
    def invoke(self, *arguments: str, cwd: Path = ROOT) -> str:
        result = subprocess.run(
            [str(ROOT / "zbuild.py"), *arguments], cwd=cwd,
            capture_output=True, text=True, check=False,
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        return result.stdout + result.stderr

    def verify_outputs(
        self, directory: Path, action: str, profile: str, level: str = "trace",
    ) -> None:
        runtime = directory / f"{action}-{profile}-{level}.runtime.log"
        important = directory / f"{action}-{profile}-{level}.important.jsonl"
        config = directory / f"{action}-{profile}-{level} configuration.json"
        config.write_text(json.dumps({"example": {"spd": {
            "async": True, "console": True, "name": str(runtime),
            "rotate_on_open": False, "pattern": "%l|%v", "level": level,
            "important": {
                "enabled": True, "name": str(important), "level": "warn",
                "format": "jsonl", "rotate_on_open": False,
            },
        }}}), encoding="utf-8")
        # Invoke outside the repository, but pass a project-relative config path.
        relative_config = str(config.relative_to(ROOT))
        output = self.invoke(
            action, "logging", "--profile", profile, "--", relative_config, "example",
            cwd=directory,
        )
        runtime_records = ALL_RECORDS if level == "trace" else LEVEL_RECORDS[3:]
        expected = [f"{severity}|{message}" for severity, message in runtime_records]
        console = [line for line in output.splitlines() if "[example:" in line]
        self.assertEqual(console, expected, output)
        self.assertNotIn(relative_config, output)
        self.assertEqual(runtime.read_text(encoding="utf-8").splitlines(), expected)
        records = [json.loads(line) for line in important.read_text(encoding="utf-8").splitlines()]
        self.assertEqual(len(records), len(IMPORTANT_RECORDS))
        for record, (severity, message) in zip(records, IMPORTANT_RECORDS):
            self.assertEqual(record["level"], "warn" if severity == "warning" else severity)
            self.assertEqual(record["component"], "example")
            self.assertEqual(record["action"], message.split("]", 1)[0].split(":")[1])
            self.assertEqual(record["message"], message)

    def test_build_and_run_debug_release_and_dev(self) -> None:
        (ROOT / "build").mkdir(exist_ok=True)
        with tempfile.TemporaryDirectory(prefix="logging-example-", dir=ROOT / "build") as temporary:
            directory = Path(temporary)
            for profile in ("debug", "release"):
                with self.subTest(profile=profile):
                    self.invoke("build", "logging", "--profile", profile)
                    suffix = ".exe" if sys.platform == "win32" else ""
                    artifact = ROOT / "build/cargo" / profile / "examples" / f"logging{suffix}"
                    self.assertTrue(artifact.is_file())
                    before = artifact.stat().st_mtime_ns
                    self.verify_outputs(directory, "run", profile)
                    self.verify_outputs(directory, "run", profile, level="warn")
                    self.assertEqual(artifact.stat().st_mtime_ns, before)
                    output = self.invoke("run", "logging", "--profile", profile)
                    messages = [line[line.index("[example:"):].split("\t", 1)[0]
                                for line in output.splitlines() if "[example:" in line]
                    self.assertEqual(messages, [message for _, message in ALL_RECORDS])
            self.verify_outputs(directory, "dev", "debug")


if __name__ == "__main__":
    unittest.main()
