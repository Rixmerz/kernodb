"""The vector store, against the real kerno-server."""

from __future__ import annotations

import math
import uuid
from pathlib import Path
from typing import List

import pytest
from conftest import Running, start_server, stop_server
from kerno_client import FieldDef, KernoClient
from kerno_langchain import KernoVectorStore
from langchain_core.documents import Document
from langchain_core.embeddings import Embeddings

VOCAB = ["rust", "python", "memory", "garbage", "graph", "sql", "cat", "dog"]


class BagOfWords(Embeddings):
    """One dimension per word of VOCAB: the embedding of a text is its word counts."""

    def embed_documents(self, texts: List[str]) -> List[List[float]]:
        return [self.embed_query(t) for t in texts]

    def embed_query(self, text: str) -> List[float]:
        words = text.lower().split()
        return [float(words.count(w)) for w in VOCAB]


@pytest.fixture
def store(kerno: KernoClient) -> KernoVectorStore:
    return KernoVectorStore(BagOfWords(), kerno)


def test_search_ranks_by_cosine_and_returns_that_similarity(store: KernoVectorStore) -> None:
    store.add_texts(["rust memory", "python garbage", "rust"])
    hits = store.similarity_search_with_score("rust", k=3)
    assert [d.page_content for d, _ in hits] == ["rust", "rust memory", "python garbage"]
    assert [round(s, 9) for _, s in hits] == [1.0, round(1 / math.sqrt(2), 9), 0.0]
    assert [d.page_content for d in store.similarity_search("rust", k=1)] == ["rust"]
    by_vector = store.similarity_search_by_vector([0, 0, 0, 0, 0, 0, 1.0, 0], k=2)
    assert len(by_vector) == 2
    assert store.similarity_search("rust", k=0) == []


def test_ties_keep_the_order_the_documents_were_added(store: KernoVectorStore) -> None:
    store.add_texts(["cat", "dog", "cat dog"])
    # "sql" is orthogonal to all of them: every score is 0.
    assert [d.page_content for d in store.similarity_search("sql", k=3)] == ["cat", "dog", "cat dog"]


def test_relevance_scores_are_between_0_and_1(store: KernoVectorStore) -> None:
    store.add_texts(["rust", "python"])
    scores = [s for _, s in store.similarity_search_with_relevance_scores("rust", k=2)]
    assert scores == [1.0, 0.5]


def test_ids_documents_and_metadata_round_trip(store: KernoVectorStore) -> None:
    ids = store.add_documents(
        [Document(id="a", page_content="rust", metadata={"n": 1, "tags": ["x", "é"], "nested": {"k": None}}),
         Document(page_content="python")]
    )
    assert ids[0] == "a" and str(uuid.UUID(ids[1])) == ids[1]
    found = store.get_by_ids([ids[1], "a", "a", "missing"])
    assert len(found) == 2  # a repeated id is returned once, an unknown one not at all
    got = {d.id: d for d in found}
    assert set(got) == set(ids)
    assert got["a"].metadata == {"n": 1, "tags": ["x", "é"], "nested": {"k": None}}
    assert got["a"].page_content == "rust"
    # The ids argument wins over Document.id.
    assert store.add_texts(["cat"], ids=["from-ids"]) == ["from-ids"]
    assert store.add_documents([Document(id="doc-id", page_content="dog")], ids=["arg-id"]) == ["arg-id"]
    assert [d.id for d in store.get_by_ids(["doc-id", "arg-id"])] == ["arg-id"]
    with pytest.raises(ValueError, match="2 ids for 1 documents"):
        store.add_documents([Document(page_content="x")], ids=["1", "2"])


def test_an_id_that_is_stored_is_replaced_not_duplicated(store: KernoVectorStore, kerno: KernoClient) -> None:
    store.add_texts(["rust"], ids=["same"])
    store.add_texts(["python"], metadatas=[{"v": 2}], ids=["same"])
    assert [d.page_content for d in store.similarity_search("python", k=5)] == ["python"]
    (doc,) = store.get_by_ids(["same"])
    assert doc.metadata == {"v": 2}
    (unit,) = list(kerno.iter_units(type="langchain_documents"))
    assert unit.revision == 2


def test_an_id_with_quotes_and_backslashes_is_found_and_deleted(store: KernoVectorStore) -> None:
    odd = ['a"b', "a\\b", 'x" OR d.doc_id != "', "line\nbreak\ttab\r\0nul"]
    store.add_texts(["rust"] * len(odd), ids=odd)
    store.add_texts(["python"], ids=["plain"])
    assert [d.id for d in store.get_by_ids([odd[2]])] == [odd[2]]
    assert sorted(d.id for d in store.get_by_ids(odd)) == sorted(odd)
    store.delete(odd)
    assert [d.id for d in store.similarity_search("rust", k=10)] == ["plain"]


def test_delete_skips_unknown_ids_and_without_ids_deletes_everything(store: KernoVectorStore) -> None:
    store.add_texts(["rust", "python", "cat"], ids=["1", "2", "3"])
    assert store.delete(["1", "nope"]) is True
    assert sorted(d.id for d in store.similarity_search("rust", k=10)) == ["2", "3"]
    assert store.delete() is True
    assert store.similarity_search("rust", k=10) == []
    with pytest.raises(TypeError):
        store.delete(["2"], where="x")


def test_filter_keeps_the_documents_whose_metadata_equals(store: KernoVectorStore) -> None:
    store.add_texts(
        ["rust a", "rust b", "rust c"],
        metadatas=[{"lang": "en", "n": 1}, {"lang": "es", "n": 1}, {"n": 2}],
    )
    texts = lambda f: [d.page_content for d in store.similarity_search("rust", k=10, filter=f)]  # noqa: E731
    assert texts({"lang": "es"}) == ["rust b"]
    assert texts({"n": 1}) == ["rust a", "rust b"]
    assert texts({"n": 1, "lang": "en"}) == ["rust a"]
    assert texts({"lang": "fr"}) == []
    assert texts({}) == texts(None)
    with pytest.raises(TypeError):
        store.similarity_search("rust", filter="lang")  # type: ignore[arg-type]


def test_a_vector_of_another_size_is_refused(store: KernoVectorStore) -> None:
    store.add_texts(["rust"])
    with pytest.raises(ValueError, match="different sizes"):
        store.similarity_search_by_vector([1.0, 2.0])


def test_the_collection_is_not_capped_at_a_page(store: KernoVectorStore) -> None:
    n = 1100  # the server pages units at 1000
    store.add_texts([f"doc{i}" for i in range(n)])
    assert len(store.similarity_search("rust", k=n + 5)) == n


def test_from_texts_and_as_retriever(kerno: KernoClient) -> None:
    store = KernoVectorStore.from_texts(
        ["rust memory", "python"], BagOfWords(), [{"i": 0}, {"i": 1}], ids=["r", "p"], client=kerno, collection="notes"
    )
    retriever = store.as_retriever(search_kwargs={"k": 1, "filter": {"i": 0}})
    assert [d.id for d in retriever.invoke("rust")] == ["r"]
    # Another collection is another unit type: nothing of "notes" is in the default one.
    assert KernoVectorStore(BagOfWords(), kerno).similarity_search("rust") == []


def test_two_stores_on_one_collection_share_it_and_a_foreign_schema_is_refused(kerno: KernoClient) -> None:
    first = KernoVectorStore(BagOfWords(), kerno)
    second = KernoVectorStore(BagOfWords(), kerno)
    first.add_texts(["rust"], ids=["x"])
    assert [d.id for d in second.get_by_ids(["x"])] == ["x"]
    kerno.define_schema("other", [FieldDef("x", "int")])
    with pytest.raises(ValueError, match="already has a schema"):
        KernoVectorStore(BagOfWords(), kerno, collection="other")


def test_an_unreachable_server_is_an_error_not_an_empty_store() -> None:
    from kerno_client import TransportError

    with pytest.raises(TransportError):
        KernoVectorStore(BagOfWords(), KernoClient("http://127.0.0.1:1", timeout=2))


def test_documents_survive_a_restart_of_the_server(server_binary: str, tmp_path: Path) -> None:
    db, log = tmp_path / "db", tmp_path / "log"
    first: Running = start_server(server_binary, db, log)
    KernoVectorStore(BagOfWords(), KernoClient(first.url)).add_texts(["rust memory"], [{"a": 1}], ids=["keep"])
    assert stop_server(first) == 0
    second = start_server(server_binary, db, log)
    try:
        reopened = KernoVectorStore(BagOfWords(), KernoClient(second.url))  # the schema is there: no error
        (doc, score), = reopened.similarity_search_with_score("rust", k=1)
        assert (doc.id, doc.metadata) == ("keep", {"a": 1})
        assert score == pytest.approx(1 / math.sqrt(2))
    finally:
        second.process.kill()
        second.process.wait()
