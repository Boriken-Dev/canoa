import hashlib
import json
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import fallback_fonts as ff

NAME = "notosanssymbols/v43/abc.woff2"


def sdk_asking_for(root: Path, *names: str) -> Path:
    data = root / "sdk" / ff.ENGINE_DATA
    data.parent.mkdir(parents=True)
    data.write_text("".join(f"NotoFont('A', '{name}'),\n" for name in names), encoding="utf-8")
    return root / "sdk"


def deps_holding(root: Path, name: str, body: bytes) -> Path:
    target = root / "deps" / "cdn" / ff.HOST / "s" / name
    target.parent.mkdir(parents=True)
    target.write_bytes(body)
    return root / "deps"


class FallbackFontTests(unittest.TestCase):
    def test_the_committed_index_holds_the_fonts_with_licences(self):
        entries = [e for e in json.loads(ff.LIST.read_text(encoding="utf-8"))["files"] if e["host"] == ff.HOST]
        self.assertGreater(len(entries), 700)
        self.assertTrue(any(e["path"].startswith("/s/notocoloremoji/") for e in entries))
        self.assertEqual({e["licence"] for e in entries}, {ff.LICENCE})

    def test_the_engine_names_the_fonts(self):
        with tempfile.TemporaryDirectory() as tmp:
            sdk = sdk_asking_for(Path(tmp), NAME, "x/v1/y.woff2")
            self.assertEqual(ff.engine_paths(sdk), [NAME, "x/v1/y.woff2"])
            with self.assertRaisesRegex(ff.FontError, "is missing"):
                ff.engine_paths(Path(tmp) / "none")

    def test_a_path_that_is_not_a_plain_woff2_is_refused(self):
        with tempfile.TemporaryDirectory() as tmp:
            sdk = sdk_asking_for(Path(tmp), "../escape.woff2")
            with self.assertRaisesRegex(ff.FontError, "not a plain woff2"):
                ff.engine_paths(sdk)

    def test_pin_rewrites_the_font_entries_and_keeps_the_others(self):
        body = b"wOF2font"
        other = dict(host="example.test", path="/lib.js", sha256="1" * 64)
        stale = dict(host=ff.HOST, path="/s/gone/v1/old.woff2", sha256="2" * 64, bytes=1, licence=ff.LICENCE)
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            index = root / "cdn_files.json"
            index.write_text(json.dumps({"files": [other, stale]}), encoding="utf-8")
            counts = ff.pin(sdk_asking_for(root, NAME), deps_holding(root, NAME, body), index)
            self.assertEqual(counts, (1, 1, 1))
            files = json.loads(index.read_text(encoding="utf-8"))["files"]
            self.assertEqual(files[0], other)
            self.assertEqual(files[1], dict(host=ff.HOST, path="/s/" + NAME, sha256=hashlib.sha256(body).hexdigest(),
                                            bytes=len(body), licence=ff.LICENCE))
            self.assertEqual(len(files), 2)


if __name__ == "__main__":
    unittest.main()
