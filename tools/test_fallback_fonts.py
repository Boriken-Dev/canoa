import hashlib
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import fallback_fonts as ff

NAME = "notosanssymbols/v43/abc.woff2"


def deps_with(root: Path, data: bytes) -> Path:
    target = root / "deps" / "cdn" / ff.HOST / "s" / NAME
    target.parent.mkdir(parents=True)
    target.write_bytes(data)
    return root / "deps"


class FallbackFontTests(unittest.TestCase):
    def test_the_committed_list_is_well_formed(self):
        pinned = ff.read_list()
        self.assertGreater(len(pinned), 700)
        self.assertTrue(any(name.startswith("notocoloremoji/") for name in pinned))

    def test_the_list_must_match_the_engine(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            data = root / ff.ENGINE_DATA
            data.parent.mkdir(parents=True)
            data.write_text(f"NotoFont(\n 'A',\n '{NAME}',\n),\nNotoFont('B', 'x/v1/y.woff2'),", encoding="utf-8")
            ff.check_engine({NAME: "0" * 64, "x/v1/y.woff2": "0" * 64}, root)
            with self.assertRaisesRegex(ff.FontError, "does not match"):
                ff.check_engine({NAME: "0" * 64}, root)

    def test_install_copies_a_pinned_file_and_refuses_a_missing_or_changed_one(self):
        body = b"wOF2font"
        pinned = {NAME: hashlib.sha256(body).hexdigest()}
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            deps = deps_with(root, body)
            dist = root / "dist"
            self.assertEqual(ff.install(deps, dist, pinned), len(body))
            self.assertEqual((dist / "assets" / "fonts" / "fallback" / NAME).read_bytes(), body)
            with self.assertRaisesRegex(ff.FontError, "not in the deps mirror"):
                ff.install(deps, dist, {**pinned, "x/v1/y.woff2": "0" * 64})
            with self.assertRaisesRegex(ff.FontError, "not in the deps mirror"):
                ff.install(None, dist, pinned)
            (deps / "cdn" / ff.HOST / "s" / NAME).write_bytes(b"changed")
            with self.assertRaisesRegex(ff.FontError, "differs"):
                ff.install(deps, dist, pinned)


if __name__ == "__main__":
    unittest.main()
