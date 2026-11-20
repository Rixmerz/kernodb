"""The vector store, against the real kerno-server."""

from __future__ import annotations

import math
from pathlib import Path
from typing import Any, List, Optional

import pytest
from conftest import start_server, stop_server
from kerno_client import FieldDef, KernoClient
from kerno_llamaindex import KernoVectorStore
from llama_index.core import Document, StorageContext, VectorStoreIndex
from llama_index.core.base.embeddings.base import BaseEmbedding
from llama_index.core.node_parser import SentenceSplitter
from llama_index.core.schema import NodeRelationship, RelatedNodeInfo, TextNode
from llama_index.core.vector_stores.types import (
    FilterCondition,
    FilterOperator,
    MetadataFilter,
    MetadataFilters,
    VectorStoreQuery,
    VectorStoreQueryMode,
)

VOCAB = ["rust", "python", "memory", "garbage", "graph", "sql", "cat", "dog"]


def embed(text: str) -> List[float]:
    words = text.lower().split()
    return [float(words.count(w)) for w in VOCAB]


class BagOfWords(BaseEmbedding):
    """One dimension per word of VOCAB: the embedding of a text is its word counts."""

    def _get_query_embedding(self, query: str) -> List[float]:
        return embed(query)

    async def _aget_query_embedding(self, query: str) -> List[float]:
        return embed(query)

    def _get_text_embedding(self, text: str) -> List[float]:
        return embed(text)


def node(text: str, node_id: Optional[str] = None, doc: Optional[str] = None, **metadata: Any) -> TextNode:
    n = TextNode(text=text, metadata=metadata, embedding=embed(text))
    if node_id:
        n.id_ = node_id
    if doc:
        n.relationships[NodeRelationship.SOURCE] = RelatedNodeInfo(node_id=doc)
    return n


def query(text: str, k: int = 10, **kwargs: Any) -> VectorStoreQuery:
    return VectorStoreQuery(query_embedding=embed(text), similarity_top_k=k, **kwargs)


def eq(key: str, value: Any) -> MetadataFilters:
    return MetadataFilters(filters=[MetadataFilter(key=key, value=value)])


@pytest.fixture
def store(kerno: KernoClient) -> KernoVectorStore:
    return KernoVectorStore(kerno)


def test_query_ranks_by_cosine_and_returns_that_similarity(store: KernoVectorStore) -> None:
    ids = store.add([node("rust memory", "a"), node("python garbage", "b"), node("rust", "c")])
    assert ids == ["a", "b", "c"]
    result = store.query(query("rust", 3))
    assert result.ids == ["c", "a", "b"]
    assert [n.node_id for n in result.nodes or []] == ["c", "a", "b"]
    assert [round(s, 9) for s in result.similarities or []] == [1.0, round(1 / math.sqrt(2), 9), 0.0]
    assert store.query(query("rust", 1)).ids == ["c"]
    assert store.query(query("rust", 0)).ids == []


def test_ties_keep_the_order_the_nodes_were_added(store: KernoVectorStore) -> None:
    store.add([node("cat", "1"), node("dog", "2"), node("cat dog", "3")])
    assert store.query(query("sql", 3)).ids == ["1", "2", "3"]  # orthogonal to all: every score is 0


def test_a_node_reads_back_whole(store: KernoVectorStore, kerno: KernoClient) -> None:
    n = node("rust memory", "a", doc="doc-1", lang="en", n=3)
    n.excluded_embed_metadata_keys = ["n"]
    store.add([n])
    (got,) = store.query(query("rust", 1)).nodes or []
    assert isinstance(got, TextNode)
    assert (got.node_id, got.get_content(), got.metadata) == ("a", "rust memory", {"lang": "en", "n": 3})
    assert got.ref_doc_id == "doc-1"
    assert got.excluded_embed_metadata_keys == ["n"]
    assert got.embedding is None  # as in the other stores: the vector is not read back with the node
    assert [x.node_id for x in store.get_nodes(["a"])] == ["a"]
    (unit,) = kerno.iter_units(type="llamaindex_nodes")
    assert "rust memory" not in unit.payload["node"]  # the text is stored once, in its own field
    assert store.get_nodes(["nope"]) == []


def test_a_node_id_that_is_stored_is_replaced_not_duplicated(store: KernoVectorStore, kerno: KernoClient) -> None:
    store.add([node("rust", "same")])
    store.add([node("python", "same", extra=1)])
    assert [n.get_content() for n in store.get_nodes()] == ["python"]
    assert store.query(query("python", 5)).ids == ["same"]
    (unit,) = list(kerno.iter_units(type="llamaindex_nodes"))
    assert unit.revision == 2


def test_delete_removes_every_node_of_a_document(store: KernoVectorStore) -> None:
    store.add([node("rust", "1", doc="d1"), node("python", "2", doc="d1"), node("cat", "3", doc="d2"), node("dog", "4")])
    store.delete("d1")
    assert sorted(store.query(query("rust")).ids or []) == ["3", "4"]
    store.delete("unknown")  # nothing to delete is not an error
    assert sorted(store.query(query("rust")).ids or []) == ["3", "4"]


def test_ids_with_quotes_and_backslashes_are_found_and_deleted(store: KernoVectorStore) -> None:
    odd = ['a"b', "a\\b", 'x" OR d.node_id != "', "line\nbreak\ttab\r\0nul"]
    store.add([node("rust", i, doc=i) for i in odd] + [node("python", "plain", doc="plain")])
    assert [n.node_id for n in store.get_nodes([odd[2]])] == [odd[2]]
    store.add([node("rust again", odd[0], doc=odd[0])])  # an upsert finds the node by its odd id
    assert len(store.get_nodes()) == 5
    for i in odd:
        store.delete(i)
    assert store.query(query("rust")).ids == ["plain"]


def test_delete_nodes_by_ids_and_by_filters_and_clear(store: KernoVectorStore) -> None:
    store.add([node("rust", "1", lang="en"), node("rust", "2", lang="es"), node("rust", "3", lang="en"), node("rust", "4")])
    store.delete_nodes(node_ids=["1", "nope"])
    store.delete_nodes(filters=eq("lang", "es"))
    store.delete_nodes(node_ids=["3", "4"], filters=eq("lang", "fr"))  # both must hold: none does
    assert sorted(n.node_id for n in store.get_nodes()) == ["3", "4"]
    with pytest.raises(ValueError, match="node_ids or filters"):
        store.delete_nodes()
    store.delete_nodes(node_ids=[])  # an empty list of ids deletes nothing (it must not read as "all")
    assert sorted(n.node_id for n in store.get_nodes()) == ["3", "4"]
    store.clear()
    assert store.get_nodes() == []
    assert store.query(query("rust")).ids == []


def test_filters_doc_ids_and_node_ids_narrow_a_query(store: KernoVectorStore) -> None:
    store.add(
        [node("rust a", "1", doc="d1", lang="en", n=1), node("rust b", "2", doc="d1", lang="es", n=2),
         node("rust c", "3", doc="d2", lang="en", n=3), node("rust d", "4", n=4)]
    )
    ids = lambda **kw: store.query(query("rust", **kw)).ids  # noqa: E731
    assert ids(filters=eq("lang", "en")) == ["1", "3"]
    assert ids(doc_ids=["d1"]) == ["1", "2"]
    assert ids(doc_ids=["d1", "d2"], filters=eq("lang", "en")) == ["1", "3"]
    assert ids(node_ids=["4", "2", "nope"]) == ["2", "4"]
    assert ids(node_ids=[], doc_ids=[]) == ["1", "2", "3", "4"]  # empty restricts nothing, as in as_retriever()
    gt = MetadataFilters(filters=[MetadataFilter(key="n", value=2, operator=FilterOperator.GT)])
    assert ids(filters=gt) == ["3", "4"]
    either = MetadataFilters(
        filters=[MetadataFilter(key="lang", value="es"), MetadataFilter(key="n", value=4)], condition=FilterCondition.OR
    )
    assert ids(filters=either) == ["2", "4"]
    assert [n.node_id for n in store.get_nodes(filters=eq("lang", "en"))] == ["1", "3"]
    assert [n.node_id for n in store.get_nodes(["1", "2"], eq("lang", "es"))] == ["2"]


def test_what_the_store_cannot_do_is_refused_not_ignored(store: KernoVectorStore) -> None:
    store.add([node("rust", "1")])
    for mode in (VectorStoreQueryMode.HYBRID, VectorStoreQueryMode.TEXT_SEARCH, VectorStoreQueryMode.MMR):
        with pytest.raises(NotImplementedError, match="query mode"):
            store.query(query("rust", mode=mode))
    with pytest.raises(NotImplementedError, match="output_fields"):
        store.query(query("rust", output_fields=["x"]))
    with pytest.raises(ValueError, match="query_embedding"):
        store.query(VectorStoreQuery(query_str="rust"))
    with pytest.raises(ValueError):  # a vector of another size than the stored ones
        store.query(VectorStoreQuery(query_embedding=[1.0, 2.0]))
    with pytest.raises(ValueError):  # a node without an embedding
        store.add([TextNode(text="x")])
    with pytest.raises(TypeError):
        store.add([node("x", "2")], index_name="y")
    with pytest.raises(TypeError):
        store.query(query("rust"), vector_store_kwargs={})
    with pytest.raises(TypeError):
        store.delete("d", x=1)


def test_the_collection_is_not_capped_at_a_page(store: KernoVectorStore) -> None:
    n = 1100  # the server pages units at 1000
    store.add([node(f"doc{i}", f"id{i}") for i in range(n)])
    assert len(store.query(query("rust", n + 5)).ids or []) == n


def test_two_stores_share_a_collection_and_a_foreign_schema_is_refused(kerno: KernoClient) -> None:
    first, second = KernoVectorStore(kerno), KernoVectorStore(kerno)
    first.add([node("rust", "x")])
    assert [n.node_id for n in second.get_nodes()] == ["x"]
    assert KernoVectorStore(kerno, collection="other").get_nodes() == []
    kerno.define_schema("taken", [FieldDef("x", "int")])
    with pytest.raises(ValueError, match="already has a schema"):
        KernoVectorStore(kerno, collection="taken")


def test_nodes_survive_a_restart_of_the_server(server_binary: str, tmp_path: Path) -> None:
    db, log = tmp_path / "db", tmp_path / "log"
    first = start_server(server_binary, db, log)
    KernoVectorStore(KernoClient(first.url)).add([node("rust memory", "keep", doc="d", a=1)])
    assert stop_server(first) == 0
    second = start_server(server_binary, db, log)
    try:
        result = KernoVectorStore(KernoClient(second.url)).query(query("rust", 1))
        assert result.ids == ["keep"]
        assert result.similarities == [pytest.approx(1 / math.sqrt(2))]
        assert result.nodes and result.nodes[0].metadata == {"a": 1}
    finally:
        second.process.kill()
        second.process.wait()


def test_a_vector_store_index_runs_on_it(store: KernoVectorStore) -> None:
    splitter = SentenceSplitter(chunk_size=64, chunk_overlap=0, tokenizer=str.split)  # no tiktoken: offline
    docs = [Document(text="rust memory", doc_id="d1"), Document(text="python garbage", doc_id="d2")]
    index = VectorStoreIndex.from_documents(
        docs,
        storage_context=StorageContext.from_defaults(vector_store=store),
        embed_model=BagOfWords(),
        transformations=[splitter],
    )
    hits = index.as_retriever(similarity_top_k=1).retrieve("rust")
    assert [h.node.get_content() for h in hits] == ["rust memory"]
    assert hits[0].score == pytest.approx(1 / math.sqrt(2))
    index.delete_ref_doc("d1")
    assert [n.get_content() for n in store.get_nodes()] == ["python garbage"]
