"""A usage reader written in Python: how an agent's own quota is read when the agent reports none. An agent entry in canoa.cfg that names
it (`usage = "listed"`) has the host read the agent's quota through it, with no change to Canoa.

This reader reads what the agent said when a session opened: the models it lists. Each becomes a bucket, full, since the agent this
stands for never limits a model. A reader of an agent that serves its figures somewhere reads them there, from the processes
`members` names.
"""

from __future__ import annotations

import json
from typing import Any, Dict, List

from canoa_sdk import Retry, UsageReader, UsageReading, bucket, failed, reading

__version__ = "1.0.0"
__all__ = ["ListedUsage"]


class Listed(UsageReading):
    """One agent's reading: the models its `session/new` answer listed."""

    def __init__(self, members: Any) -> None:
        super().__init__(members)
        self.models: List[Dict[str, str]] = []

    def opened(self, result: str) -> None:
        try:
            answer = json.loads(result)
            listed = answer["models"]["availableModels"]
        except (ValueError, KeyError, TypeError):
            return
        self.models = [
            {"id": model["modelId"], "name": model.get("name", model["modelId"])}
            for model in listed
            if isinstance(model, dict) and isinstance(model.get("modelId"), str)
        ]

    def read(self, root: int, tree: bool) -> Dict[str, Any]:
        if not self.models:
            return failed("the agent has listed no model yet", Retry.NOT_YET)
        # The agent's own process is always among its members; a table with none says the process is gone.
        if root not in self.processes(root):
            return failed("the agent is not running", Retry.NEVER)
        return reading(
            [
                bucket(f"listed:{model['id']}", label=model["name"], models=[model["id"]], remaining=1000, limit=1000, status="ok")
                for model in self.models
            ]
        )


class ListedUsage(UsageReader):
    """The reader an entry names as `listed`."""

    reader = "listed"
    bucket_prefix = "listed:"
    # The host reads soon after a session opens and again after a tenth of a second: an example is looked at, not waited for.
    settle_ms = 100
    cache_ms = 100

    def open(self, members: Any) -> Listed:
        return Listed(members)
