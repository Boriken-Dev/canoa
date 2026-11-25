"""Tests of the Download button's hook: how a release's files are sorted, and what the button says with and without one."""
from __future__ import annotations

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import docs_download as dd  # noqa: E402

BASE = "https://github.com/Boriken-Dev/canoa/releases/download/v1.0.0/"


def asset(name: str) -> dict:
    return {"name": name, "browser_download_url": BASE + name}


RELEASE = {
    "tag_name": "v1.0.0",
    "html_url": "https://github.com/Boriken-Dev/canoa/releases/tag/v1.0.0",
    "assets": [
        asset("CanoaUI-windows-arm64.exe"),
        asset("CanoaUI-windows-x64.exe"),
        asset("Canoa-linux-x86_64.AppImage"),
        asset("Canoa-linux-aarch64.AppImage"),
        asset("canoa-browser-extension.zip"),
        asset("CanoaUI-windows-x64.exe.sha256"),
        asset("SHA256SUMS"),
        asset("source.zip"),
    ],
}


class Classify(unittest.TestCase):
    def test_files_by_name(self):
        self.assertEqual(dd.classify("CanoaUI-windows-x64.exe"), ("windows", "x64"))
        self.assertEqual(dd.classify("CanoaUI-windows-arm64.exe"), ("windows", "arm64"))
        self.assertEqual(dd.classify("Canoa-linux-x86_64.AppImage"), ("linux", "x64"))
        self.assertEqual(dd.classify("Canoa-linux-aarch64.AppImage"), ("linux", "arm64"))
        self.assertEqual(dd.classify("CanoaUI.exe"), ("windows", None))
        self.assertEqual(dd.classify("canoa-browser-extension.zip"), ("extension", None))

    def test_checksums_and_sources_are_not_downloads(self):
        for name in ("CanoaUI-windows-x64.exe.sha256", "SHA256SUMS", "source.zip", "notes.md", "CanoaUI.exe.sig"):
            self.assertIsNone(dd.classify(name), name)


class Options(unittest.TestCase):
    def test_order_and_links(self):
        keys = [o["key"] for o in dd.options(RELEASE)]
        self.assertEqual(keys, ["windows-x64", "windows-arm64", "linux-x64", "linux-arm64", "extension"])
        self.assertTrue(all(o["url"].startswith(BASE) for o in dd.options(RELEASE)))

    def test_a_file_at_another_address_is_not_linked(self):
        release = {"assets": [{"name": "CanoaUI.exe", "browser_download_url": "https://example.com/CanoaUI.exe"}]}
        self.assertEqual(dd.options(release), [])


class Render(unittest.TestCase):
    def test_with_a_release_every_file_is_linked_and_the_main_button_has_a_live_link(self):
        out = dd.render(RELEASE, "Boriken-Dev/canoa", "Canoa")
        self.assertIn('data-release="v1.0.0"', out)
        for o in dd.options(RELEASE):
            self.assertIn(f'href="{o["url"]}" data-platform="{o["key"]}"', out)
        self.assertIn('class="md-button md-button--primary canoa-download__main" href="https://github.com/Boriken-Dev/canoa/releases/tag/v1.0.0"', out)
        self.assertNotIn("\n\n", out, "a blank line would end the Markdown HTML block")

    def test_without_a_release_it_says_so_and_links_no_file(self):
        out = dd.render(None, "Boriken-Dev/canoa", "Canoa")
        self.assertIn('data-release=""', out)
        self.assertIn("has not published a release yet", out)
        self.assertIn("Build from source", out)
        self.assertNotIn("releases/download", out)
        self.assertNotIn("data-platform", out)
        self.assertNotIn("\n\n", out)

    def test_a_release_with_no_matching_file_still_offers_the_release_page_and_the_build(self):
        out = dd.render({"tag_name": "v2", "assets": []}, "Boriken-Dev/canoa", "Canoa")
        self.assertIn("All files of this release", out)
        self.assertIn("Build from source", out)


class Source(unittest.TestCase):
    def test_none_means_no_release_and_no_network(self):
        import os

        before = os.environ.get("CANOA_DOCS_RELEASE")
        os.environ["CANOA_DOCS_RELEASE"] = "none"
        try:
            self.assertIsNone(dd._fetch("Boriken-Dev/canoa"))
        finally:
            if before is None:
                del os.environ["CANOA_DOCS_RELEASE"]
            else:
                os.environ["CANOA_DOCS_RELEASE"] = before

    def test_repo_from_the_site_url(self):
        self.assertEqual(dd._repo("https://github.com/Boriken-Dev/canoa"), "Boriken-Dev/canoa")
        self.assertEqual(dd._repo("https://example.com/x"), "")


if __name__ == "__main__":
    unittest.main()
