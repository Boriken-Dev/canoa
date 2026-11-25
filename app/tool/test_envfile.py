import tempfile
import unittest
from pathlib import Path

import envfile


class EnvTests(unittest.TestCase):
    def test_env_file_is_read_with_comments_quotes_and_process_variables(self):
        text = (
            "# a comment\n"
            "FLUTTER_ROOT=${HOME_DIR}/flutter\n"
            "export PUB_CACHE=\"/pub cache\"  \n"
            "CARGO_HOME='${HOME_DIR}/literal'\n"
            "CANOA_DEPS=/deps # trailing\n"
            "OTHER=ignored\n"
            "not a line\n"
        )
        values = envfile.parse_env(text, {"HOME_DIR": "/home"})
        self.assertEqual(values["FLUTTER_ROOT"], "/home/flutter")
        self.assertEqual(values["PUB_CACHE"], "/pub cache")
        self.assertEqual(values["CARGO_HOME"], "${HOME_DIR}/literal")
        self.assertEqual(values["CANOA_DEPS"], "/deps")
        self.assertNotIn("OTHER", values)

    def test_the_process_environment_overrides_the_env_file(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / ".env").write_text("PUB_CACHE=/from_file\nFLUTTER_ROOT=/sdk\n")
            settings = envfile.load_settings(root, {"PUB_CACHE": "/from_process", "UNRELATED": "x"})
            self.assertEqual(settings, {"PUB_CACHE": "/from_process", "FLUTTER_ROOT": "/sdk"})
            self.assertEqual(envfile.load_settings(root / "none", {}), {})

    def test_settings_reach_the_environment(self):
        environ = {"PATH": "/usr/bin"}
        envfile.apply_settings(
            {"FLUTTER_ROOT": "/sdk", "PUB_CACHE": "/pc", "CARGO_HOME": "/cargo"}, environ,
        )
        self.assertEqual(environ["PUB_CACHE"], "/pc")
        self.assertEqual(environ["CARGO_HOME"], "/cargo")
        parts = environ["PATH"].split(envfile.os.pathsep)
        self.assertEqual(parts[0], str(Path("/sdk") / "bin"))
        self.assertEqual(parts[1], str(Path("/cargo") / "bin"))
        self.assertEqual(parts[-1], "/usr/bin")


    def test_the_deps_checkout_reaches_the_environment_for_the_shared_tool(self):
        environ = {"PATH": "/usr/bin"}
        envfile.apply_settings({"CANOA_DEPS": "/deps"}, environ)
        self.assertEqual(environ["CANOA_DEPS"], "/deps")
        self.assertEqual(environ["PATH"], "/usr/bin")


if __name__ == "__main__":
    unittest.main()
