import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from build import flutter_args, package_ui


class PackagingTests(unittest.TestCase):
    @patch("build.shutil.which", return_value="/bin/flutter")
    def test_web_wasm_flag(self, _):
        self.assertIn("--wasm", flutter_args("pwa", True))
        self.assertNotIn("--wasm", flutter_args("pwa", False))
        self.assertIn("--base-href", flutter_args("chromium", False))

    def test_extension_assets_are_local(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp)
            source = path / "web"
            source.mkdir()
            (source / "index.html").write_text("<html></html>")
            (source / "flutter_bootstrap.js").write_text("// bootstrap")
            (source / "main.dart.js").write_text("// code")
            (source / "main.dart.js.map").write_text("map")
            package_ui(source, path / "ui")
            self.assertTrue((path / "ui" / "index.html").exists())
            self.assertFalse((path / "ui" / "main.dart.js.map").exists())

    def test_broken_bundle_fails_closed(self):
        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaises(FileNotFoundError):
                package_ui(Path(tmp), Path(tmp) / "out")


if __name__ == "__main__":
    unittest.main()
