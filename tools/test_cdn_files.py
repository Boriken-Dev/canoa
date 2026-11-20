import hashlib
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import cdn_files as cdn
import office_js as oj

LOADER = (
    'var OSF=OSF||{};OSF.HostSpecificFileVersionMap={outlook:{web:"16.01",win32:"16.02"},'
    'word:{web:"16.00"},excel:{web:"16.00"},powerpoint:{web:"16.00"},onenote:{web:"16.00"}};'
    'OSF.SupportedLocales={"en-us":true,"ja-jp":true};OSF.AssociatedLocales={};'
)


def entry(data: bytes, path: str = "/a.woff2") -> dict:
    return dict(host="fonts.example", path=path, sha256=cdn.digest(data))


class CdnFilesTests(unittest.TestCase):
    def test_the_committed_list_pins_the_fonts(self):
        files = cdn.read_list()["files"]
        self.assertGreater(len(files), 700)
        self.assertEqual({item["host"] for item in files}, {"fonts.gstatic.com"})

    def test_an_entry_that_is_not_a_plain_cdn_file_is_refused(self):
        for bad in (
            dict(host="x.example", path="/../etc/passwd", sha256="0" * 64),
            dict(host="x.example", path="relative", sha256="0" * 64),
            dict(host="X.example/", path="/a", sha256="0" * 64),
            dict(host="x.example", path="/a", sha256="abc"),
        ):
            with self.assertRaises(cdn.CdnError, msg=bad):
                cdn.check_entry(bad)

    def test_mirror_downloads_checks_and_reuses_a_file(self):
        body = b"font"
        calls = []

        def fetch(url):
            calls.append(url)
            return body

        original, cdn.fetch = cdn.fetch, fetch
        try:
            with tempfile.TemporaryDirectory() as tmp:
                deps = Path(tmp)
                done = cdn.mirror(deps, [entry(body)])
                self.assertEqual(done[0]["bytes"], len(body))
                self.assertEqual((deps / "cdn" / "fonts.example" / "a.woff2").read_bytes(), body)
                cdn.mirror(deps, [entry(body)])
                self.assertEqual(len(calls), 1, "a file already there with its digest is not fetched again")
                with self.assertRaisesRegex(cdn.CdnError, "changed upstream"):
                    cdn.mirror(deps, [entry(b"other", "/b.woff2")])
        finally:
            cdn.fetch = original


class OfficeJsTests(unittest.TestCase):
    def test_the_committed_list_pins_a_whole_tree_and_names_what_the_pane_loads(self):
        data = oj.read_list()
        self.assertRegex(data["commit"], r"^[0-9a-f]{40}$")
        self.assertIn("LICENSE.md", data["files"])
        self.assertIn("dist/office.js", data["carried"])
        self.assertIn("LICENSE.md", data["carried"])
        self.assertTrue(any(name.endswith("/outlook-web-16.01.js") for name in data["carried"]))
        self.assertGreater(len(data["files"]), 700)
        self.assertLess(len(data["carried"]), len(data["files"]))

    def test_the_loader_names_the_files_of_the_applications_and_the_languages(self):
        present = {"dist/office.js", "dist/outlook-web-16.01.js", "dist/en-us/office_strings.js", "dist/en-us/outlook_strings.js"}
        found, absent = oj.carried_names(LOADER, present)
        self.assertEqual(sorted(found), sorted(present))
        self.assertIn("dist/outlook-win32-16.02.js", absent)
        self.assertIn("dist/ja-jp/outlook_strings.js", absent)
        self.assertNotIn("dist/onenote-web-16.00.js", absent)
        with self.assertRaises(oj.OfficeJsError):
            oj.carried_names("var nothing;", present)

    def test_mirror_writes_the_tree_as_it_is_and_refuses_another_tree(self):
        tree = {"LICENSE.md": b"terms\r\n", "dist/office.js": b"var x;"}
        data = dict(commit="a" * 40, files={n: hashlib.sha256(b).hexdigest() for n, b in tree.items()})
        calls = []
        original = oj.tree_of
        oj.tree_of = lambda commit: calls.append(commit) or dict(tree)
        try:
            with tempfile.TemporaryDirectory() as tmp:
                deps = Path(tmp)
                done = oj.mirror(deps, data)
                self.assertEqual(len(done), 2)
                self.assertEqual((deps / "cdn" / "office-js" / "LICENSE.md").read_bytes(), b"terms\r\n")
                oj.mirror(deps, data)
                self.assertEqual(calls, ["a" * 40], "a mirrored tree is not fetched again")
                (deps / "cdn" / "office-js" / "extra.js").write_bytes(b"")
                with self.assertRaisesRegex(oj.OfficeJsError, "does not pin"):
                    oj.mirror(deps, data)
            oj.tree_of = lambda commit: {"LICENSE.md": b"changed", "dist/office.js": b"var x;"}
            with tempfile.TemporaryDirectory() as tmp, self.assertRaisesRegex(oj.OfficeJsError, "is not the tree"):
                oj.mirror(Path(tmp), data)
        finally:
            oj.tree_of = original


if __name__ == "__main__":
    unittest.main()
