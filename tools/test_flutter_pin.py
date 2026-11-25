import shutil
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
            containerfile.write_text(containerfile.read_text() + "ARG FLUTTER_STORAGE_BASE_URL=https://example.test\n")
            workflow = root / ".github/workflows/flutter.yml"
            workflow.write_text(workflow.read_text() + f"      - uses: subosito/flutter-action@v2\n        with:\n          flutter-version: {PIN['FLUTTER_UPSTREAM']}\n")
            docs = root / "docs/getting-started.md"
            docs.write_text(docs.read_text().replace(PIN["FLUTTER_VERSION"], "9.9.9"))
            problems = "\n".join(fp.consumer_problems(root, PIN))
            self.assertIn("Containerfile: still has FLUTTER_STORAGE_BASE_URL", problems)
            self.assertIn("flutter.yml: installs Flutter on its own", problems)
            self.assertIn("flutter.yml: names the Flutter commit", problems)
            self.assertIn(f"does not name {PIN['FLUTTER_VERSION']}", problems)


if __name__ == "__main__":
    unittest.main()
