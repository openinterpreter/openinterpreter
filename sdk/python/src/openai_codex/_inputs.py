from __future__ import annotations

from collections.abc import Sequence
from dataclasses import dataclass

from .generated.v2_all import FunctionCallOutputContentItem, TurnToolOutput
from .models import JsonObject


@dataclass(slots=True)
class TextInput:
    """Text supplied to a turn or steering request."""

    text: str


@dataclass(slots=True)
class ImageInput:
    """Image data URL supplied as turn input."""

    url: str


@dataclass(slots=True)
class LocalImageInput:
    """Local image path supplied as turn input."""

    path: str


@dataclass(slots=True)
class SkillInput:
    """Named skill reference supplied as turn input."""

    name: str
    path: str


@dataclass(slots=True)
class MentionInput:
    """Named resource mention supplied as turn input."""

    name: str
    path: str


@dataclass(slots=True)
class ExternalMessage:
    """Untrusted content supplied by another agent, tool, or application.

    Content has tool-level authority, below user and developer instructions. It
    does not establish user authorization or approval. Pass this as the whole
    input to ``thread.run()`` or ``thread.turn()`` to start a turn or join an
    active regular turn. ``tool_name`` identifies the tool delivering it.

    ``content`` accepts text or Responses-compatible function-output content
    items. Structured items can be dictionaries; no generated wrapper is needed.
    """

    tool_name: str
    content: str | Sequence[JsonObject | FunctionCallOutputContentItem]
    namespace: str | None = None


InputItem = TextInput | ImageInput | LocalImageInput | SkillInput | MentionInput
Input = list[InputItem] | InputItem
RunInput = Input | str | ExternalMessage


def _to_wire_item(item: InputItem) -> JsonObject:
    if isinstance(item, TextInput):
        return {"type": "text", "text": item.text}
    if isinstance(item, ImageInput):
        return {"type": "image", "url": item.url}
    if isinstance(item, LocalImageInput):
        return {"type": "localImage", "path": item.path}
    if isinstance(item, SkillInput):
        return {"type": "skill", "name": item.name, "path": item.path}
    if isinstance(item, MentionInput):
        return {"type": "mention", "name": item.name, "path": item.path}
    raise TypeError(f"unsupported input item: {type(item)!r}")


def _to_wire_input(input: Input) -> list[JsonObject]:
    if isinstance(input, list):
        return [_to_wire_item(i) for i in input]
    return [_to_wire_item(input)]


def _normalize_run_input(input: Input | str) -> Input:
    if isinstance(input, str):
        return TextInput(input)
    return input


def _to_wire_turn_input(input: RunInput) -> tuple[list[JsonObject], TurnToolOutput | None]:
    if isinstance(input, ExternalMessage):
        if not isinstance(input.tool_name, str) or not input.tool_name.strip():
            raise ValueError("ExternalMessage.tool_name must be a nonempty string")
        return [], TurnToolOutput.model_validate(
            {"name": input.tool_name, "namespace": input.namespace, "output": input.content}
        )
    return _to_wire_input(_normalize_run_input(input)), None
