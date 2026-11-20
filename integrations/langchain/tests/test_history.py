"""The chat message history, against the real kerno-server."""

from __future__ import annotations

from pathlib import Path

from conftest import start_server, stop_server
from kerno_client import KernoClient
from kerno_langchain import KernoChatMessageHistory
from langchain_core.messages import AIMessage, HumanMessage, SystemMessage


def test_messages_come_back_in_order_with_their_kind_and_extras(kerno: KernoClient) -> None:
    history = KernoChatMessageHistory("s1", kerno)
    assert history.messages == []
    history.add_messages([SystemMessage(content="be brief"), HumanMessage(content="hi", additional_kwargs={"k": 1})])
    history.add_user_message("again")
    history.add_ai_message("hello")
    many = [HumanMessage(content=str(i)) for i in range(40)]
    history.add_messages(many)  # in the same millisecond, mostly
    got = history.messages
    assert [type(m) for m in got[:4]] == [SystemMessage, HumanMessage, HumanMessage, AIMessage]
    assert [m.content for m in got] == ["be brief", "hi", "again", "hello"] + [str(i) for i in range(40)]
    assert got[1].additional_kwargs == {"k": 1}


def test_sessions_are_isolated_and_clear_only_clears_its_own(kerno: KernoClient) -> None:
    a, b = KernoChatMessageHistory("a", kerno), KernoChatMessageHistory('b" OR 1 = 1 "\\', kerno)
    a.add_user_message("from a")
    b.add_user_message("from b")
    assert [m.content for m in a.messages] == ["from a"]
    assert [m.content for m in b.messages] == ["from b"]
    b.clear()
    assert b.messages == []
    assert [m.content for m in a.messages] == ["from a"]
    a.clear()
    a.add_user_message("after clear")
    assert [m.content for m in a.messages] == ["after clear"]


def test_a_history_survives_a_restart_of_the_server(server_binary: str, tmp_path: Path) -> None:
    db, log = tmp_path / "db", tmp_path / "log"
    first = start_server(server_binary, db, log)
    KernoChatMessageHistory("s", KernoClient(first.url)).add_user_message("remember me")
    assert stop_server(first) == 0
    second = start_server(server_binary, db, log)
    try:
        assert [m.content for m in KernoChatMessageHistory("s", KernoClient(second.url)).messages] == ["remember me"]
    finally:
        second.process.kill()
        second.process.wait()


def test_it_works_as_the_memory_of_a_runnable(kerno: KernoClient) -> None:
    from langchain_core.language_models import FakeListChatModel
    from langchain_core.runnables.history import RunnableWithMessageHistory

    chain = RunnableWithMessageHistory(
        FakeListChatModel(responses=["first answer", "second answer"]),
        lambda session_id: KernoChatMessageHistory(session_id, kerno),
    )
    config = {"configurable": {"session_id": "chat"}}
    chain.invoke("question one", config)  # type: ignore[arg-type]
    chain.invoke("question two", config)  # type: ignore[arg-type]
    assert [(type(m).__name__, m.content) for m in KernoChatMessageHistory("chat", kerno).messages] == [
        ("HumanMessage", "question one"),
        ("AIMessage", "first answer"),
        ("HumanMessage", "question two"),
        ("AIMessage", "second answer"),
    ]
