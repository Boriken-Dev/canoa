"""The SDK's classes as Guatiao reads them: the descriptions, a form carried in `meta`, and a slot left in place refused.

Needs a build of Guatiao's C surface: `GUATIAO_LIBRARY` names it (a directory, or a file that exports it, as the Python source
`guatiao-source-py` is).
"""

from __future__ import annotations

import pathlib
import subprocess
import sys
from dataclasses import dataclass
from typing import Annotated, List

import pytest

import canoa_sdk as sdk
from canoa_sdk import forms
from guatiao import provide

ROOT = pathlib.Path(__file__).resolve().parents[1]


class Quill(sdk.AgentVendor):
    agent = "quill"
    title = "Quill"
    vendor = "quill-labs"
    standards = [sdk.StandardTitle("quill-key", "Quill key")]
    modes = [sdk.ModeTitle("ask", "Ask")]


def test_a_vendor_answers_its_description_from_its_class_attributes() -> None:
    assert Quill().describe() == {
        "agent": "quill",
        "title": "Quill",
        "vendor": "quill-labs",
        "standards": [{"id": "quill-key", "title": "Quill key"}],
        "names": [],
        "markers": [],
        "modes": [{"id": "ask", "title": "Ask"}],
    }


def test_a_vendor_can_compute_its_description_instead() -> None:
    class Dynamic(sdk.AgentVendor):
        def info(self) -> sdk.VendorInfo:
            return sdk.VendorInfo("dyn", "Dynamic", mark="<svg/>")

    assert Dynamic().describe()["mark"] == "<svg/>"


def test_an_extension_lists_its_types_and_leaves_an_absent_icon_out() -> None:
    class Notes(sdk.StoreExtension):
        title = "Notes"
        publisher = "Example"
        types = [sdk.ExtensionType("notes", "Notes")]

    assert Notes().describe() == {
        "title": "Notes",
        "publisher": "Example",
        "types": [{"id": "notes", "title": "Notes", "stores": []}],
    }


@dataclass
class Settings:
    command: Annotated[List[str], sdk.ui(section="run", placeholder="whisper-cli")]
    language: Annotated[str, sdk.ui(section="run")] = "en"
    translate: bool = False
    beam: Annotated[int, sdk.ui(section="run", unit="beams", visible_when=("translate", True))] = 5


def test_a_form_is_written_on_the_dataclass() -> None:
    assert sdk.form_of(Settings, [sdk.section("run", "Run", "How it runs")]) == {
        "sections": [{"id": "run", "title": "Run", "description": "How it runs"}],
        "fields": {
            "command": {"placeholder": "whisper-cli"},
            "beam": {"unit": "beams", "visibleWhen": {"field": "translate", "equals": True}},
        },
    }


def test_a_section_a_field_names_but_the_form_does_not_list_is_listed_by_its_id() -> None:
    assert sdk.form_of(Settings)["sections"] == [{"id": "run", "title": "run"}]


def test_a_provider_given_a_form_carries_it_in_its_meta() -> None:
    class Speak(sdk.SpeechProvider, form=Settings):
        title = "Speak"
        encodings = ["pcm_s16le"]

        def __init__(self, settings: Settings) -> None:
            self.settings = settings

        def open(self, request, spawner):  # type: ignore[no-untyped-def]
            raise NotImplementedError

    spec = Speak.__guatiao_provider__
    assert spec.meta == {"form": sdk.form_of(Settings)}
    assert spec.schema["properties"]["command"]["x-section"] == "run"
    assert spec.schema["required"] == ["command"]
    assert provide.library("speak", "1", providers=[Speak]).kinds == ("speech_provider",)


def test_a_class_that_leaves_an_abstract_slot_in_place_is_refused_at_creation() -> None:
    with pytest.raises(TypeError, match="Silent does not implement open"):

        class Silent(sdk.SpeechProvider):
            title = "Silent"
            encodings = ["opus"]


def test_a_base_with_abstract_slots_is_a_base_for_others() -> None:
    class Middle(sdk.SpeechProvider, abstract=True):
        encodings = ["opus"]

    class Leaf(Middle):
        title = "Leaf"

        def open(self, request, spawner):  # type: ignore[no-untyped-def]
            raise NotImplementedError

    assert Leaf().describe()["encodings"] == ["opus"]


def test_the_kinds_module_is_what_the_descriptions_write() -> None:
    done = subprocess.run(
        [sys.executable, str(ROOT / "tools" / "write_kinds.py"), "--check"], capture_output=True, text=True, check=False
    )
    assert done.returncode == 0, done.stderr


def test_every_form_helper_is_exported() -> None:
    assert set(forms.__all__) <= set(dir(forms))
