"""Plain-data models: what a kind's slot takes or answers, as dataclasses a provider fills in."""

from __future__ import annotations

import dataclasses
from typing import Any, Dict


def data_of(value: Any) -> Any:
    """`value` as the plain data a guatiao slot answers: a model as a map with its absent (`None`) fields left out, lists and
    maps by their members."""
    if dataclasses.is_dataclass(value) and not isinstance(value, type):
        return {
            field.name: data_of(getattr(value, field.name))
            for field in dataclasses.fields(value)
            if getattr(value, field.name) is not None
        }
    if isinstance(value, (list, tuple)):
        return [data_of(each) for each in value]
    if isinstance(value, dict):
        return {key: data_of(each) for key, each in value.items()}
    return value


class Model:
    """A dataclass that is a model of a kind."""

    def to_data(self) -> Dict[str, Any]:
        """The model as the plain map the host reads."""
        result: Dict[str, Any] = data_of(self)
        return result
