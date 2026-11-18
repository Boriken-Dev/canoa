# canoa-sdk

Write a Canoa provider in Python. Each kind Canoa loads has one base class: a new provider subclasses it, sets a few attributes or
overrides a few methods, and lists the class in its module's `__all__`. A [Guatiao](https://github.com/Boriken-Dev/guatiao) source
for Python loads the module like any other, and the host offers the provider with no change to Canoa.

```python
from canoa_sdk import AgentVendor, StandardTitle

__version__ = "1.0.0"
__all__ = ["Quill"]

class Quill(AgentVendor):
    agent = "quill"
    title = "Quill"
    standards = [StandardTitle("quill-key", "Quill API key")]
```

A library is the module plus a `*.guatiao` manifest naming it and the kinds it serves; the host's `canoa.cfg` entry lists the
manifest's folder and the folders holding Guatiao's Python binding and this package under `source_settings` (`dirs`, `paths`). The
examples in `examples/providers/python` are loaded by `crates/canoa-host/tests/script_providers.rs`.

| Kind | Base class | Module |
| --- | --- | --- |
| `agent_vendor` | `AgentVendor` | `canoa_sdk.vendor` |
| `store_extension`, `program_store` | `StoreExtension`, `ProgramStore` | `canoa_sdk.store` |
| `speech_provider`, `speech_output` | `SpeechProvider`, `SpeechOutput` | `canoa_sdk.speech` |
| coqui-acp's `acp_harness`, `agent_provider` | re-exported | `canoa_sdk.acp` |

A provider's settings are the dataclass its `__init__` takes; `canoa_sdk.forms` writes the form that shows them (`ui()` on a field,
`form=` on the class). See `AGENTS.md` for the API.

Development: `GUATIAO_LIBRARY=<guatiao build> PYTHONPATH=<guatiao>/bindings/python/guatiao/src pytest`; after a kind's description
changes, `python tools/write_kinds.py`.
