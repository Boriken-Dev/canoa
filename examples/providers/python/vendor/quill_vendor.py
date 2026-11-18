"""An agent vendor written in Python: who the agent is and how Canoa shows it. Listing it in canoa.cfg (an entry's source) is all it takes
for the host and the app to offer the agent, with no change to Canoa.
"""

from __future__ import annotations

from canoa_sdk import AgentVendor, ModeTitle, StandardTitle

__version__ = "1.0.0"
__all__ = ["Quill"]

MARK = (
    "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 24 24'>"
    "<path d='M3 3h18L3 21h18' fill='none' stroke='#2563EB' stroke-width='2'/></svg>"
)


class Quill(AgentVendor):
    """The vendor of an agent called Quill: the id its harness has, its name, the credential standards that are its own."""

    agent = "quill"
    title = "Quill"
    program = "Quill Agent"
    vendor = "quill-labs"
    aos = "quill"
    model_variable = "QUILL_MODEL"
    standards = [StandardTitle("quill-key", "Quill API key")]
    names = ["quill-agent"]
    modes = [ModeTitle("ask", "Ask"), ModeTitle("code", "Code")]
    mark = MARK
