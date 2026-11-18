"""The mixin every Canoa provider class takes before `guatiao.provide.Provider`: the class keywords `form` and `abstract`."""

from __future__ import annotations

from typing import Any, Callable, Mapping, Optional, TypeVar

from .forms import form_meta

F = TypeVar("F", bound=Callable[..., Any])


def abstract(method: F) -> F:
    """Marks a slot a base class leaves to its subclasses: Guatiao counts it implemented, so the base is a provider class, and
    `WithForm` refuses a concrete subclass that does not replace it."""
    method.__canoa_abstract__ = True  # type: ignore[attr-defined]
    return method


class WithForm:
    """Gives a provider class two keywords.

    `form` is the settings dataclass whose form (sections, placeholders, units, conditions) the provider carries in its `meta`,
    so any program that draws a guatiao provider's settings draws these. `abstract=True` says the class is a base for others and
    may leave `@abstract` slots in place; any other class that does is a `TypeError` naming them, as Guatiao refuses a missing
    required slot.

    It comes before `provide.Provider` in the bases, which it hands the merged `meta`. The settings' schema still comes from
    the `__init__` annotation, as Guatiao reads it.
    """

    def __init_subclass__(
        cls,
        *,
        form: Optional[type] = None,
        meta: Optional[Mapping[str, Any]] = None,
        abstract: bool = False,
        **keywords: Any,
    ) -> None:
        if form is not None:
            meta = {**(meta or {}), **form_meta(form)}
        # The next class in the bases is guatiao.provide.Provider, which takes `meta`; `object` does not, so the checker cannot see it.
        super().__init_subclass__(meta=meta, **keywords)  # type: ignore[call-arg]
        if not abstract:
            left = sorted(
                name for name in dir(cls) if getattr(getattr(cls, name, None), "__canoa_abstract__", False)
            )
            if left:
                raise TypeError(f"{cls.__name__} does not implement {', '.join(left)}")
