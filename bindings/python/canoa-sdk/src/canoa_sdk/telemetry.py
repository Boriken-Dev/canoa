"""The `telemetry_exporter` kind: somewhere the host's traces, metrics and events go, when the person turned that on.

```python
class LineExporter(TelemetryExporter):
    title = "A line a batch"
    signals = ["logs"]

    def __init__(self, folder: str) -> None:
        self.folder = folder

    def export(self, batch):
        ...  # deliver it, or raise Gone to have the host try again, BadValue to have it drop the batch
```

The host decides what is recorded (a closed list the person reads and accepts) and owns batching, the queue, retries and the stop; an
exporter only delivers the batches it is handed. A batch is plain data in the shape of OpenTelemetry's: `resource` (attributes),
`spans`, `metrics` and `logs`, a time in Unix nanoseconds, an attribute holding one of `text`, `int`, `number` and `flag`.

`export` returns when the batch is delivered. Raise `guatiao.errors.Gone` when the destination cannot be reached or refused the
credentials (the host keeps the batch and tries again after a growing pause) and `guatiao.errors.BadValue` when another try will not help
(the batch is dropped and counted); the message is one line with no secret and never the destination's own words. `flush` is called
once when the host stops; leave it as it is when nothing is held. The settings are the `__init__` arguments of the class, as for any
provider.
"""

from __future__ import annotations

from typing import Any, ClassVar, Dict, List, Optional

from guatiao import Ref, provide
from guatiao.errors import Unsupported
from guatiao.kinds import ValueAnswer

from . import kinds
from ._model import present
from ._provider import WithForm, abstract

__all__ = ["TelemetryExporter"]


class TelemetryExporter(WithForm, provide.Provider, kinds.TelemetryExporter, abstract=True):
    """Subclass it with `title`, optionally `help` and `signals` (which of `"spans"`, `"metrics"` and `"logs"` it takes: all three
    when left out) and implement `export`."""

    title: ClassVar[str]
    help: ClassVar[Optional[str]] = None
    signals: ClassVar[List[str]] = ["spans", "metrics", "logs"]

    def info(self) -> Dict[str, Any]:
        """The description, from the class attributes."""
        return present({"title": self.title, "help": self.help, "signals": list(self.signals)})

    def describe(self) -> ValueAnswer:
        return self.info()

    @abstract
    def export(self, batch: Ref) -> None:
        """Delivers a batch."""
        raise Unsupported("export is not implemented")

    def flush(self) -> None:
        """Finishes what the exporter holds, when the host stops."""
        return None
