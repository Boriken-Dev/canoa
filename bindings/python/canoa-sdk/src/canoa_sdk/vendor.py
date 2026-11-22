"""The `agent_vendor` kind: who an agent is and how Canoa shows it, so the host and the app hold no list of agents.

```python
class Quill(AgentVendor):
    agent = "quill"
    title = "Quill"
    vendor = "quill-labs"
    standards = [StandardTitle("quill-key", "Quill API key")]
    mark = "<svg viewBox='0 0 24 24'><path d='M2 2h20L2 22h20'/></svg>"
```
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import ClassVar, List, Optional, Sequence

from guatiao import provide
from guatiao.kinds import ValueAnswer

from . import kinds
from ._model import Model
from ._provider import WithForm

__all__ = ["AgentVendor", "ModeTitle", "StandardTitle", "VendorInfo"]


@dataclass
class StandardTitle(Model):
    """A credential standard that is the vendor's own: its id (`anthropic-api`) and what a person reads for it."""

    id: str
    title: str


@dataclass
class ModeTitle(Model):
    """A mode the agent is known to offer before it has run, with its name for people."""

    id: str
    title: str


@dataclass
class VendorInfo(Model):
    """What a vendor description holds; the fields are `canoa::vendor::VendorInfo`'s."""

    agent: str
    title: str
    program: Optional[str] = None
    vendor: Optional[str] = None
    aos: Optional[str] = None
    model_variable: Optional[str] = None
    tool_name: Optional[str] = None
    skills_folder: Optional[str] = None
    context_files: List[str] = field(default_factory=list)
    standards: List[StandardTitle] = field(default_factory=list)
    names: List[str] = field(default_factory=list)
    markers: List[str] = field(default_factory=list)
    modes: List[ModeTitle] = field(default_factory=list)
    endpoints: List[str] = field(default_factory=list)
    mark: Optional[str] = None
    look: Optional[str] = None


class AgentVendor(WithForm, provide.Provider, kinds.AgentVendor):
    """Subclass it and set the class attributes below; the provider answers `describe` from them. Override `info()` to compute
    the description instead.

    `agent` is the id of the agent's `acp_harness` provider when it has one, else its source's name in lower case. `mark` is
    SVG text and `look` a JSON document, each at most 65536 characters; the host refuses a description that does not fit
    and says why in its log.
    """

    agent: ClassVar[str]
    title: ClassVar[str]
    program: ClassVar[Optional[str]] = None
    vendor: ClassVar[Optional[str]] = None
    aos: ClassVar[Optional[str]] = None
    model_variable: ClassVar[Optional[str]] = None
    tool_name: ClassVar[Optional[str]] = None
    skills_folder: ClassVar[Optional[str]] = None
    context_files: ClassVar[Sequence[str]] = ()
    standards: ClassVar[Sequence[StandardTitle]] = ()
    names: ClassVar[Sequence[str]] = ()
    markers: ClassVar[Sequence[str]] = ()
    modes: ClassVar[Sequence[ModeTitle]] = ()
    endpoints: ClassVar[Sequence[str]] = ()
    mark: ClassVar[Optional[str]] = None
    look: ClassVar[Optional[str]] = None

    def info(self) -> VendorInfo:
        """The description, from the class attributes."""
        return VendorInfo(
            agent=self.agent,
            title=self.title,
            program=self.program,
            vendor=self.vendor,
            aos=self.aos,
            model_variable=self.model_variable,
            tool_name=self.tool_name,
            skills_folder=self.skills_folder,
            context_files=list(self.context_files),
            standards=list(self.standards),
            names=list(self.names),
            markers=list(self.markers),
            modes=list(self.modes),
            endpoints=list(self.endpoints),
            mark=self.mark,
            look=self.look,
        )

    def describe(self) -> ValueAnswer:
        return self.info().to_data()
