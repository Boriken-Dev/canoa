"""An echo agent that prints, as Python authors do: while it is imported, and during every prompt.

What it prints is what a client must never see: a plain line, and a line shaped like an ACP notification. A prompt of
`flood` prints far more than the host keeps.
"""

from __future__ import annotations

import os
import queue
import sys
from typing import Any, Dict, Optional

from coqui_acp_provider_kinds import AgentProvider, AgentSession

from guatiao import Ref, provide

__version__ = "1.0.0"
__all__ = ["Echo"]

FORGED = (
    '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"forged","update":'
    '{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"forged by a print"}}}}'
)

print("printed while importing")
sys.stdout.flush()


class Conversation(AgentSession):
    def __init__(self, identity: str) -> None:
        self.identity = identity
        self.events: "queue.Queue[Dict[str, Any]]" = queue.Queue()

    def answer(self, text: str) -> None:
        print("printed during a prompt")
        print(FORGED)
        sys.stdout.flush()
        os.write(1, b"written to file descriptor 1\n")
        print("printed to standard error", file=sys.stderr)
        if text == "flood":
            line = "x" * 1023 + "\n"
            for _ in range(4096):
                sys.stdout.write(line)
            sys.stdout.flush()
        self.events.put({"kind": "text", "text": f"printing: {text}"})
        self.events.put({"kind": "completed"})

    def session_id(self) -> Optional[str]:
        return self.identity

    def send_message(self, message: Ref) -> None:
        self.answer(message.to_python()["text"])

    def next_event(self, timeout_ms: int) -> Optional[Dict[str, Any]]:
        try:
            return self.events.get(timeout=timeout_ms / 1000)
        except queue.Empty:
            return None

    def interrupt(self) -> None:
        self.events.put({"kind": "cancelled"})

    def close_(self) -> None:
        pass


class Echo(provide.Provider, AgentProvider, display_name="An echo that prints"):
    def start(self, request: Ref) -> Conversation:
        print("printed while starting")
        conversation = Conversation("printing-1")
        conversation.answer(request.to_python()["prompt"])
        return conversation
