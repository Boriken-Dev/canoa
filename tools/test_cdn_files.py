import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import cdn_files as cdn

LOADER = (
    'var OSF=OSF||{};OSF.HostSpecificFileVersionMap={outlook:{web:"16.01",win32:"16.02"},'
    'word:{web:"16.00"},excel:{web:"16.00"},powerpoint:{web:"16.00"},onenote:{web:"16.00"}};'
    'OSF.SupportedLocales={"en-us":true,"ja-jp":true};OSF.AssociatedLocales={};'
)


def entry(data: bytes, path: str = "/1/a.js") -> dict:
    return dict(host=cdn.OFFICE_HOST, path=path, sha256=cdn.digest(data))


class CdnFilesTests(unittest.TestCase):
    def test_the_committed_list_pins_the_fonts_and_office_js(self):
        files = cdn.read_list()["files"]
        office = [item for item in files if item["host"] == cdn.OFFICE_HOST]
        self.assertGreater(len([item for item in files if item["host"] == "fonts.gstatic.com"]), 700)
        self.assertIn("/1/office.js", [item["path"] for item in office])
        self.assertTrue(any(item["path"].endswith("/outlook-web-16.01.js") for item in office))
        self.assertTrue(any(item["path"] == "/1/en-us/office_strings.js" for item in office))
        self.assertEqual({item["host"] for item in files}, {"fonts.gstatic.com", cdn.OFFICE_HOST})

    def test_an_entry_that_is_not_a_plain_cdn_file_is_refused(self):
        for bad in (
            dict(host="x.example", path="/../etc/passwd", sha256="0" * 64),
            dict(host="x.example", path="relative", sha256="0" * 64),
            dict(host="X.example/", path="/a", sha256="0" * 64),
            dict(host="x.example", path="/a", sha256="abc"),
        ):
            with self.assertRaises(cdn.CdnError, msg=bad):
                cdn.check_entry(bad)

    def test_the_loader_names_the_files_of_the_applications_and_the_languages(self):
        always, platform = cdn.office_names(LOADER)
        self.assertEqual(
            platform, ["outlook-web-16.01.js", "outlook-win32-16.02.js", "word-web-16.00.js",
                       "excel-web-16.00.js", "powerpoint-web-16.00.js"])
        self.assertIn("ja-jp/outlook_strings.js", always)
        self.assertIn("en-us/office_strings.js", always)
        self.assertIn("microsoftajax.js", always)
        with self.assertRaises(cdn.CdnError):
            cdn.office_names("var nothing;")

    def test_mirror_downloads_checks_and_reuses_a_file(self):
        body = b"office"
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
                self.assertEqual((deps / "cdn" / cdn.OFFICE_HOST / "1" / "a.js").read_bytes(), body)
                cdn.mirror(deps, [entry(body)])
                self.assertEqual(len(calls), 1, "a file already there with its digest is not fetched again")
                with self.assertRaisesRegex(cdn.CdnError, "changed upstream"):
                    cdn.mirror(deps, [entry(b"other", "/1/b.js")])
        finally:
            cdn.fetch = original


if __name__ == "__main__":
    unittest.main()
