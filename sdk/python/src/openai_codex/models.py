from __future__ import annotations

from dataclasses import dataclass
from typing import TypeAlias

from pydantic import BaseModel

from .generated.notification_registry import KnownNotificationPayload as _KnownNotificationPayload

# Preserve the notification names previously importable from this module.
from .generated.v2_all import (
    AccountLoginCompletedNotification as AccountLoginCompletedNotification,
    AccountRateLimitsUpdatedNotification as AccountRateLimitsUpdatedNotification,
    AccountUpdatedNotification as AccountUpdatedNotification,
    AgentMessageDeltaNotification as AgentMessageDeltaNotification,
    AppListUpdatedNotification as AppListUpdatedNotification,
    CommandExecutionOutputDeltaNotification as CommandExecutionOutputDeltaNotification,
    ConfigWarningNotification as ConfigWarningNotification,
    ContextCompactedNotification as ContextCompactedNotification,
    DeprecationNoticeNotification as DeprecationNoticeNotification,
    ErrorNotification as ErrorNotification,
    FileChangeOutputDeltaNotification as FileChangeOutputDeltaNotification,
    ItemCompletedNotification as ItemCompletedNotification,
    ItemStartedNotification as ItemStartedNotification,
    McpServerOauthLoginCompletedNotification as McpServerOauthLoginCompletedNotification,
    McpToolCallProgressNotification as McpToolCallProgressNotification,
    PlanDeltaNotification as PlanDeltaNotification,
    RawResponseItemCompletedNotification as RawResponseItemCompletedNotification,
    ReasoningSummaryPartAddedNotification as ReasoningSummaryPartAddedNotification,
    ReasoningSummaryTextDeltaNotification as ReasoningSummaryTextDeltaNotification,
    ReasoningTextDeltaNotification as ReasoningTextDeltaNotification,
    TerminalInteractionNotification as TerminalInteractionNotification,
    ThreadGoalClearedNotification as ThreadGoalClearedNotification,
    ThreadGoalUpdatedNotification as ThreadGoalUpdatedNotification,
    ThreadNameUpdatedNotification as ThreadNameUpdatedNotification,
    ThreadStartedNotification as ThreadStartedNotification,
    ThreadTokenUsageUpdatedNotification as ThreadTokenUsageUpdatedNotification,
    TurnCompletedNotification as TurnCompletedNotification,
    TurnDiffUpdatedNotification as TurnDiffUpdatedNotification,
    TurnPlanUpdatedNotification as TurnPlanUpdatedNotification,
    TurnStartedNotification as TurnStartedNotification,
    WindowsWorldWritableWarningNotification as WindowsWorldWritableWarningNotification,
)

JsonScalar: TypeAlias = str | int | float | bool | None
JsonValue: TypeAlias = JsonScalar | dict[str, "JsonValue"] | list["JsonValue"]
JsonObject: TypeAlias = dict[str, JsonValue]


@dataclass(slots=True)
class UnknownNotification:
    params: JsonObject


# Preserve the existing raw-item type, which app-server omits from its notification schema.
NotificationPayload: TypeAlias = (
    _KnownNotificationPayload | RawResponseItemCompletedNotification | UnknownNotification
)


@dataclass(slots=True)
class Notification:
    method: str
    payload: NotificationPayload


class ServerInfo(BaseModel):
    name: str | None = None
    version: str | None = None


class InitializeResponse(BaseModel):
    serverInfo: ServerInfo | None = None
    userAgent: str | None = None
    platformFamily: str | None = None
    platformOs: str | None = None
