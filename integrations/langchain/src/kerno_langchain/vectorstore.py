"""A LangChain vector store whose documents live in a Kerno database server."""

from __future__ import annotations

import heapq
import json
import uuid
from typing import Any, Dict, Iterable, List, Optional, Sequence, Tuple

from kerno_client import FieldDef, KernoClient, Unit
from langchain_core.documents import Document
from langchain_core.embeddings import Embeddings
from langchain_core.vectorstores import VectorStore

from ._records import cosine, ensure_schema, pack, scan, unpack, where_equals

DEFAULT_COLLECTION = "langchain_documents"

FIELDS = [
    FieldDef("doc_id", "text", required=True, unique=True),
    FieldDef("content", "text", required=True),
    FieldDef("metadata", "text", required=True),  # a JSON object
    FieldDef("embedding", "bytes", required=True),
]


class KernoVectorStore(VectorStore):
    """Documents and their embeddings, stored as typed units of one Kerno collection.

    Search is exact: it reads the collection and ranks it by cosine similarity, so the score of a
    hit is that similarity (-1 to 1). The server keeps no vector index yet, and the cost of a
    query is the size of the collection.

    ``add_documents`` and ``add_texts`` upsert: an id that is already stored replaces that
    document (a new revision of its unit, so Kerno keeps the old one in its history).
    """

    # ponytail: a query reads the whole collection and ranks it in Python (measured: 2000 vectors of
    # 1536 dimensions in 1.4 s, linear in both); fine to a few thousand items, then switch
    # to the server's vector leg or an index when kerno-server grows one.

    def __init__(self, embedding: Embeddings, client: KernoClient, collection: str = DEFAULT_COLLECTION) -> None:
        self._embedding = embedding
        self._kerno = client
        self._collection = collection
        ensure_schema(client, collection, FIELDS)

    @property
    def embeddings(self) -> Embeddings:
        return self._embedding

    # ------------------------------------------------------------------ writing

    def add_documents(self, documents: List[Document], *, ids: Optional[List[str]] = None) -> List[str]:
        """Stores the documents and returns their ids: ``ids``, else each ``Document.id``, else a new uuid4."""
        if ids is not None and len(ids) != len(documents):
            raise ValueError(f"{len(ids)} ids for {len(documents)} documents")
        wanted = ids if ids is not None else [d.id for d in documents]
        vectors = self._embedding.embed_documents([d.page_content for d in documents])
        stored = []
        for doc, doc_id, vector in zip(documents, wanted, vectors):
            doc_id = doc_id or str(uuid.uuid4())
            fields = {
                "doc_id": doc_id,
                "content": doc.page_content,
                "metadata": json.dumps(doc.metadata),
                "embedding": pack(vector),
            }
            existing = self._find(doc_id)
            if existing is None:
                self._kerno.create_unit(self._collection, fields)
            else:
                self._kerno.update_unit(existing.id, fields)
            stored.append(doc_id)
        return stored

    def delete(self, ids: Optional[List[str]] = None, **kwargs: Any) -> Optional[bool]:
        """Deletes the documents with these ids (an unknown id is skipped), or all of them if ``ids`` is None."""
        if kwargs:
            raise TypeError(f"unexpected arguments {sorted(kwargs)}")
        units = scan(self._kerno, self._collection) if ids is None else [u for i in ids if (u := self._find(i))]
        for unit in units:
            self._kerno.delete_unit(unit.id)
        return True

    # ------------------------------------------------------------------ reading

    def get_by_ids(self, ids: Sequence[str], /) -> List[Document]:
        units = [u for i in dict.fromkeys(ids) if (u := self._find(i))]
        return [_document(u) for u in units]

    def similarity_search(self, query: str, k: int = 4, *, filter: Optional[Dict[str, Any]] = None) -> List[Document]:
        return [doc for doc, _ in self.similarity_search_with_score(query, k, filter=filter)]

    def similarity_search_by_vector(
        self, embedding: List[float], k: int = 4, *, filter: Optional[Dict[str, Any]] = None
    ) -> List[Document]:
        return [doc for doc, _ in self.similarity_search_with_score_by_vector(embedding, k, filter=filter)]

    def similarity_search_with_score(
        self, query: str, k: int = 4, *, filter: Optional[Dict[str, Any]] = None
    ) -> List[Tuple[Document, float]]:
        return self.similarity_search_with_score_by_vector(self._embedding.embed_query(query), k, filter=filter)

    def similarity_search_with_score_by_vector(
        self, embedding: List[float], k: int = 4, *, filter: Optional[Dict[str, Any]] = None
    ) -> List[Tuple[Document, float]]:
        """The ``k`` documents closest to ``embedding`` with their cosine similarity, best first.

        ``filter`` keeps the documents whose metadata holds every ``key: value`` of it (equality).
        """
        if filter is not None and not isinstance(filter, dict):
            raise TypeError("filter is a dict of metadata values that must be equal")
        scored = []
        for unit in scan(self._kerno, self._collection):
            doc = _document(unit)
            if filter and any(key not in doc.metadata or doc.metadata[key] != value for key, value in filter.items()):
                continue
            scored.append((doc, cosine(embedding, unpack(unit.payload["embedding"]))))  # type: ignore[index]
        return heapq.nlargest(k, scored, key=lambda pair: pair[1])

    def _select_relevance_score_fn(self) -> Any:
        return lambda similarity: (similarity + 1) / 2

    # ------------------------------------------------------------------ building

    @classmethod
    def from_texts(  # type: ignore[override]
        cls,
        texts: List[str],
        embedding: Embeddings,
        metadatas: Optional[List[dict]] = None,
        *,
        ids: Optional[List[str]] = None,
        client: KernoClient,
        collection: str = DEFAULT_COLLECTION,
    ) -> "KernoVectorStore":
        store = cls(embedding, client, collection)
        store.add_texts(texts, metadatas, ids=ids)
        return store

    # ----------------------------------------------------------------- plumbing

    def _find(self, doc_id: str) -> Optional[Unit]:
        found = scan(self._kerno, self._collection, where_equals("doc_id", doc_id))
        return found[0] if found else None


def _document(unit: Unit) -> Document:
    fields: Any = unit.payload
    return Document(id=fields["doc_id"], page_content=fields["content"], metadata=json.loads(fields["metadata"]))
