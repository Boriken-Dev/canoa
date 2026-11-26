"""The SDK's classes as Guatiao reads them: the descriptions, a form carried in `meta`, and a slot left in place refused.

Needs a build of Guatiao's C surface: `GUATIAO_LIBRARY` names it (a directory, or a file that exports it, as the Python source
`guatiao-source-py` is).
"""

from __future__ import annotations

import json
import pathlib
import subprocess
import sys
from dataclasses import dataclass, field
from typing import Annotated, Any, List

import pytest

import canoa_sdk as sdk
from canoa_sdk import forms
from guatiao import provide
from guatiao.errors import BadValue, Internal, NotFound, Unsupported

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
        "context_files": [],
        "home_context_files": [],
        "names": [],
        "markers": [],
        "modes": [{"id": "ask", "title": "Ask"}],
        "endpoints": [],
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


def test_an_extension_type_carries_the_catalogs_it_ships() -> None:
    class Skills(sdk.StoreExtension):
        title = "Skills"
        publisher = "Example"
        types = [
            sdk.ExtensionType(
                "skills",
                "Skills",
                catalogs=sdk.CatalogWords(
                    "catalog", "catalogs", "skills", defaults=[sdk.DefaultCatalog("main", "skills_source", pin="abc")]
                ),
            )
        ]

    assert Skills().describe()["types"][0]["catalogs"] == {
        "noun": "catalog",
        "plural": "catalogs",
        "lists": "skills",
        "defaults": [{"label": "main", "provider": "skills_source", "pin": "abc"}],
    }


def test_a_config_store_an_environment_and_an_sdk_provider_are_providers_of_their_kinds() -> None:
    class Memory(sdk.ConfigStore):
        def describe(self) -> Any:
            return {"title": "Memory", "layers": []}

        def read(self) -> Any:
            return {}

        def get(self, key: Any) -> Any:
            return {}

        def change(self, changes: Any) -> Any:
            return {}

        def revision(self) -> str:
            return "1"

    class Sandbox(sdk.Environment):
        def describe(self) -> Any:
            return {"title": "Sandbox"}

        def spawner(self) -> Any:
            raise Unsupported("no spawner here")

    class Client(sdk.SdkProvider):
        def describe(self) -> Any:
            return {"title": "Client"}

        def open(self, request: Any, http: Any) -> Any:
            return sdk.Sdk()

    assert provide.library("memory", "1", providers=[Memory]).kinds == ("config_store",)
    assert provide.library("sandbox", "1", providers=[Sandbox]).kinds == ("environment",)
    assert provide.library("client", "1", providers=[Client]).kinds == ("sdk_provider",)
    for call in (lambda: Sandbox().read_file({}), lambda: Sandbox().list_dir({}), lambda: Sandbox().call({})):
        with pytest.raises(Unsupported):
            call()
    for call in (lambda: Client().open({}, None).chat({}), lambda: Client().open({}, None).models()):
        with pytest.raises(Unsupported):
            call()

    with pytest.raises(TypeError, match="IncompleteStore does not implement"):

        class IncompleteStore(sdk.ConfigStore):
            pass


def test_an_account_speech_provider_lists_its_services_with_their_models() -> None:
    class Acme(sdk.AccountSpeech):
        services = [
            sdk.AccountSpeechService(
                "acme",
                "Acme",
                "acme-api",
                "https://api.acme.test/v1",
                "https://api.acme.test/v1/audio/transcriptions",
                "api_key",
                models=[sdk.AccountSpeechModel("m", "M")],
            )
        ]

    assert Acme().describe() == {
        "services": [
            {
                "id": "acme",
                "title": "Acme",
                "standard": "acme-api",
                "address": "https://api.acme.test/v1",
                "url": "https://api.acme.test/v1/audio/transcriptions",
                "credential": "api_key",
                "summary": "",
                "location": "",
                "models": [{"id": "m", "label": "M"}],
            }
        ]
    }
    assert provide.library("acme", "1", providers=[Acme]).kinds == ("account_speech",)


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


class Memory(sdk.SystemStoreSecretProvider):
    title = "Memory"

    def __init__(self) -> None:
        self.cells: dict = {}

    def read(self, key):  # type: ignore[no-untyped-def]
        return self.cells.get(key)

    def write(self, key, text):  # type: ignore[no-untyped-def]
        self.cells[key] = text

    def remove(self, key):  # type: ignore[no-untyped-def]
        self.cells.pop(key, None)


def test_a_secret_provider_answers_its_description_from_its_class_attributes() -> None:
    class Vault(sdk.SecretProvider):
        title = "Vault"
        can_list = True

        def get(self, request, spawner):  # type: ignore[no-untyped-def]
            return sdk.Held("v", expires_in=30).to_data()

    assert Vault().describe() == {"title": "Vault", "can_store": False, "can_list": True, "needs_person": False}
    assert Vault().get({"name": "n"}, None) == {"value": "v", "expires_in": 30}
    assert provide.library("vault", "1", providers=[Vault]).kinds == ("secret_provider",)


def test_a_secret_provider_refuses_what_it_does_not_say_it_can_do() -> None:
    class Vault(sdk.SecretProvider):
        title = "Vault"

        def get(self, request, spawner):  # type: ignore[no-untyped-def]
            return sdk.Held().to_data()

    from guatiao.errors import Unsupported

    for call in (lambda: Vault().names(None), lambda: Vault().put({"name": "n", "value": "v"}, None), lambda: Vault().forget({"name": "n"}, None)):
        with pytest.raises(Unsupported):
            call()


def test_a_secret_provider_that_leaves_get_in_place_is_refused_at_creation() -> None:
    with pytest.raises(TypeError, match="Mute does not implement get"):

        class Mute(sdk.SecretProvider):
            title = "Mute"


def test_the_form_provider_holds_nothing_and_needs_the_person() -> None:
    from guatiao.errors import Unsupported

    form = sdk.FormSecretProvider()
    assert form.describe()["needs_person"] is True
    assert form.get({"name": "token"}, None) == {}
    with pytest.raises(Unsupported):
        form.put({"name": "token", "value": "v"}, None)


def test_a_request_names_one_secret_checked_as_the_kind_says() -> None:
    from guatiao.errors import BadValue

    assert sdk.name_of({"name": "github_token"}) == "github_token"
    for bad in ({}, {"name": ""}, {"name": "a\nb"}, {"name": "n", "extra": 1}, {"name": 1}):
        with pytest.raises(BadValue):
            sdk.name_of(bad)
    assert sdk.keeping_of({"name": "n", "value": "v"}) == ("n", "v")
    big = "v" * (64 * 1024 + 1)
    assert sdk.keeping_of({"name": "n" * 300, "value": big}) == ("n" * 300, big)


def test_a_store_provider_keeps_secrets_with_an_index_of_their_names() -> None:
    store = Memory()
    assert store.describe()["can_store"] is True and store.describe()["can_list"] is True
    store.put({"name": "b", "value": "2"}, None)
    store.put({"name": "a", "value": "1"}, None)
    store.put({"name": "a", "value": "3"}, None)
    assert store.names(None) == {"names": ["a", "b"]}
    assert store.get({"name": "a"}, None) == {"value": "3"}
    assert store.get({"name": "missing"}, None) == {}
    store.forget({"name": "a"}, None)
    store.forget({"name": "a"}, None)
    assert store.names(None) == {"names": ["b"]}
    assert "s:a" not in store.cells


@dataclass
class RegionValues:
    name: Annotated[str, sdk.ui(section="where", placeholder="eu-west")]
    hosts: List[str] = field(default_factory=list)


class Region(sdk.EnvProfileItem):
    item = "region"
    title = "Region"
    help = "Where the session runs."
    harnesses = ["claude-code"]
    values = RegionValues
    variables = {"name": ["REGION"]}
    lines = "hosts"
    note = "puts the session in a region"


def test_a_profile_item_describes_the_values_it_takes_from_its_dataclass() -> None:
    described = Region().describe()
    assert described["id"] == "region" and described["harnesses"] == ["claude-code"]
    assert described["schema"]["required"] == ["name"]
    assert described["schema"]["properties"]["name"]["x-section"] == "where"
    assert described["form"]["fields"]["name"] == {"placeholder": "eu-west"}
    assert provide.library("region", "1", providers=[Region]).kinds == ("profile_item",)


def _apply(item: Any, values: dict, harness: str = "") -> Any:
    return item.apply({"harness": harness, "values": values})


def test_an_env_item_turns_text_members_and_name_value_lines_into_variables() -> None:
    done = _apply(Region(), {"name": " north ", "hosts": ["A=1", "B=x=y"]})
    assert done["env"] == [
        {"name": "REGION", "value": "north"},
        {"name": "A", "value": "1"},
        {"name": "B", "value": "x=y"},
    ]
    assert done["notes"] == ["puts the session in a region"]
    assert "network" not in done
    assert _apply(Region(), {}) == {"env": [], "args": [], "settings": [], "notes": []}
    with pytest.raises(BadValue, match="NAME=value"):
        _apply(Region(), {"hosts": ["A"]})
    with pytest.raises(BadValue):
        Region().apply({"harness": "", "values": "[1]"})


def test_a_setting_item_sets_one_of_the_sessions_own_settings_when_its_member_is_text() -> None:
    class Model(sdk.SettingProfileItem):
        item = "model"
        title = "Model"
        setting = "model"
        note = "sets the model the agent starts with"

    assert _apply(Model(), {"model": "opus"})["settings"] == [{"key": "model", "value": "opus"}]
    assert _apply(Model(), {})["settings"] == []


def test_a_network_item_names_the_routes_the_session_leaves_by() -> None:
    class Network(sdk.NetworkProfileItem):
        item = "network"
        title = "Network"

    assert _apply(Network(), {})["network"] == {"routes": [], "bypass": [], "intercept": []}
    own = _apply(
        Network(),
        {
            "routes": [{"route": "gateway", "hosts": ["*.internal.example"]}, {"route": " corp "}],
            "bypass": ["a.example"],
        },
    )
    assert own["network"]["routes"] == [
        {"route": "gateway", "hosts": ["*.internal.example"]},
        {"route": "corp", "hosts": []},
    ]
    assert own["network"]["bypass"] == ["a.example"]
    assert "gateway, corp" in own["notes"][0]
    ended = _apply(Network(), {"client_certificate_hosts": [{"pattern": "a.example"}]})
    assert ended["network"]["intercept"] == [{"pattern": "a.example", "identity": "system"}]
    assert "a.example" in ended["notes"][1]
    for values in ({"routes": "corp"}, {"routes": [{"hosts": []}]}, {"routes": [{"route": "a"}, {"route": "b"}]}):
        with pytest.raises(BadValue):
            _apply(Network(), values)


class _Process:
    """A program the stand-in spawner started: what it prints, what it says on its error stream and how it ended."""

    def __init__(self, out: bytes = b"", err: bytes = b"", code: int = 0, hang: bool = False) -> None:
        self.out, self.err, self.code, self.hang = out, err, code, hang
        self.input_closed = self.killed = self.destroyed = False

    @staticmethod
    def _give(source: bytes, buffer: bytearray) -> int:
        size = min(len(buffer), len(source))
        buffer[:size] = source[:size]
        return size

    def read(self, buffer: bytearray, wait: int) -> int:
        if self.hang:
            return -1
        size = self._give(self.out, buffer)
        self.out = self.out[size:]
        return size

    def read_error(self, buffer: bytearray, wait: int) -> int:
        size = self._give(self.err, buffer)
        self.err = self.err[size:]
        return size

    def close_input(self) -> None:
        self.input_closed = True

    def wait(self, timeout: int) -> dict:
        return {"running": False, "code": self.code}

    def kill(self) -> None:
        self.killed = True

    def destroy(self) -> None:
        self.destroyed = True


class _Spawner:
    def __init__(self, process: Any = None, files: Any = None) -> None:
        self.process, self.files, self.specs, self.destroyed = process, files or {}, [], False

    def spawn(self, spec: dict) -> Any:
        self.specs.append(spec)
        if self.process is None:
            raise NotFound("no such program")
        return self.process

    def read_file(self, path: str, limit: int) -> bytes:
        if path not in self.files:
            raise NotFound("no such file")
        return self.files[path]

    def destroy(self) -> None:
        self.destroyed = True


class _Fetch:
    def __init__(self) -> None:
        self.destroyed = False

    def destroy(self) -> None:
        self.destroyed = True


def _read(source: Any, fields: dict, spawner: _Spawner) -> str:
    return source.read({"kind": "overlays", "label": "x", "fields": json.dumps(fields)}, spawner, _Fetch())


class Command(sdk.CommandCatalogSource):
    pass


def test_the_command_source_answers_what_the_program_prints_and_lets_go_of_what_it_was_handed() -> None:
    process = _Process(out=b"name = made\n")
    spawner = _Spawner(process)
    assert _read(Command(), {"command": ["tool", "--flag"], "cwd": "/work"}, spawner) == "name = made\n"
    assert spawner.specs == [{"program": "tool", "args": ["--flag"], "error_output": "separate", "cwd": "/work"}]
    assert process.input_closed and process.destroyed and spawner.destroyed
    description = Command().describe()
    assert description["id"] == "command" and description["types"] == []
    assert description["schema"]["required"] == ["command"]
    assert description["form"] == {"fields": {"timeout": {"unit": "s"}}}


def _fails(process: Any, fields: Any = None) -> str:
    spawner = _Spawner(process)
    with pytest.raises(Internal) as raised:
        _read(Command(), fields or {"command": ["tool"]}, spawner)
    assert process is None or process.destroyed or not spawner.specs
    assert spawner.destroyed
    return str(raised.value)


def test_the_command_source_says_why_a_program_gave_no_catalog() -> None:
    assert "it broke here" in _fails(_Process(err=b"it broke here\n", code=3))
    assert "ended with" in _fails(_Process(code=3))
    assert "printed no catalog" in _fails(_Process())
    assert "is not text" in _fails(_Process(out=b"n\xff\xfea"))
    assert "was stopped after 0 s" in _fails(_Process(hang=True), {"command": ["tool"], "timeout": 0})
    assert "could not start" in _fails(None)
    for fields in ({}, {"command": []}, {"command": ["tool"], "timeout": "soon"}):
        assert _fails(_Process(), fields)
    named = _Spawner(_Process(), {"/w/out.toml": b"from a file"})
    assert _read(Command(), {"command": ["t"], "cwd": "/w", "file": "out.toml"}, named) == "from a file"


def test_the_read_source_reads_a_file_through_the_spawner_and_says_when_it_cannot() -> None:
    class Read(sdk.ReadCatalogSource):
        pass

    spawner = _Spawner(files={"/here/catalog.toml": b'name = "here"\n'})
    assert _read(Read(), {"source": "/here/catalog.toml"}, spawner) == 'name = "here"\n'
    assert spawner.destroyed
    with pytest.raises(Internal, match="cannot be read"):
        _read(Read(), {"source": "/nowhere"}, _Spawner())
    with pytest.raises(BadValue):
        _read(Read(), {}, _Spawner())
    assert Read().describe()["id"] == "read"


def test_a_settings_section_and_a_panel_tab_describe_themselves_from_their_class_attributes() -> None:
    class Region(sdk.SettingsSection):
        section = "region"
        title = "Region"
        under = "network"
        stored = "provider"
        values = RegionValues

    described = Region().describe()
    assert described["id"] == "region" and described["under"] == "network" and described["stored"] == "provider"
    assert described["schema"]["required"] == ["name"] and Region().read() == {}
    assert provide.library("sections", "1", providers=[Region]).kinds == ("settings_section",)

    class Builds(sdk.LogPanelTab):
        tab = "builds"
        title = "Builds"

        def lines(self) -> List[str]:
            return ["one", "two", "three"]

    assert Builds().describe() == {"id": "builds", "title": "Builds", "shows": "log"}
    assert Builds().tail({"lines": 2}) == "two\nthree\n" and Builds().tail({"lines": 0}) == ""
    assert Builds().tail({"lines": 10}) == "one\ntwo\nthree\n"

    class Shell(sdk.TerminalPanelTab):
        tab = "dev"
        title = "Dev shell"
        shell = "git-bash"

    assert Shell().describe()["shows"] == "terminal" and Shell().tail({"lines": 5}) == ""
    assert provide.library("tabs", "1", providers=[Builds, Shell]).kinds == ("panel_tab",)


def test_a_panel_tab_can_show_a_page_at_an_address() -> None:
    class Docs(sdk.PagePanelTab):
        tab = "docs"
        title = "Docs"
        icon = "book"
        url = "http://127.0.0.1:4100/docs"

    assert Docs().describe() == {
        "id": "docs",
        "title": "Docs",
        "icon": "book",
        "shows": "page",
        "url": "http://127.0.0.1:4100/docs",
    }
    assert Docs().tail({"lines": 5}) == ""

    class Plain(sdk.PanelTab):
        tab = "plain"
        title = "Plain"
        shows = "page"
        url = "https://localhost/"

    assert Plain().describe()["shows"] == "page" and Plain().describe()["url"] == "https://localhost/"
    # A tab that shows no page leaves the address out of its description.
    class Shell(sdk.TerminalPanelTab):
        tab = "dev"
        title = "Dev shell"

    assert "url" not in Shell().describe()
    assert provide.library("tabs", "1", providers=[Docs, Plain]).kinds == ("panel_tab",)


def test_a_catalog_source_is_the_provider_of_the_id_it_is_described_under() -> None:
    class Team(sdk.CatalogSource):
        source = "team"
        title = "Team"

        def read(self, request: Any, spawner: Any, fetch: Any) -> str:
            return ""

    assert Team.__guatiao_provider__.id == "team"
    assert Command.__guatiao_provider__.id == "command"
    assert provide.library("sources", "1", providers=[Team, Command]).kinds == ("catalog_source",)


class Members:
    """The host's process table, as a handle answers: a tree that is read and closed."""

    class Answer:
        def __init__(self, ids: List[int]) -> None:
            self.ids = ids

        def __enter__(self) -> "Members.Answer":
            return self

        def __exit__(self, *_: object) -> None:
            return None

        def to_python(self) -> List[int]:
            return self.ids

    def members(self, root: int) -> "Members.Answer":
        return Members.Answer([root, root + 1])


class Quota(sdk.UsageReading):
    def read(self, root: int, tree: bool):  # type: ignore[no-untyped-def]
        if root + 1 not in self.processes(root):
            return sdk.failed("not a member", sdk.Retry.NEVER)
        return sdk.reading([sdk.bucket("q:five_hour", remaining=750, resets_at=1893456000, observed_at=1)])


class QuillUsage(sdk.UsageReader):
    reader = "quill"
    bucket_prefix = "q:"
    settle_ms = 100
    retries_ms = [2000, 4000]

    def open(self, members):  # type: ignore[no-untyped-def]
        return Quota(members)


def test_a_usage_reader_answers_its_description_from_its_class_attributes() -> None:
    assert QuillUsage().describe() == {
        "id": "quill",
        "bucket_prefix": "q:",
        "settle_ms": 100,
        "retries_ms": [2000, 4000],
        "cache_ms": 15000,
        "private_limit_ms": 40000,
        "private_retries_ms": [],
        "stored": False,
    }
    assert provide.library("quill", "1", providers=[QuillUsage]).kinds == ("usage_reader",)


def test_a_reading_asks_the_host_for_the_agents_processes_and_answers_buckets_as_plain_data() -> None:
    answer = QuillUsage().open(Members()).read(10, False)
    assert answer == {
        "buckets": [
            {
                "id": "q:five_hour",
                "models": {"kind": "unknown"},
                "unit": "permille",
                "remaining": 750,
                "refill": {"kind": "fixed_window", "resets_at": 1893456000},
                "status": "unknown",
                "observed_at": 1,
                "source": "reported",
            }
        ]
    }


def test_a_failed_reading_says_whether_to_ask_again() -> None:
    assert sdk.failed("no port", sdk.Retry.NOT_YET) == {"buckets": [], "failure": {"why": "no port", "retry": "not_yet"}}
    assert sdk.failed("gone")["failure"]["retry"] == "never"


def test_a_usage_reader_that_leaves_open_in_place_is_refused_at_creation() -> None:
    with pytest.raises(TypeError, match="Mute does not implement open"):

        class Mute(sdk.UsageReader):
            reader = "mute"
            bucket_prefix = "m:"


def test_account_provider_and_model_gateway_describe_and_answer_defaults() -> None:
    class DemoAccounts(sdk.AccountProvider):
        title = "Demo accounts"
        description = "An account kind."
        kinds_offered = [
            sdk.AccountKindInfo(
                id="demo-key",
                title="Demo key",
                settings={
                    "type": "object",
                    "properties": {"api_key": sdk.credential_schema("Key")},
                    "required": ["api_key"],
                },
                endpoints=[sdk.EndpointSlot(purpose="model_api", key="base_url")],
                provides=["openai-api"],
            )
        ]
        logins = [sdk.LoginMethodInfo(id="demo-login", title="Key", produces="demo-key", steps=["ask"])]

    class LocalAccounts(DemoAccounts):
        local = True

    destroyed: List[str] = []

    class DummyHandle:
        def __init__(self, tag: str) -> None:
            self.tag = tag

        def destroy(self) -> None:
            destroyed.append(self.tag)

    provider = DemoAccounts()
    assert provider.provides({"kind": "demo-key", "settings": {}}) == ["openai-api"]
    with pytest.raises(BadValue):
        provider.provides({"kind": "other"})
    assert provider.describe() == {"title": "Demo accounts", "description": "An account kind."}
    offered = provider.offers()
    assert offered["kinds"][0]["id"] == "demo-key"
    assert offered["logins"][0]["produces"] == "demo-key"
    assert provider.usage({"account": {"id": "acc-1"}}, DummyHandle("s"), DummyHandle("k")) == {
        "supported": False,
        "unsupported_because": "this account provider reports no plan limits",
        "sources": [],
        "account": "acc-1",
        "revision": 0,
        "observed_at": 0,
        "buckets": [],
    }
    assert provider.models({}, DummyHandle("s2"), DummyHandle("k2")) == {
        "known": False,
        "unknown_because": "this account provider does not list models",
        "models": [],
    }
    with pytest.raises(Unsupported):
        provider.renew({}, DummyHandle("s3"))
    assert destroyed == ["s", "k", "s2", "k2", "s3"]
    with pytest.raises(Unsupported):
        provider.signed_in({"environment": {"DEMO_KEY": "CANARY"}})

    assert LocalAccounts().signed_in({})["state"] == "unknown"
    assert LocalAccounts().provides({"kind": "local"}) == []
    assert sdk.models_listed([sdk.ModelInfo("m1", capabilities=["chat"], limits=sdk.ModelLimits(context_tokens=8))]) == {
        "known": True,
        "models": [
            {
                "id": "m1",
                "efforts": [],
                "is_default": False,
                "capabilities": ["chat"],
                "limits": {
                    "context_tokens": 8,
                    "dimensions_adjustable": False,
                    "audio_in_formats": [],
                    "audio_out_formats": [],
                    "voices": [],
                    "styles": [],
                },
            }
        ],
    }
    assert sdk.models_listed([], signed_out=True)["signed_out"] is True
    assert sdk.login_present() == {"state": "signed_in", "variable": None}
    assert sdk.login_present("DEMO_KEY") == {"state": "signed_in", "variable": "DEMO_KEY"}
    assert sdk.login_absent("no file") == {"state": "signed_out", "reason": "no file"}
    assert sdk.login_unknown("no keyring") == {"state": "unknown", "reason": "no keyring"}
    assert provide.library("demo_accounts", "1", providers=[DemoAccounts]).kinds == ("account_provider",)

    class LocalLease(sdk.GatewayLease):
        def address(self) -> str:
            return "http://127.0.0.1:4100"

        def bearer(self) -> str:
            return "session-bearer"

    class LocalGateway(sdk.ModelGateway):
        kind = "demo-key"
        vendor = "demo"
        table = "demo"

        def open(self, request: Any, spawner: Any) -> sdk.GatewayLease:
            sdk.release(spawner)
            assert sdk.request_of(request)["account"] == "acc-1"
            return LocalLease()

    with pytest.raises(TypeError, match="IncompleteGateway does not implement open"):

        class IncompleteGateway(sdk.ModelGateway):
            kind = "demo-key"
            vendor = "demo"

    gateway = LocalGateway()
    assert gateway.describe() == {"kind": "demo-key", "vendor": "demo", "table": "demo"}
    lease = gateway.open({"account": "acc-1"}, DummyHandle("gw-s"))
    assert lease.address() == "http://127.0.0.1:4100"
    assert lease.bearer() == "session-bearer"
    assert lease.models()["known"] is False
    assert destroyed[-1] == "gw-s"
    assert provide.library("local_gw", "1", providers=[LocalGateway]).kinds == ("model_gateway",)


def test_a_bucket_says_its_group_the_names_of_its_models_and_that_it_is_disabled() -> None:
    made = sdk.bucket("q:weekly", group="Gemini Models", model_names=["Gemini Alpha"], status="disabled", observed_at=1)
    assert made["group"] == "Gemini Models"
    assert made["model_names"] == ["Gemini Alpha"]
    assert made["status"] == "disabled"
    assert "group" not in sdk.bucket("q:weekly", observed_at=1)


def test_a_source_lets_go_of_the_fetch_it_was_handed_whether_or_not_it_answers() -> None:
    class Read(sdk.ReadCatalogSource):
        pass

    spawner, fetch = _Spawner(files={"/here/catalog.toml": b"x = 1\n"}), _Fetch()
    request = {"kind": "overlays", "label": "x", "fields": json.dumps({"source": "/here/catalog.toml"})}
    assert Read().read(request, spawner, fetch) == "x = 1\n"
    assert spawner.destroyed and fetch.destroyed
    refused = _Fetch()
    with pytest.raises(BadValue):
        Read().read({"kind": "overlays", "label": "x", "fields": json.dumps({})}, _Spawner(), refused)
    assert refused.destroyed


def test_a_source_describes_the_places_it_fetches_from_and_the_keys_it_verifies_with() -> None:
    class Hub(sdk.CatalogSource):
        source = "hub"
        title = "Hub"
        endpoints = ["https://huggingface.co/"]
        keys = ["AB12"]

        def read(self, request: Any, spawner: Any, fetch: Any) -> str:
            return ""

    described = Hub().describe()
    assert list(described["endpoints"]) == ["https://huggingface.co/"] and list(described["keys"]) == ["AB12"]


def test_a_notifier_describes_itself_and_a_slot_left_in_place_is_refused() -> None:
    class Webhook(sdk.Notifier):
        notifier = "webhook"
        title = "Webhook"
        events = ["finished", "failed"]
        values = RegionValues

        def send(self, request: Any) -> Any:
            notice = sdk.notice_of(request)
            return sdk.delivery(notice["id"])

    described = Webhook().describe()
    assert described["id"] == "webhook" and described["events"] == ["finished", "failed"]
    assert described["replies"] is False and described["schema"]["required"] == ["name"]
    request = {"settings": '{"name": "x"}', "notice": {"id": "n1", "event": "finished", "title": "T", "body": "B"}}
    assert sdk.settings_of(request) == {"name": "x"}
    assert Webhook().send(request) == {"id": "n1", "replies": False}
    assert Webhook().reply(request) is None and Webhook().withdraw(request) is None
    assert provide.library("notifiers", "1", providers=[Webhook]).kinds == ("notifier",)

    with pytest.raises(TypeError, match="send"):

        class Silent(sdk.Notifier):
            notifier = "silent"
            title = "Silent"


class _Host:
    """The host's `content_host`, as a call answers: reads what a test holds, by address."""

    def __init__(self, files: dict) -> None:
        self.files = files
        self.destroyed = False

    def child(self, base: str, name: str) -> str:
        return base.rstrip("/") + "/" + name

    def open_text(self, uri: str, limit: int) -> str:
        if uri not in self.files:
            raise NotFound("no such document")
        return self.files[uri]

    def destroy(self) -> None:
        self.destroyed = True


class Scheme(sdk.ContentSource):
    source = "files_of_team"
    title = "Team files"
    schemes = [sdk.scheme("team", "Team files", "team://name/path", lists=True)]

    def open(self, request: Any) -> Any:
        uri = sdk.request_of(request)["target"]["uri"]
        if uri.endswith("/missing"):
            return sdk.refused("not_found", "team: no such file")
        return sdk.opened_inline(sdk.stat_file(), uri.encode())

    def stat(self, target: Any) -> Any:
        return sdk.stat_found(sdk.stat_file(size=3))


class Listing(sdk.CatalogGenerator):
    generator = "team_listing"
    title = "A team's listing"
    types = ["skills"]
    grammar = "team://name"

    def generate(self, request: Any, spawner: Any, host: Any) -> str:
        try:
            source = sdk.request_of(request)["source"]
            return host.open_text(host.child(source, "list.json"), 1024)
        finally:
            spawner.destroy()
            host.destroy()


def test_a_content_source_is_described_by_its_schemes_and_answers_with_data_or_a_refusal() -> None:
    described = Scheme().describe()
    assert described["id"] == "files_of_team"
    assert described["schemes"] == [
        {"name": "team", "title": "Team files", "grammar": "team://name/path", "lists": True, "resolves": False, "ranges": False}
    ]
    opened = Scheme().open({"target": {"uri": "team://core/a"}})
    assert opened["opened"]["delivery"] == {"inline": b"team://core/a"}
    assert opened["opened"]["stat"]["size"] == len(b"team://core/a")
    assert Scheme().open({"target": {"uri": "team://core/missing"}}) == {
        "refusal": {"kind": "not_found", "why": "team: no such file"}
    }
    assert Scheme().stat({"uri": "team://x"}) == {"stat": {"kind": "file", "size": 3, "immutable": False, "digests": []}}
    with pytest.raises(Unsupported):
        Scheme().list({"target": {"uri": "team://x"}})
    with pytest.raises(Unsupported):
        Scheme().resolve({"uri": "team://x"})
    assert Scheme.__guatiao_provider__.id == "files_of_team"
    assert provide.library("content", "1", providers=[Scheme, Listing]).kinds == ("content_source", "catalog_generator")


def test_a_catalog_generator_reads_through_the_host_and_lets_go_of_what_it_was_handed() -> None:
    host = _Host({"team://core/list.json": '{"skills": []}'})
    spawner = _Spawner(None)
    request = {"kind": "skills", "label": "x", "source": "team://core", "fields": "{}"}
    assert Listing().generate(request, spawner, host) == '{"skills": []}'
    assert host.destroyed and spawner.destroyed
    description = Listing().describe()
    assert description["id"] == "team_listing" and description["types"] == ["skills"]
    assert description["source"] == "required" and description["grammar"] == "team://name"
    assert "schema" not in description
    with pytest.raises(NotFound):
        Listing().generate({**request, "source": "team://other"}, _Spawner(None), _Host({}))
    assert Listing.__guatiao_provider__.id == "team_listing"


def test_a_generator_that_reads_no_source_says_so() -> None:
    class Runs(sdk.CatalogGenerator):
        generator = "runs"
        title = "Runs"
        reads = "absent"

        def generate(self, request: Any, spawner: Any, host: Any) -> str:
            return ""

    assert Runs().describe()["source"] == "absent"

def test_a_file_viewer_describes_itself_and_answers_from_the_file_the_host_read() -> None:
    import base64
    import json as _json

    class Stamp(sdk.SurfaceViewer):
        viewer = "stamp"
        title = "Stamp"
        extensions = ["stamp"]

        def surface(self, file: sdk.ViewedFile) -> List[Any]:
            return [{"version": "1.0", "createSurface": {"surfaceId": "s", "components": [
                {"id": "root", "component": "Text", "text": f"{file.name}: {file.text()}"}]}}]

    assert Stamp().describe() == {
        "id": "stamp",
        "title": "Stamp",
        "mime_types": [],
        "extensions": ["stamp"],
        "priority": 0,
        "shows": "surface",
        "opens_sections": False,
    }
    asked = {"name": "a.stamp", "size": 6, "data": base64.b64encode("h\u00e9llo".encode()).decode(), "truncated": False}
    assert _json.loads(Stamp().render(asked))[0]["createSurface"]["components"][0]["text"] == "a.stamp: h\u00e9llo"

    class Page(sdk.PageViewer):
        viewer = "page"
        title = "Page"
        mime_types = ["text/x-page"]
        priority = 3

        def page(self, file: sdk.ViewedFile) -> str:
            return f"<p>{file.size}</p>"

    assert Page().describe()["shows"] == "page" and Page().describe()["priority"] == 3
    assert Page().render(asked) == "<p>6</p>"
    assert provide.library("viewers", "1", providers=[Stamp, Page]).kinds == ("file_viewer",)
