"""Somewhere the host's telemetry can go, written in Python: an exporter that appends one line of counts for each batch to a file.

It takes the three groups and keeps nothing of what a batch holds but how many spans, metrics and events it had; a real exporter would
send the batch on. The host's own `file` exporter writes the whole batch as OTLP's JSON.
"""

from __future__ import annotations

import json
from typing import Any

from guatiao import Ref

from canoa_sdk import TelemetryExporter

__version__ = "1.0.0"
__all__ = ["LineExporter"]


class LineExporter(TelemetryExporter):
    """One line a batch: how many spans, metrics and events it held."""

    title = "A line a batch"
    help = "Appends the counts of each batch to a file."

    def __init__(self, path: str) -> None:
        self.path = path

    def export(self, batch: Ref) -> None:
        body: Any = batch.to_python() if hasattr(batch, "to_python") else batch
        counts = {group: len(body.get(group) or []) for group in ("spans", "metrics", "logs")}
        with open(self.path, "a", encoding="utf-8", newline="") as file:
            file.write(json.dumps(counts) + "\n")
