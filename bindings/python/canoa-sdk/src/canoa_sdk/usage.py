"""The `usage_reader` kind: how an agent's own quota is read when the agent does not report it over ACP.

```python
class QuillUsage(UsageReader):
    reader = "quill"
    bucket_prefix = "quill:"

    def open(self, members):
        return QuillReading(members)


class QuillReading(UsageReading):
    def opened(self, result: str) -> None:
        ...  # keep what the agent said when a session opened

    def read(self, root: int, tree: bool):
        return reading([bucket("quill:five_hour", remaining=750, resets_at=1893456000)])
```

The host decides when to read; the reader says what a reading is. A reading is the complete set of buckets (each id starting with
`bucket_prefix`) or a failure that says why there is none and whether to ask again. What a reader needs of the machine it asks for
through the `members` object `open` is handed: the processes that belong to the agent.
"""

from __future__ import annotations

import time
from dataclasses import dataclass, field
from typing import Any, ClassVar, Dict, List, Optional, Sequence

from guatiao import provide
from guatiao.errors import Unsupported
from guatiao.kinds import Taken, ValueAnswer

from . import kinds
from ._model import Model, data_of
from ._provider import WithForm, abstract

__all__ = ["ReaderInfo", "Retry", "UsageReader", "UsageReading", "bucket", "failed", "reading"]


class Retry:
    """Whether a reading that gave nothing is worth asking for again."""

    #: The reason will not change by waiting.
    NEVER = "never"
    #: The agent shows nothing to read yet: no session has started what serves its figures.
    NOT_YET = "not_yet"
    #: It showed something and it did not answer in time or could not be reached.
    NO_ANSWER = "no_answer"


@dataclass
class ReaderInfo(Model):
    """How a reader's figures are asked for and kept; the fields are `canoa::usage_reader::ReaderInfo`'s. Times are milliseconds."""

    id: str
    bucket_prefix: str
    settle_ms: int = 10_000
    retries_ms: List[int] = field(default_factory=list)
    cache_ms: int = 15_000
    private_limit_ms: int = 40_000
    private_retries_ms: List[int] = field(default_factory=list)


def bucket(
    id: str,
    *,
    label: Optional[str] = None,
    models: Optional[Sequence[str]] = None,
    unit: str = "permille",
    used: Optional[int] = None,
    limit: Optional[int] = None,
    remaining: Optional[int] = None,
    resets_at: Optional[int] = None,
    status: str = "unknown",
    observed_at: Optional[int] = None,
    evidence: Optional[str] = None,
) -> Dict[str, Any]:
    """One bucket of the account's quota, as the host reads it: unknown coverage, refill and status, in permille and reported,
    unless a field says otherwise. `id` starts with the reader's `bucket_prefix`. `remaining` and `limit` count `unit` (`permille`,
    `tokens`, `requests`, `micro_usd`); `resets_at` is Unix seconds and makes the refill a fixed window."""
    fields = {
        "id": id,
        "label": label,
        "models": {"kind": "only", "models": list(models)} if models is not None else {"kind": "unknown"},
        "unit": unit,
        "used": used,
        "limit": limit,
        "remaining": remaining,
        "refill": {"kind": "fixed_window", "resets_at": resets_at} if resets_at is not None else {"kind": "unknown"},
        "status": status,
        "observed_at": int(time.time()) if observed_at is None else observed_at,
        "source": "reported",
        "evidence": evidence,
    }
    # A field left out is absent, not null.
    return {name: data_of(value) for name, value in fields.items() if value is not None}


def reading(buckets: Sequence[Dict[str, Any]]) -> Dict[str, Any]:
    """A complete reading of `buckets`."""
    return {"buckets": list(buckets)}


def failed(why: str, retry: str = Retry.NEVER) -> Dict[str, Any]:
    """No reading, for `why` (one line for a log, never what the agent said), to be asked again as `retry` says."""
    return {"buckets": [], "failure": {"why": why, "retry": retry}}


class UsageReading(kinds.UsageReading):
    """One agent's quota, read as often as the host asks, one call at a time. Subclass it and implement `read`; keep what `opened` is
    told if the figures need it. The host destroys a reading when it is done with it, which lets go of `members`."""

    def __init__(self, members: Taken[kinds.ProcessMembers]) -> None:
        self.members = members

    def opened(self, result: str) -> None:
        """The agent answered a `session/new` or `session/load` request: `result` is its `result` as JSON text."""

    def processes(self, root: int) -> List[int]:
        """`root` and every process below it, plus on Unix the ones in its process group. An agent may sit below a launcher, so
        what it owns is not always a child of the process the host started."""
        with self.members.members(root) as answer:
            return [int(each) for each in answer.to_python()]

    def read(self, root: int, tree: bool) -> ValueAnswer:
        """Reads the figures of the agent whose process is `root`. `tree` says the agent may sit below a launcher, so the whole
        tree is looked through from the first try; otherwise look at the process itself first and widen when nothing is there.
        Answer `reading(...)` or `failed(...)`."""
        raise Unsupported("read is not implemented")


class UsageReader(WithForm, provide.Provider, kinds.UsageReader, abstract=True):
    """Subclass it with `reader` (the id an entry's `usage` names) and `bucket_prefix`, and implement `open`. The timings are in
    milliseconds; each has the default the host's own readers start from."""

    reader: ClassVar[str]
    bucket_prefix: ClassVar[str]
    settle_ms: ClassVar[int] = 10_000
    retries_ms: ClassVar[Sequence[int]] = ()
    cache_ms: ClassVar[int] = 15_000
    private_limit_ms: ClassVar[int] = 40_000
    private_retries_ms: ClassVar[Sequence[int]] = ()

    def info(self) -> ReaderInfo:
        """The description, from the class attributes."""
        return ReaderInfo(
            id=self.reader,
            bucket_prefix=self.bucket_prefix,
            settle_ms=self.settle_ms,
            retries_ms=list(self.retries_ms),
            cache_ms=self.cache_ms,
            private_limit_ms=self.private_limit_ms,
            private_retries_ms=list(self.private_retries_ms),
        )

    def describe(self) -> ValueAnswer:
        return self.info().to_data()

    @abstract
    def open(self, members: Taken[kinds.ProcessMembers]) -> kinds.UsageReading:
        """A reading of one agent's figures, for the connection or entry the host opens it for. `members` is the host's, and
        the reading keeps it."""
        raise Unsupported("open is not implemented")
