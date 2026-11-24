import json
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import flutter_pin as fp

ROOT = Path(__file__).resolve().parents[1]
PIN = fp.load()


def copy_consumers(into: Path) -> None:
    for name in ("packaging/container/Containerfile", "docs/getting-started.md", ".github/actions/flutter/action.yml"):
        (into / name).parent.mkdir(parents=True, exist_ok=True)
        shutil.copy(ROOT / name, into / name)
    shutil.copytree(ROOT / ".github/workflows", into / ".github/workflows")


def git(*args: str, cwd: Path) -> None:
    subprocess.run(["git", "-c", "user.name=t", "-c", "user.email=t@example.test", *args], check=True, cwd=cwd, capture_output=True)


class PinFile(unittest.TestCase):
    def test_the_pin_is_complete(self):
        for key in fp.KEYS:
            self.assertTrue(PIN[key])
        self.assertEqual(PIN["FLUTTER_IMAGE_TAG"], PIN["FLUTTER_COMMIT"][:10])

    def test_a_pin_without_a_hash_is_refused(self):
        with tempfile.TemporaryDirectory() as d:
            path = Path(d) / "flutter.pin"
            path.write_text("FLUTTER_COMMIT=abc\nFLUTTER_UPSTREAM=x\nFLUTTER_VERSION=1\nFLUTTER_ENGINE=y\n")
            with self.assertRaises(ValueError):
                fp.load(path)


class Consumers(unittest.TestCase):
    def test_the_repository_agrees_with_the_pin(self):
        self.assertEqual(fp.consumer_problems(ROOT, PIN), [])

    def test_a_consumer_that_names_another_sdk_is_found(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            copy_consumers(root)
            containerfile = root / "packaging/container/Containerfile"
            containerfile.write_text(containerfile.read_text().replace(f"FLUTTER_IMAGE_TAG={PIN['FLUTTER_IMAGE_TAG']}", "FLUTTER_IMAGE_TAG=0000000000"))
            workflow = root / ".github/workflows/flutter.yml"
            workflow.write_text(workflow.read_text() + f"      - uses: subosito/flutter-action@v2\n        with:\n          flutter-version: {PIN['FLUTTER_UPSTREAM']}\n")
            docs = root / "docs/getting-started.md"
            docs.write_text(docs.read_text().replace(PIN["FLUTTER_VERSION"], "9.9.9"))
            problems = "\n".join(fp.consumer_problems(root, PIN))
            self.assertIn("FLUTTER_IMAGE_TAG", problems)
            self.assertIn("flutter.yml: installs Flutter on its own", problems)
            self.assertIn("flutter.yml: names the Flutter commit", problems)
            self.assertIn(f"does not name {PIN['FLUTTER_VERSION']}", problems)


class Sdk(unittest.TestCase):
    def make(self, root: Path, engine: str, version: str) -> Path:
        sdk = root / "sdk"
        (sdk / "bin/internal").mkdir(parents=True)
        (sdk / "bin/cache").mkdir()
        (sdk / "bin/cache/engine.stamp").write_text(engine + "\n")
        (sdk / "bin/cache/flutter.version.json").write_text(json.dumps({"flutterVersion": version}))
        git("init", "-q", cwd=sdk)
        (sdk / "f").write_text("x")
        git("add", "f", cwd=sdk)
        git("commit", "-qm", "x", cwd=sdk)
        return sdk

    def test_an_sdk_at_neither_commit_is_not_the_pin(self):
        with tempfile.TemporaryDirectory() as d:
            sdk = self.make(Path(d), PIN["FLUTTER_ENGINE"], PIN["FLUTTER_VERSION"])
            problems = fp.sdk_problems(sdk, PIN)
            self.assertEqual(len(problems), 1)
            self.assertIn("is at", problems[0])

    def test_an_sdk_with_another_engine_or_version_is_not_the_pin(self):
        with tempfile.TemporaryDirectory() as d:
            sdk = self.make(Path(d), "0" * 40, "3.0.0")
            problems = "\n".join(fp.sdk_problems(sdk, PIN))
            self.assertIn("engine " + "0" * 40, problems)
            self.assertIn("version 3.0.0", problems)

    def test_an_sdk_at_the_pin_matches(self):
        with tempfile.TemporaryDirectory() as d:
            sdk = self.make(Path(d), PIN["FLUTTER_ENGINE"], PIN["FLUTTER_VERSION"])
            head = subprocess.run(["git", "-C", str(sdk), "rev-parse", "HEAD"], capture_output=True, text=True).stdout.strip()
            self.assertEqual(fp.sdk_problems(sdk, dict(PIN, FLUTTER_UPSTREAM=head)), [])

    def test_a_folder_that_is_no_sdk_is_refused(self):
        with tempfile.TemporaryDirectory() as d:
            self.assertEqual(len(fp.sdk_problems(Path(d), PIN)), 1)


if __name__ == "__main__":
    unittest.main()
