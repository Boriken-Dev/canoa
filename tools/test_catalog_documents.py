import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import catalog_documents as cd


def entry(data: bytes, path: str = "canoa-x/doc.json") -> dict:
    return dict(path=path, url="https://host.example/doc", sha256=cd.digest(data), bytes=len(data))


class CatalogDocumentsTests(unittest.TestCase):
    def setUp(self):
        self.calls = []
        self.original = cd.fetch

    def tearDown(self):
        cd.fetch = self.original

    def serving(self, body: bytes) -> None:
        def fetch(url):
            self.calls.append(url)
            return body

        cd.fetch = fetch

    def test_the_committed_list_pins_the_documents_the_shipped_snapshots_are_made_from(self):
        paths = {each["path"] for each in cd.read_list()}
        self.assertEqual(
            {path.split("/")[0] for path in paths}, {"canoa-whisper", "canoa-piper"}, paths
        )
        for each in cd.read_list():
            self.assertNotEqual(each["sha256"], "0" * 64, each["path"])

    def test_an_entry_that_is_not_a_plain_https_document_is_refused(self):
        for bad in (
            dict(path="../x", url="https://h/x", sha256="0" * 64),
            dict(path="/abs", url="https://h/x", sha256="0" * 64),
            dict(path="a/b", url="http://h/x", sha256="0" * 64),
            dict(path="a/b", url="https://h/x", sha256="abc"),
        ):
            with self.assertRaises(cd.CatalogError, msg=bad):
                cd.check_entry(bad)

    def test_sync_downloads_checks_and_reuses_a_document_and_check_names_what_is_wrong(self):
        body = b'{"a": 1}'
        item = entry(body)
        self.serving(body)
        with tempfile.TemporaryDirectory() as tmp:
            deps = Path(tmp)
            self.assertEqual(cd.mirror(deps, [item]), ["canoa-x/doc.json"])
            self.assertEqual(cd.mirror(deps, [item]), [], "a document that is there is not fetched again")
            self.assertEqual(len(self.calls), 1)
            self.assertEqual(cd.problems(deps, [item]), [])
            (deps / "catalogs" / "canoa-x" / "doc.json").write_bytes(b"changed")
            self.assertEqual(
                cd.problems(deps, [item]), ["canoa-x/doc.json: changed, its digest is not the pinned one"]
            )
            (deps / "catalogs" / "canoa-x" / "other.json").write_bytes(b"x")
            self.assertTrue(any("other.json: stale" in line for line in cd.problems(deps, [item])))
            cd.mirror(deps, [item])
            self.assertEqual(cd.problems(deps, [item]), [], "sync repairs the changed and removes the stale")
            self.assertFalse((deps / "catalogs" / "canoa-x" / "other.json").exists())
            (deps / "catalogs" / "canoa-x" / "doc.json").unlink()
            self.assertEqual(cd.problems(deps, [item]), ["canoa-x/doc.json: missing"])

    def test_a_document_that_is_not_the_pinned_one_is_refused_and_nothing_is_written(self):
        self.serving(b"not what was pinned")
        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaises(cd.CatalogError):
                cd.mirror(Path(tmp), [entry(b"what was pinned")])
            self.assertFalse((Path(tmp) / "catalogs" / "canoa-x" / "doc.json").exists())


if __name__ == "__main__":
    unittest.main()
