"""What a team might add to Canoa's screens, written in Python: a profile item that tells a session which region it runs in, a catalog
source that lists a team's skills, a URI scheme of the team's own and a generator that reads it, a page of Settings, a tab of the
bottom panel, a place notices are sent to and a viewer of the Files surface. One library serves eight kinds; each class is a provider
the host offers as it offers any extension's.
"""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from typing import Annotated, List

from canoa_sdk import (
    CatalogGenerator,
    CatalogSource,
    CommandCatalogSource,
    ContentSource,
    EnvProfileItem,
    LogPanelTab,
    Notifier,
    PagePanelTab,
    ReadCatalogSource,
    SettingsSection,
    delivery,
    SurfaceViewer,
    ViewedFile,
    fields_of,
    notice_of,
    opened_inline,
    request_of,
    scheme,
    section,
    stat_file,
    ui,
)

__version__ = "1.0.0"
__all__ = ["BuildsTab", "DocsTab", "EchoCommand", "EchoRead", "RegionItem", "RegionSection", "StampViewer", "TeamChat", "TeamFiles", "TeamListing", "TeamSource"]


@dataclass
class RegionValues:
    __sections__ = (section("where", "Where"),)

    name: Annotated[str, ui(section="where", placeholder="eu-west")]
    hosts: List[str] = field(default_factory=list)


class RegionItem(EnvProfileItem):
    """The region the session runs in, as the variable `REGION`; each entry of `hosts` is a `NAME=value` line."""

    item = "region"
    title = "Region"
    help = "Where the session runs."
    harnesses = ["claude-code"]
    values = RegionValues
    variables = {"name": ["REGION"]}
    lines = "hosts"
    note = "puts the session in a region"


@dataclass
class TeamValues:
    team: str


class TeamSource(CatalogSource):
    """A team's skills: no program or file is read, so the spawner and the fetch the host hands over are let go."""

    source = "team"
    title = "A team's list"
    types = ["skills"]
    values = TeamValues

    def read(self, request, spawner, fetch):  # type: ignore[no-untyped-def]
        spawner.destroy()
        fetch.destroy()
        team = fields_of(request)["team"]
        return json.dumps({"skills": [{"name": f"from-{team}", "description": "Listed by a script.", "source": "owner/repo"}]})


class EchoCommand(CommandCatalogSource):
    """The SDK's command source under an id of its own: the program a catalog names runs through the host's spawner."""

    source = "echo_command"
    title = "Echo a program"


class EchoRead(ReadCatalogSource):
    """The SDK's read source under an id of its own: the file a catalog names is read through the host's spawner."""

    source = "echo_read"
    title = "Echo a file"


class TeamFiles(ContentSource):
    """A URI scheme of a team's own: `team://NAME/skills.json` is a list of that team's skills, made up here instead of fetched."""

    source = "team_files"
    title = "Team files"
    schemes = [scheme("team", "A team's files", "team://name/path")]

    def open(self, request):  # type: ignore[no-untyped-def]
        uri = request_of(request)["target"]["uri"]
        team = uri[len("team://") :].split("/")[0]
        listing = {"skills": [{"name": f"from-{team}", "description": "Read through a scheme a script added.", "source": "owner/repo"}]}
        return opened_inline(stat_file(), json.dumps(listing).encode("utf-8"))


class TeamListing(CatalogGenerator):
    """A generator that makes a skills catalog from what its source holds: it reads `<source>/skills.json` through the host."""

    generator = "team_listing"
    title = "A team's listing"
    types = ["skills"]
    grammar = "team://name"

    def generate(self, request, spawner, host):  # type: ignore[no-untyped-def]
        try:
            source = request_of(request)["source"]
            return host.open_text(host.child(source, "skills.json"), 1 << 20)
        finally:
            spawner.destroy()
            host.destroy()


class RegionSection(SettingsSection):
    """A page of Settings whose values the host keeps in canoa.cfg."""

    section = "region"
    title = "Region"
    group = "Team"
    values = RegionValues


class BuildsTab(LogPanelTab):
    """A tab of the bottom panel showing the last builds."""

    tab = "builds"
    title = "Builds"

    def lines(self) -> List[str]:
        return ["build 41 passed", "build 42 failed"]


class DocsTab(PagePanelTab):
    """A tab of the bottom panel that names a page this computer serves and opens it in the browser."""

    tab = "docs"
    title = "Team docs"
    url = "http://127.0.0.1:4100/docs"


class TeamChat(Notifier):
    """Somewhere a notice could be sent: it keeps the notice's id and says it was delivered."""

    notifier = "teamchat"
    title = "Team chat"
    help = "Sends each notice to the team's chat."
    events = ["finished", "failed"]

    def send(self, request):  # type: ignore[no-untyped-def]
        return delivery(notice_of(request)["id"])


class StampViewer(SurfaceViewer):
    """Shows a `.stamp` file as one line of text in the application's own widgets: the host reads the file and hands over its name and
    bytes, so the viewer never sees where the file is."""

    viewer = "stamp"
    title = "Stamp"
    extensions = ["stamp"]
    priority = 20

    def surface(self, file: ViewedFile) -> List[dict]:  # type: ignore[type-arg]
        text = f"{file.name}: {file.text().strip()}"
        return [
            {
                "version": "1.0",
                "createSurface": {"surfaceId": "stamp", "components": [{"id": "root", "component": "Text", "text": text}]},
            }
        ]
