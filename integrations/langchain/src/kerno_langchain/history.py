"""A LangChain chat message history kept in a Kerno database server."""

from __future__ import annotations

import json
from typing import Any, List, Sequence

from kerno_client import FieldDef, KernoClient
from langchain_core.chat_history import BaseChatMessageHistory
from langchain_core.messages import BaseMessage, message_to_dict, messages_from_dict

from ._records import ensure_schema, scan, where_equals

DEFAULT_COLLECTION = "langchain_chat_messages"

FIELDS = [
    FieldDef("session", "text", required=True, indexed=True),
    FieldDef("message", "text", required=True),  # langchain_core's message_to_dict, as JSON
]


class KernoChatMessageHistory(BaseChatMessageHistory):
    """The messages of one session, one typed unit each, read back in the order they were added.

    The order is the order of the unit ids, which are UUIDv7 with a counter: they increase within
    a server run and, across runs, with the server's clock.
    """

    def __init__(self, session_id: str, client: KernoClient, collection: str = DEFAULT_COLLECTION) -> None:
        self.session_id = session_id
        self._kerno = client
        self._collection = collection
        ensure_schema(client, collection, FIELDS)

    @property  # type: ignore[override]
    def messages(self) -> List[BaseMessage]:
        units = scan(self._kerno, self._collection, where_equals("session", self.session_id))
        fields: Any
        return messages_from_dict([json.loads(fields["message"]) for u in units if (fields := u.payload)])

    def add_messages(self, messages: Sequence[BaseMessage]) -> None:
        for message in messages:
            fields = {"session": self.session_id, "message": json.dumps(message_to_dict(message))}
            self._kerno.create_unit(self._collection, fields)

    def clear(self) -> None:
        for unit in scan(self._kerno, self._collection, where_equals("session", self.session_id)):
            self._kerno.delete_unit(unit.id)
