# kerno-llamaindex

A [LlamaIndex](https://www.llamaindex.ai) vector store whose nodes live in a
[`kerno-server`](../../crates/kerno-server), built on the Python client in
[`clients/python`](../../clients/python). It stores each node (text, metadata, relationships and
embedding) as one typed unit, so a Kerno database holds the index and survives restarts. Not
published anywhere yet: use it from a checkout.

```python
from kerno_client import KernoClient
from kerno_llamaindex import KernoVectorStore
from llama_index.core import Document, StorageContext, VectorStoreIndex

store = KernoVectorStore(KernoClient("http://127.0.0.1:8080"))  # collection="llamaindex_nodes"
index = VectorStoreIndex.from_documents(
    [Document(text="Rust has no garbage collector", doc_id="rust")],
    storage_context=StorageContext.from_defaults(vector_store=store),
    embed_model=my_embed_model,
)
print(index.as_retriever(similarity_top_k=3).retrieve("memory management"))
```

## What it does

* `add`, `delete(ref_doc_id)`, `delete_nodes(node_ids, filters)`, `clear`, `get_nodes` and `query`.
* `add` **upserts**: a node whose id is stored replaces it, as a new revision of its unit (Kerno
  keeps the old revision in its history).
* `query` is **exact**: it reads the collection and ranks it by cosine similarity, and the
  `similarities` it returns are that cosine (-1 to 1). Ties keep the order the nodes were added.
* Metadata filters use LlamaIndex's own evaluator, so every operator and condition it has works,
  as does `doc_ids` and `node_ids`. An empty `node_ids` or `doc_ids` restricts nothing.
* A node reads back whole (metadata, relationships, excluded keys), except its embedding.

## What it does not do

* **No index.** The server has no vector index yet, so a query costs the size of the collection:
  a query over 2000 vectors of 1536 dimensions took 1.4 s when measured, and the time is linear in
  both, so it suits a few thousand items. The dimension of the query must be that of
  the stored nodes, else it raises `ValueError`.
* **Only the default query mode.** `HYBRID`, `TEXT_SEARCH`, `MMR` and the rest, `output_fields` and
  `embedding_field` raise `NotImplementedError`; they are never silently ignored. (The server's keyword
  index is opt-in per type and this store does not turn it on: it searches by embedding, so there is
  no text leg.)
* A collection is one unit type with one schema for good. Two stores may share a collection, but a
  collection that already has another schema raises `ValueError`: pick another `collection`.
* The server's API tokens are not sent by this store yet (use a server started with `--db`, or a
  proxy that adds the token); TLS works with an `https://` URL.

## Tests

```sh
pip install -e ../../clients/python -e '.[test]'
pytest            # from integrations/llamaindex
```

The tests start the real server: `$KERNO_SERVER_BIN` when set, else the binary
`cargo build -p kerno-server` makes. A missing binary fails the run, it does not skip it. They need
no network and no model: the embeddings are a bag of words defined in the tests.
