"""Tests of tools/docs_brand.py and of the pages' placeholders: `python -m unittest tools/test_docs_brand.py` with mkdocs installed."""
import json
import os
import re
import shutil
import sys
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace

sys.path.insert(0, str(Path(__file__).resolve().parent))
import docs_brand as hook  # noqa: E402

ROOT = hook.ROOT
SAMPLE = """# {{ name }} on this computer

{{ executable }} reads {{ config_file }} from /etc/{{ folder }}/{{ config_file }}, and {{ env_prefix }}_CONFIG names it.
The home is ~/{{ home_dir }}. The unit is {{ program }}@NAME.service. The {{ short_name }} host is {{ id }}.
Windows: %APPDATA%\\{{ folder }}\\{{ config_file }}. Kept: {{ other }} and {{{ name }}}, crates/canoa-host, Boriken-Dev/canoa.
See [it](browser.md#{{ name }}-on-this-computer) and [the file](x.md#{{ config_file }}).
"""

# Lines of `docs/*.md` that name the product literally on purpose: (page, reason). The whole page is exempt.
ALLOWED = [
    ("themes_and_brands.md", "describes Canoa's own brand file and its default values"),
]

# What names the product and so is written as a placeholder: the name as a word, the file, the variables, the folders.
LITERAL_PRODUCT = re.compile(
    r"(?<![\w-])Canoa(?![A-Za-z0-9_])"
    r"|(?<![\w-])CanoaUI(?![\w-])"
    r"|(?<![\w./-])canoa\.cfg(?![\w-])"
    r"|(?<![\w-])CANOA_(?:CONFIG|INSTALLED_DIR|HOST)\b"
    r"|(?<![\w.])\.canoa(?![\w-])"
    r"|(?<![\w-])canoa@"
    r"|/(?:etc|opt|run)/canoa(?![\w-])"
)


def config():
    return SimpleNamespace(
        site_name="Canoa", theme={}, extra_css=[], docs_dir=str(ROOT / "docs"), site_dir="site",
        use_directory_urls=True, plugins=SimpleNamespace(_current_plugin=None),
    )


class DocsBrand(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.saved = os.environ.pop("BRAND_FILE", None)
        self.addCleanup(lambda: os.environ.__setitem__("BRAND_FILE", self.saved) if self.saved else os.environ.pop("BRAND_FILE", None))

    def brand(self, **fields) -> Path:
        shutil.copy(ROOT / "app" / "assets" / "icons" / "canoa.svg", self.tmp / "mark.svg")
        data = {"name": "AgentUI", "short_name": "AgentUI", "executable": "AgentUI", "id": "dev.example.agentui",
                "env_prefix": "AGENTUI", "icon": "mark.svg", "accent": "#3D6DAF", "accent_on_dark": "#7FA8E6",
                "program": "agents", "config_file": "agents.cfg", "folder": "agents", "home_dir": ".agentui", **fields}
        path = self.tmp / "brand.json"
        path.write_text(json.dumps(data), encoding="utf-8")
        return path

    def render(self, markdown: str) -> str:
        return hook.on_page_markdown(markdown, SimpleNamespace(meta={}), None, None)

    def test_the_default_brand_renders_canoas_names(self):
        cfg = config()
        hook.on_config(cfg)
        self.assertEqual(cfg.site_name, "Canoa")
        self.assertNotIn("favicon", cfg.theme)
        self.assertEqual(cfg.theme["logo"], "assets/logo.svg")
        out = self.render(SAMPLE)
        self.assertIn("# Canoa on this computer", out)
        self.assertIn("CanoaUI reads canoa.cfg from /etc/Canoa/canoa.cfg, and CANOA_CONFIG names it.", out)
        self.assertIn("The home is ~/.canoa. The unit is canoa@NAME.service. The Canoa host is dev.boriken.canoa.", out)
        self.assertIn("%APPDATA%\\Canoa\\canoa.cfg", out)

    def test_another_brand_renames_the_site_and_the_names(self):
        os.environ["BRAND_FILE"] = str(self.brand())
        cfg = config()
        hook.on_config(cfg)
        self.assertEqual(cfg.site_name, "AgentUI")
        self.assertEqual(cfg.theme["favicon"], "assets/favicon.svg")
        out = self.render(SAMPLE)
        self.assertIn("# AgentUI on this computer", out)
        self.assertIn("AgentUI reads agents.cfg from /etc/agents/agents.cfg, and AGENTUI_CONFIG names it.", out)
        self.assertIn("The home is ~/.agentui. The unit is agents@NAME.service. The AgentUI host is dev.example.agentui.", out)
        self.assertIn("%APPDATA%\\agents\\agents.cfg", out)

    def test_a_link_follows_the_heading_it_names(self):
        os.environ["BRAND_FILE"] = str(self.brand())
        hook.on_config(config())
        out = self.render(SAMPLE)
        self.assertIn("[it](browser.md#agentui-on-this-computer)", out)
        self.assertIn("[the file](x.md#agentscfg)", out)

    def test_an_unknown_placeholder_and_a_triple_brace_stay_as_written(self):
        os.environ["BRAND_FILE"] = str(self.brand())
        hook.on_config(config())
        out = self.render(SAMPLE)
        self.assertIn("Kept: {{ other }} and {{ name }}, crates/canoa-host, Boriken-Dev/canoa.", out)

    def test_names_default_from_the_program(self):
        os.environ["BRAND_FILE"] = str(self.brand(config_file="", folder="", home_dir=""))
        hook.on_config(config())
        self.assertEqual(self.render("{{ config_file }} in /etc/{{ folder }}, ~/{{ home_dir }}"), "agents.cfg in /etc/AgentUI, ~/.agents")

    def test_the_logo_takes_the_accent_and_the_dark_accent(self):
        os.environ["BRAND_FILE"] = str(self.brand())
        hook.on_config(config())
        made = []
        files = SimpleNamespace(append=made.append, get_file_from_path=lambda p: None, remove=lambda f: None)
        hook.on_files(files, config())
        content = {f.src_uri: f.content_string for f in made}
        self.assertIn('fill="#3D6DAF"', content["assets/logo.svg"])
        self.assertIn('fill="#7FA8E6"', content["assets/logo-dark.svg"])
        self.assertIn("<title>AgentUI</title>", content["assets/logo.svg"])
        self.assertIn("--md-default-bg-color: #262523", content["assets/brand.css"])

    def test_no_page_names_the_product_literally(self):
        allowed = {page for page, _ in ALLOWED}
        found = []
        for doc in sorted((ROOT / "docs").glob("*.md")):
            if doc.name in allowed:
                continue
            for number, line in enumerate(doc.read_text(encoding="utf-8").split("\n"), 1):
                match = LITERAL_PRODUCT.search(line)
                if match:
                    found.append(f"{doc.name}:{number}: {match.group(0)!r}")
        self.assertEqual(found, [], "write the product's names as placeholders (docs/themes_and_brands.md), or allow the page in ALLOWED")


if __name__ == "__main__":
    unittest.main()
