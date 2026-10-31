import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from build import flutter_args, package_ui, package_debug_extension


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
            (source / "index.html").write_text("<html><head></head><body></body></html>")
            (source / "flutter_bootstrap.js").write_text("// bootstrap")
            (source / "main.dart.js").write_text("// code")
            (source / "main.dart.js.map").write_text("map")
            package_ui(source, path / "ui")
            self.assertTrue((path / "ui" / "index.html").exists())
            self.assertIn("../bridge.js", (path / "ui" / "index.html").read_text())
            self.assertFalse((path / "ui" / "main.dart.js.map").exists())

    def test_debug_extension_is_a_distinct_unpacked_build(self):
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            source = base / "chromium"
            (source / "ui").mkdir(parents=True)
            (source / "ui" / "index.html").write_text("<html></html>")
            (source / "manifest.json").write_text('{"permissions":["tabs"]}')
            (source / "manifest.debug.json").write_text(
                '{"permissions":["tabs","debugger"]}'
            )
            package_debug_extension(source, base / "chromium-debug")
            self.assertIn(
                "debugger",
                (base / "chromium-debug" / "manifest.json").read_text(),
            )
            self.assertNotIn(
                "debugger", (source / "manifest.json").read_text(),
            )

    def test_broken_bundle_fails_closed(self):
        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaises(FileNotFoundError):
                package_ui(Path(tmp), Path(tmp) / "out")



    def test_pinned_coqui_peer_method_matches_git_blob(self):
        import hashlib
        import json
        source = (Path(__file__).resolve().parents[1] / "web" / "vendor" / "coqui-peer-webcrypto.mjs").read_bytes()
        meta = json.loads((Path(__file__).resolve().parents[1] / "web" / "vendor" / "COQUI-SOURCE.json").read_text())
        self.assertEqual(
            hashlib.sha1(f"blob {len(source)}\0".encode() + source).hexdigest(),
            meta["git_blob_sha1"],
        )

if __name__ == "__main__":
    unittest.main()
