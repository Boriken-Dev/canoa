"""A Store extension written in Python: it adds one type, Notes, to the Store. A type's `stores` name the `program_store` providers the same
library offers; this one has none, so the type lists nothing to install and shows what an extension is made of.
"""

from __future__ import annotations

from canoa_sdk import ExtensionType, StoreExtension

__version__ = "1.0.0"
__all__ = ["Notes"]


class Notes(StoreExtension):
    title = "Notes"
    publisher = "Example publisher"
    types = [ExtensionType("notes", "Notes", icon="extension")]
