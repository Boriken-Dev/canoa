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


def test_a_network_item_names_the_network_its_mode_asks_for_and_what_that_mode_needs() -> None:
    class Proxy(sdk.NetworkProfileItem):
        item = "proxy"
        title = "Proxy"

    assert _apply(Proxy(), {})["network"]["mode"] == "system"
    own = _apply(
        Proxy(),
        {"mode": "proxy", "url": "http://p.example:3128", "bypass": ["a.example"], "user": "me", "password_variable": "PW"},
    )["network"]
    assert (own["url"], own["bypass"], own["password_variable"]) == ("http://p.example:3128", ["a.example"], "PW")
    command = _apply(Proxy(), {"mode": "command", "command": ["myproxy", "{listen}"]})
    assert command["network"]["command"] == ["myproxy", "{listen}"]
    ended = _apply(Proxy(), {"client_certificate_hosts": [{"pattern": "a.example"}]})
    assert ended["network"]["intercept"] == [{"pattern": "a.example", "identity": "system"}]
    assert "a.example" in ended["notes"][1]
    for values in ({"mode": "proxy"}, {"mode": "pac"}, {"mode": "command"}, {"mode": "tor"}):
        with pytest.raises(BadValue):
            _apply(Proxy(), values)


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


def _read(source: Any, fields: dict, spawner: _Spawner) -> str:
    return source.read({"kind": "overlays", "label": "x", "fields": json.dumps(fields)}, spawner)


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

        def read(self, request: Any, spawner: Any) -> str:
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

        def provides(self, account: Any) -> List[str]:
            return ["openai-api"]

    destroyed: List[str] = []

    class DummyHandle:
        def __init__(self, tag: str) -> None:
            self.tag = tag

        def destroy(self) -> None:
            destroyed.append(self.tag)

    provider = DemoAccounts()
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
