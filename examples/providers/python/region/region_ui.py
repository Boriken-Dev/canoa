"""What a team might add to Canoa's screens, written in Python: a profile item that tells a session which region it runs in, a catalog
source that lists a team's skills, a page of Settings and a tab of the bottom panel. One library serves four kinds; each class is a
provider the host offers as it offers any extension's.
"""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from typing import Annotated, List

from canoa_sdk import (
    CatalogSource,
    CommandCatalogSource,
    EnvProfileItem,
    LogPanelTab,
    ReadCatalogSource,
    SettingsSection,
    fields_of,
    section,
    ui,
)

__version__ = "1.0.0"
__all__ = ["BuildsTab", "EchoCommand", "EchoRead", "RegionItem", "RegionSection", "TeamSource"]


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
    """A team's skills: no program or file is read, so the spawner the host hands over is let go."""

    source = "team"
    title = "A team's list"
    types = ["skills"]
    values = TeamValues

    def read(self, request, spawner):  # type: ignore[no-untyped-def]
        spawner.destroy()
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
