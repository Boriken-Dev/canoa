"""The Store's kinds: `store_extension`, which adds types to the Store, `program_store`, which knows one program, and
`model_store`, which lists the models a type's programs run.

```python
class Notes(StoreExtension):
    title = "Notes"
    publisher = "Example"
    types = [ExtensionType("notes", "Notes", stores=["notes_program"])]
```

`stores` names `program_store` providers the same library offers; the host lists and installs them as any store. A type's `models`
names the `model_store` provider that lists its models, which the Store shows as a page of their own.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, ClassVar, Dict, List, Optional, Sequence

from guatiao import Ref, provide
from guatiao.errors import Unsupported
from guatiao.kinds import Taken, ValueAnswer

from . import kinds
from ._model import Model
from ._provider import WithForm, abstract

__all__ = ["CatalogWords", "DefaultCatalog", "ExtensionInfo", "ExtensionType", "ModelStore", "ProgramStore", "StoreExtension"]


@dataclass
class DefaultCatalog(Model):
    """A catalog a type ships: its `label` in canoa.cfg, the `catalog_source` provider that reads it, that provider's `fields` (a JSON
    object as text), and optionally the `pin` the author verified and the catalog's `snapshot` text as of that pin."""

    label: str
    provider: str
    fields: Optional[str] = None
    pin: Optional[str] = None
    snapshot: Optional[str] = None


@dataclass
class CatalogWords(Model):
    """The catalogs of a type: what one is called (`noun`, `plural`), what the type lists (`lists`), the `format` each answers in,
    and the `defaults` the type ships."""

    noun: str
    plural: str
    lists: str
    format: Optional[str] = None
    defaults: List[DefaultCatalog] = field(default_factory=list)


@dataclass
class ExtensionType(Model):
    """One type an extension adds: a word of lower-case letters and digits (1 to 32, none the Store has itself), its title in the
    plural, an icon (the name of one of the application's, or a small `data:image/png;base64,` or `data:image/webp;base64,`
    address), the ids of the `program_store` providers that serve it and, when its programs run models, the id of the `model_store`
    provider that lists them."""

    id: str
    title: str
    stores: List[str] = field(default_factory=list)
    icon: Optional[str] = None
    models: Optional[str] = None
    catalogs: Optional[CatalogWords] = None


@dataclass
class ExtensionInfo(Model):
    """What an extension is: its title, who publishes it and the types it adds."""

    title: str
    publisher: str
    types: List[ExtensionType] = field(default_factory=list)


class StoreExtension(WithForm, provide.Provider, kinds.StoreExtension):
    """Subclass it with `title`, `publisher` and `types`, or override `info()`."""

    title: ClassVar[str]
    publisher: ClassVar[str]
    types: ClassVar[Sequence[ExtensionType]] = ()

    def info(self) -> ExtensionInfo:
        """The description, from the class attributes."""
        return ExtensionInfo(self.title, self.publisher, list(self.types))

    def describe(self) -> ValueAnswer:
        return self.info().to_data()


class ProgramStore(WithForm, provide.Provider, kinds.ProgramStore, abstract=True):
    """A provider that knows one program: where its copies are, what is newest, how it installs.

    Override `info()` (the `StoreInfo` as a map: `program`, `endpoints`, `serves`, `summary`, `icon`, `commands`) and the four
    slots `program`, `latest`, `plan` and `apply`, as `canoa.store.kinds.json` describes them. Every one is required and
    synchronous; a slot takes the host's objects (`spawner`, `tools`, `prompter`) and the provider owns and destroys them.
    """

    def info(self) -> Dict[str, Any]:
        """The `StoreInfo` this provider answers `describe` with."""
        raise NotImplementedError("a ProgramStore says its info()")

    def describe(self) -> ValueAnswer:
        return self.info()

    @abstract
    def program(
        self, query: Ref, spawner: Taken[kinds.ProcessSpawner], tools: Taken[kinds.InstallTools]
    ) -> ValueAnswer:
        raise Unsupported("program is not implemented")

    @abstract
    def latest(self, query: Ref, tools: Taken[kinds.InstallTools]) -> ValueAnswer:
        raise Unsupported("latest is not implemented")

    @abstract
    def plan(self, request: Ref, tools: Taken[kinds.InstallTools]) -> ValueAnswer:
        raise Unsupported("plan is not implemented")

    @abstract
    def apply(
        self,
        request: Ref,
        spawner: Taken[kinds.ProcessSpawner],
        tools: Taken[kinds.InstallTools],
        prompter: Taken[kinds.InstallPrompter],
    ) -> ValueAnswer:
        raise Unsupported("apply is not implemented")


class ModelStore(WithForm, provide.Provider, kinds.ModelStore, abstract=True):
    """The models of a Store type: what the Store lists, moves and sets up for the programs of the type's `program_store`.

    Override `info()` (the `ModelStoreInfo` as a map: `title`, `catalog`, `publisher`, `missing`, and `settings` and `speech` when the
    type keeps a setting per model or its service can transcribe) and the four slots `list`, `act`, `keep` and `service`, as
    `canoa.model_store.kinds.json` describes them. Every one is required and synchronous; a slot takes the host's `spawner`, which the
    provider owns and destroys, and runs the type's program through it. A slot the provider has nothing to answer refuses with
    `Unsupported`, with its reason.
    """

    def info(self) -> Dict[str, Any]:
        """The `ModelStoreInfo` this provider answers `describe` with."""
        raise NotImplementedError("a ModelStore says its info()")

    def describe(self) -> ValueAnswer:
        return self.info()

    @abstract
    def list(self, query: Ref, spawner: Taken[kinds.ProcessSpawner]) -> ValueAnswer:
        raise Unsupported("list is not implemented")

    @abstract
    def act(self, request: Ref, spawner: Taken[kinds.ProcessSpawner]) -> ValueAnswer:
        raise Unsupported("act is not implemented")

    @abstract
    def keep(self, request: Ref, spawner: Taken[kinds.ProcessSpawner]) -> ValueAnswer:
        raise Unsupported("keep is not implemented")

    @abstract
    def service(self, query: Ref, spawner: Taken[kinds.ProcessSpawner]) -> ValueAnswer:
        raise Unsupported("service is not implemented")
