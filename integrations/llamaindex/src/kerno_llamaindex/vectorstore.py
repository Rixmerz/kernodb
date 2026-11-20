"""A LlamaIndex vector store whose nodes live in a Kerno database server."""

from __future__ import annotations

import heapq
import json
from typing import Any, Dict, List, Optional, Sequence

from kerno_client import FieldDef, KernoClient, Unit
from llama_index.core.schema import BaseNode, MetadataMode
from llama_index.core.vector_stores.types import (
    BasePydanticVectorStore,
    MetadataFilters,
    VectorStoreQuery,
    VectorStoreQueryMode,
    VectorStoreQueryResult,
)
from llama_index.core.vector_stores.utils import build_metadata_filter_fn, metadata_dict_to_node, node_to_metadata_dict
from pydantic import PrivateAttr

from ._records import cosine, ensure_schema, pack, scan, unpack, where_equals

DEFAULT_COLLECTION = "llamaindex_nodes"

FIELDS = [
    FieldDef("node_id", "text", required=True, unique=True),
    FieldDef("ref_doc_id", "text", indexed=True),  # absent for a node of no document
    FieldDef("content", "text", required=True),
    FieldDef("node", "text", required=True),  # node_to_metadata_dict(node), as JSON
    FieldDef("embedding", "bytes", required=True),
]


class KernoVectorStore(BasePydanticVectorStore):
    """Nodes, their text and their embeddings, stored as typed units of one Kerno collection.

    Search is exact: it reads the collection and ranks it by cosine similarity, which is also the
    ``similarities`` it returns. The server keeps no vector index yet, so the cost of a query is
    the size of the collection. Only the default query mode is supported; any other mode raises.

    ``add`` upserts: a node id that is already stored replaces that node (a new revision of its
    unit, so Kerno keeps the old one in its history).
    """

    # ponytail: a query reads the whole collection and ranks it in Python (measured: 2000 vectors of
    # 1536 dimensions in 1.4 s, linear in both); fine to a few thousand items, then switch
    # to the server's vector leg or an index when kerno-server grows one.

    stores_text: bool = True
    collection: str = DEFAULT_COLLECTION
    _client: KernoClient = PrivateAttr()

    def __init__(self, client: KernoClient, collection: str = DEFAULT_COLLECTION) -> None:
        super().__init__(collection=collection)
        self._client = client
        ensure_schema(client, collection, FIELDS)

    @classmethod
    def class_name(cls) -> str:
        return "KernoVectorStore"

    @property
    def client(self) -> KernoClient:
        return self._client

    # ------------------------------------------------------------------ writing

    def add(self, nodes: Sequence[BaseNode], **add_kwargs: Any) -> List[str]:
        _no_kwargs(add_kwargs)
        for node in nodes:
            fields: Dict[str, Any] = {
                "node_id": node.node_id,
                "content": node.get_content(metadata_mode=MetadataMode.NONE),
                "node": json.dumps(node_to_metadata_dict(node, remove_text=True), ensure_ascii=False),
                "embedding": pack(node.get_embedding()),
            }
            if node.ref_doc_id is not None:
                fields["ref_doc_id"] = node.ref_doc_id
            existing = self._find("node_id", node.node_id)
            if existing:
                self._client.update_unit(existing[0].id, fields)
            else:
                self._client.create_unit(self.collection, fields)
        return [node.node_id for node in nodes]

    def delete(self, ref_doc_id: str, **delete_kwargs: Any) -> None:
        """Deletes every node of the document ``ref_doc_id``."""
        _no_kwargs(delete_kwargs)
        self._delete(self._find("ref_doc_id", ref_doc_id))

    def delete_nodes(
        self, node_ids: Optional[List[str]] = None, filters: Optional[MetadataFilters] = None, **delete_kwargs: Any
    ) -> None:
        """Deletes the nodes with these ids that match the filters; at least one of the two is needed."""
        _no_kwargs(delete_kwargs)
        if node_ids is None and filters is None:
            raise ValueError("give node_ids or filters; clear() deletes everything")
        self._delete(self._select(node_ids, None, filters))

    def clear(self) -> None:
        self._delete(scan(self._client, self.collection))

    # ------------------------------------------------------------------ reading

    def get_nodes(self, node_ids: Optional[List[str]] = None, filters: Optional[MetadataFilters] = None) -> List[BaseNode]:
        return [_node(u) for u in self._select(node_ids, None, filters)]

    def query(self, query: VectorStoreQuery, **kwargs: Any) -> VectorStoreQueryResult:
        _no_kwargs(kwargs)
        if query.mode != VectorStoreQueryMode.DEFAULT:
            raise NotImplementedError(f"query mode {query.mode.value!r}: only 'default' is supported")
        if query.output_fields or query.embedding_field:
            raise NotImplementedError("output_fields and embedding_field are not supported")
        if query.query_embedding is None:
            raise ValueError("a query needs query_embedding")
        # An empty list restricts nothing: VectorStoreIndex.as_retriever() sends node_ids=[] for a
        # store that keeps its own text, and the other stores read it that way.
        units = self._select(query.node_ids or None, query.doc_ids or None, query.filters)
        scored = [(cosine(query.query_embedding, unpack(_fields(u)["embedding"])), u) for u in units]
        best = heapq.nlargest(query.similarity_top_k, scored, key=lambda pair: pair[0])
        return VectorStoreQueryResult(
            nodes=[_node(u) for _, u in best],
            similarities=[s for s, _ in best],
            ids=[_fields(u)["node_id"] for _, u in best],
        )

    # ----------------------------------------------------------------- plumbing

    def _find(self, field: str, value: str) -> List[Unit]:
        return scan(self._client, self.collection, where_equals(field, value))

    def _select(
        self, node_ids: Optional[List[str]], doc_ids: Optional[List[str]], filters: Optional[MetadataFilters]
    ) -> List[Unit]:
        """The units in the ids and doc ids given and matching the filters, in creation order."""
        units = scan(self._client, self.collection)
        if node_ids is not None:
            units = [u for u in units if _fields(u)["node_id"] in node_ids]
        if doc_ids is not None:
            units = [u for u in units if _fields(u).get("ref_doc_id") in doc_ids]
        if filters is not None:
            by_id = {_fields(u)["node_id"]: json.loads(_fields(u)["node"]) for u in units}
            matches = build_metadata_filter_fn(by_id.__getitem__, filters)
            units = [u for u in units if matches(_fields(u)["node_id"])]
        return units

    def _delete(self, units: List[Unit]) -> None:
        for unit in units:
            self._client.delete_unit(unit.id)


def _fields(unit: Unit) -> Dict[str, Any]:
    assert isinstance(unit.payload, dict)
    return unit.payload


def _node(unit: Unit) -> BaseNode:
    fields = _fields(unit)
    return metadata_dict_to_node(json.loads(fields["node"]), text=fields["content"])


def _no_kwargs(kwargs: Dict[str, Any]) -> None:
    if kwargs:
        raise TypeError(f"unexpected arguments {sorted(kwargs)}")
