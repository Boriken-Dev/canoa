"""coqui-acp's provider kinds, which Canoa loads as they are: an `acp_harness` runs an agent, an `agent_provider` serves one, and an
account provider keeps its logins.

coqui-acp ships these as typed `Protocol` modules (`coqui_acp_api_kinds`, `coqui_acp_provider_kinds`) next to its crates; a guatiao
source puts the folders on `sys.path` (`paths`). They are re-exported here, not written again, so a provider that is both a Canoa
and an ACP kind imports from one place. Importing this module without them is an `ImportError` naming where to find them.
"""

from __future__ import annotations

try:
    from coqui_acp_api_kinds import (  # type: ignore[import-not-found]
        AcpHarness,
        AccountKeeper,
        AccountProvider,
        AccountWatch,
        Channel,
        ChannelSink,
        LoginPrompter,
    )
    from coqui_acp_provider_kinds import AgentProvider, AgentSession  # type: ignore[import-not-found]
except ImportError as error:  # pragma: no cover - the message is the behaviour
    raise ImportError(
        "canoa_sdk.acp needs coqui-acp's `coqui_acp_api_kinds` and `coqui_acp_provider_kinds` on sys.path "
        "(the entry's source_settings `paths`): they are in coqui-acp's crates/coqui-acp-api/python and "
        "crates/coqui-acp-provider/python"
    ) from error

__all__ = [
    "AccountKeeper",
    "AccountProvider",
    "AccountWatch",
    "AcpHarness",
    "AgentProvider",
    "AgentSession",
    "Channel",
    "ChannelSink",
    "LoginPrompter",
]
