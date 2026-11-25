import json
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import catalog_documents as cd


class CatalogDocumentsTests(unittest.TestCase):
    def setUp(self):
        self.original = cd.fetch

    def tearDown(self):
        cd.fetch = self.original

    def test_the_index_pins_the_documents_the_shipped_snapshots_are_made_from(self):
        entries = [e for e in json.loads(cd.LIST.read_text(encoding="utf-8"))["files"] if e["host"] == cd.HOST]
        paths = "\n".join(e["path"] for e in entries)
        self.assertIn("whisper.cpp/tree/", paths)
        self.assertIn("piper-voices/raw/", paths)
        self.assertEqual(len(entries), 4)
        for entry in entries:
            self.assertNotEqual(entry["sha256"], "0" * 64, entry["path"])

    def test_refresh_rewrites_the_digest_of_a_catalog_document_and_leaves_the_other_entries(self):
        fonts = dict(host="fonts.example", path="/s/a.woff2", sha256="1" * 64, bytes=1)
        doc = dict(host=cd.HOST, path="/api/doc", sha256="0" * 64, bytes=0)
        asked = []
        cd.fetch = lambda url: asked.append(url) or b'{"a": 1}'
        with tempfile.TemporaryDirectory() as tmp:
            index = Path(tmp) / "cdn_files.json"
            index.write_text(json.dumps({"files": [fonts, doc]}), encoding="utf-8")
            self.assertEqual(cd.refresh(index), ["/api/doc"])
            files = json.loads(index.read_text(encoding="utf-8"))["files"]
        self.assertEqual(asked, ["https://huggingface.co/api/doc"])
        self.assertEqual(files[0], fonts)
        self.assertEqual((files[1]["sha256"], files[1]["bytes"]), (cd.digest(b'{"a": 1}'), 8))


if __name__ == "__main__":
    unittest.main()
