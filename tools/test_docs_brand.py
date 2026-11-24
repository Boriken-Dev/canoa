"""Tests of tools/docs_brand.py: `python -m unittest tools/test_docs_brand.py` with mkdocs installed."""
import json
import os
import shutil
import sys
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace

sys.path.insert(0, str(Path(__file__).resolve().parent))
import docs_brand as hook  # noqa: E402

ROOT = hook.ROOT
SAMPLE = """# Canoa on this computer

CanoaUI reads canoa.cfg from /etc/Canoa/canoa.cfg or ~/.config/Canoa/canoa.cfg, and CANOA_CONFIG names it.
The home is ~/.canoa. The unit is canoa@NAME.service, in /opt/canoa and /var/lib/canoa-portal.
Kept: crates/canoa-host, Boriken-Dev/canoa, dev.boriken.canoa/account, https://boriken-dev.github.io/canoa/.
See [it](browser.md#canoa-on-this-computer).
"""


def config(_name: str):
    holder = SimpleNamespace(site_name="Canoa", theme={}, extra_css=[], docs_dir=str(ROOT / "docs"), site_dir="site", use_directory_urls=True, plugins=SimpleNamespace(_current_plugin=None))
    return holder


def page(markdown: str, **meta):
    return SimpleNamespace(meta=meta), markdown


class DocsBrand(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.saved = os.environ.pop("BRAND_FILE", None)
        self.addCleanup(lambda: os.environ.__setitem__("BRAND_FILE", self.saved) if self.saved else os.environ.pop("BRAND_FILE", None))

    def brand(self, **fields) -> Path:
        icon = ROOT / "app" / "assets" / "icons" / "canoa.svg"
        shutil.copy(icon, self.tmp / "mark.svg")
        data = {"name": "AgentUI", "short_name": "AgentUI", "executable": "AgentUI", "id": "dev.example.agentui",
                "env_prefix": "AGENTUI", "icon": "mark.svg", "accent": "#3D6DAF", "accent_on_dark": "#7FA8E6",
                "program": "agents", "config_file": "agents.cfg", "folder": "agents", "home_dir": ".agentui", **fields}
        path = self.tmp / "brand.json"
        path.write_text(json.dumps(data), encoding="utf-8")
        return path

    def substitute(self, markdown: str, **meta) -> str:
        page_, _ = page(markdown, **meta)
        return hook.on_page_markdown(markdown, page_, None, None)

    def test_the_default_brand_leaves_every_page_unchanged(self):
        hook.on_config(config("Canoa"))
        for doc in sorted((ROOT / "docs").glob("*.md")):
            text = doc.read_text(encoding="utf-8")
            self.assertEqual(self.substitute(text), text, doc.name)

    def test_the_default_brand_keeps_the_site_name_and_the_committed_favicon(self):
        cfg = config("Canoa")
        hook.on_config(cfg)
        self.assertEqual(cfg.site_name, "Canoa")
        self.assertNotIn("favicon", cfg.theme)
        self.assertEqual(cfg.theme["logo"], "assets/logo.svg")

    def test_another_brand_renames_the_site_and_the_tokens(self):
        os.environ["BRAND_FILE"] = str(self.brand())
        cfg = config("Canoa")
        hook.on_config(cfg)
        self.assertEqual(cfg.site_name, "AgentUI")
        self.assertEqual(cfg.theme["favicon"], "assets/favicon.svg")
        out = self.substitute(SAMPLE)
        self.assertIn("# AgentUI on this computer", out)
        self.assertIn("AgentUI reads agents.cfg from /etc/agents/agents.cfg or ~/.config/agents/agents.cfg, and AGENTUI_CONFIG names it.", out)
        self.assertIn("The home is ~/.agentui. The unit is agents@NAME.service, in /opt/agents and /var/lib/agents-portal.", out)
        self.assertIn("[it](browser.md#agentui-on-this-computer)", out)
        self.assertIn("crates/canoa-host, Boriken-Dev/canoa, dev.boriken.canoa/account, https://boriken-dev.github.io/canoa/.", out)

    def test_a_page_can_opt_out(self):
        os.environ["BRAND_FILE"] = str(self.brand())
        hook.on_config(config("Canoa"))
        self.assertEqual(self.substitute(SAMPLE, brand_names=False), SAMPLE)

    def test_names_default_from_the_program(self):
        os.environ["BRAND_FILE"] = str(self.brand(config_file="", folder="", home_dir=""))
        hook.on_config(config("Canoa"))
        out = self.substitute("canoa.cfg in /etc/Canoa, ~/.canoa")
        self.assertEqual(out, "agents.cfg in /etc/AgentUI, ~/.agents")

    def test_the_logo_takes_the_accent_and_the_dark_accent(self):
        os.environ["BRAND_FILE"] = str(self.brand())
        hook.on_config(config("Canoa"))
        files = SimpleNamespace(append=lambda f: made.append(f), get_file_from_path=lambda p: None, remove=lambda f: None)
        made = []
        hook.on_files(files, config("Canoa"))
        content = {f.src_uri: f.content_string for f in made}
        self.assertIn('fill="#3D6DAF"', content["assets/logo.svg"])
        self.assertIn('fill="#7FA8E6"', content["assets/logo-dark.svg"])
        self.assertIn("<title>AgentUI</title>", content["assets/logo.svg"])
        self.assertIn("--md-default-bg-color: #262523", content["assets/brand.css"])


if __name__ == "__main__":
    unittest.main()
