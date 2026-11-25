import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import office_js as oj

LOADER = (
    'var OSF=OSF||{};OSF.HostSpecificFileVersionMap={outlook:{web:"16.01",win32:"16.02"},'
    'word:{web:"16.00"},excel:{web:"16.00"},powerpoint:{web:"16.00"},onenote:{web:"16.00"}};'
    'OSF.SupportedLocales={"en-us":true,"ja-jp":true};OSF.AssociatedLocales={};'
)


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


if __name__ == "__main__":
    unittest.main()
