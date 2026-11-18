"""canoa_bitwarden against a stand-in `bw`: what it asks, what it answers, and that no value reaches a failure.

Run with Guatiao's Python binding on PYTHONPATH: `python -m unittest discover -s tests` from the extension's folder. The two tests that
describe the library as a host reads it need Guatiao's core library too (GUATIAO_LIBRARY) and are skipped without it; the host's own
test loads the library through the Python source and covers them.
"""

import json
import os
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.dirname(HERE))
sys.path.insert(0, os.path.join(os.path.dirname(HERE), "..", "..", "..", "canoa", "python"))

from canoa_bitwarden import Bitwarden, BitwardenConfig  # noqa: E402
from canoa_secret_kinds import SecretProvider  # noqa: E402

from guatiao import provide  # noqa: E402
from guatiao.errors import BadValue, Gone, GuatiaoError, Unsupported  # noqa: E402

STANDIN = os.path.join(HERE, "bw_standin.py")
GOOD = "good-session-key-made-up"


def provider(session=GOOD, part="password", log=None, command=None, **more):
    if log is not None:
        os.environ["BW_STANDIN_LOG"] = log
    return Bitwarden(
        BitwardenConfig(
            command=command or sys.executable,
            arguments=[STANDIN] if command is None else [],
            part=part,
            session=session,
            **more,
        )
    )


class Reads(unittest.TestCase):
    def test_a_part_of_an_item_is_the_value_and_the_line_break_bw_adds_is_not(self):
        self.assertEqual(provider().get({"name": "github"}), {"value": "gh-pw-zq9-made-up"})
        self.assertEqual(provider(part="username").get({"name": "github"}), {"value": "ana"})
        self.assertEqual(provider().get({"name": "two lines"}), {"value": "line one\nline two"})

    def test_an_item_that_is_not_there_or_has_no_such_part_is_no_value_and_no_failure(self):
        self.assertEqual(provider().get({"name": "ghost"}), {})
        self.assertEqual(provider(part="totp").get({"name": "empty"}), {})
        self.assertEqual(provider().get({"name": "empty"}), {})

    def test_the_answer_carries_no_end_because_bitwarden_items_have_none(self):
        self.assertNotIn("expires_in", provider().get({"name": "github"}))

    def test_names_are_listed_once_each_and_nothing_else(self):
        names = provider().names()
        self.assertEqual(names, {"names": ["empty", "github", "two lines"]})
        self.assertNotIn("x", json.dumps(names["names"]))

    def test_it_describes_a_reader_that_lists_and_needs_no_person(self):
        info = provider().describe()
        self.assertEqual(info["title"], "Bitwarden")
        self.assertTrue(info["can_list"])
        self.assertFalse(info["can_store"])
        self.assertFalse(info["needs_person"])


class Locked(unittest.TestCase):
    def test_a_wrong_or_missing_session_key_is_gone_and_the_message_holds_no_key(self):
        for session in ("not-the-key-zq9", None):
            with self.assertRaises(Gone) as caught:
                provider(session=session).get({"name": "github"})
            text = str(caught.exception)
            self.assertNotIn("not-the-key-zq9", text)
            self.assertIn("locked", text)

    def test_a_program_that_is_not_there_is_gone_and_says_what_is_needed(self):
        with self.assertRaises(Gone) as caught:
            provider(command="no-such-program-bw-zq9").get({"name": "github"})
        self.assertIn("Bitwarden's command line is needed", str(caught.exception))

    def test_a_command_that_does_not_answer_in_time_is_gone(self):
        slow = os.path.join(HERE, "bw_slow.py")
        with open(slow, "w", encoding="utf-8") as out:
            out.write("import time\ntime.sleep(5)\n")
        try:
            with self.assertRaises(Gone) as caught:
                Bitwarden(BitwardenConfig(command=sys.executable, arguments=[slow], timeout_seconds=1)).get({"name": "x"})
            self.assertIn("in time", str(caught.exception))
        finally:
            os.remove(slow)


class Safe(unittest.TestCase):
    def tearDown(self):
        os.environ.pop("BW_STANDIN_LOG", None)

    def test_the_session_key_goes_in_the_environment_and_never_on_a_command_line(self):
        with tempfile.TemporaryDirectory() as folder:
            log = os.path.join(folder, "calls.jsonl")
            provider(log=log).get({"name": "github"})
            provider(log=log).names()
            with open(log, encoding="utf-8") as read:
                calls = [json.loads(line) for line in read]
        self.assertEqual(len(calls), 2)
        for call in calls:
            self.assertTrue(call["session_in_env"])
            self.assertFalse(any(GOOD in word for word in call["words"]), call)
            self.assertIn("--nointeraction", call["words"])

    def test_a_request_for_a_name_that_is_no_name_is_refused_before_bw_is_run(self):
        with tempfile.TemporaryDirectory() as folder:
            log = os.path.join(folder, "calls.jsonl")
            for body in ({"name": ""}, {"name": "a\nb"}, {"name": "x" * 300}, {"name": 3}, {"name": "a", "more": 1}, {}):
                with self.assertRaises(BadValue):
                    provider(log=log).get(body)
            self.assertFalse(os.path.exists(log))

    def test_it_keeps_nothing_and_forgets_nothing(self):
        for call, body in ((provider().put, {"name": "a", "value": "v"}), (provider().forget, {"name": "a"})):
            with self.assertRaises(Unsupported):
                call(body)


@unittest.skipUnless(os.environ.get("GUATIAO_LIBRARY"), "describing a library needs Guatiao's core library: GUATIAO_LIBRARY")
class Script(unittest.TestCase):
    def test_it_is_a_library_of_one_secret_provider_a_host_can_instantiate_with_the_settings_it_declares(self):
        library = provide.library_of(sys.modules["canoa_bitwarden"])
        self.assertIn("secret_provider", library.kinds)
        self.assertEqual(library.providers, (Bitwarden,))
        spec = Bitwarden.__guatiao_provider__
        self.assertEqual(spec.id, "bitwarden")
        self.assertTrue(spec.builds_instances)
        self.assertEqual([kind.name for kind in spec.kinds], ["secret_provider"])
        self.assertEqual(SecretProvider.kind.name, "secret_provider")

    def test_the_session_setting_is_declared_secret_so_a_form_hides_it(self):
        schema = Bitwarden.__guatiao_provider__.schema
        schema = schema.to_python() if hasattr(schema, "to_python") else schema
        session = schema["properties"]["session"]
        self.assertTrue(session.get("writeOnly") or session.get("x-sensitive"), session)
        # Every setting has a default, so a description needs none.
        self.assertEqual(schema.get("required", []), [])


if __name__ == "__main__":
    unittest.main()
