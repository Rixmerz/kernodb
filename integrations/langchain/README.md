# kerno-langchain

[LangChain](https://www.langchain.com) integrations for a [`kerno-server`](../../crates/kerno-server),
built on the Python client in [`clients/python`](../../clients/python): a **vector store** and a
**chat message history** (memory). Everything is stored as typed units of a Kerno database, which
survives restarts. Not published anywhere yet: use it from a checkout.

```python
from kerno_client import KernoClient
from kerno_langchain import KernoChatMessageHistory, KernoVectorStore

kerno = KernoClient("http://127.0.0.1:8080")

store = KernoVectorStore(my_embeddings, kerno)  # collection="langchain_documents"
store.add_texts(["Rust has no garbage collector"], [{"topic": "rust"}], ids=["rust-gc"])
for doc, similarity in store.similarity_search_with_score("memory management", k=3, filter={"topic": "rust"}):
    print(doc.id, doc.page_content, similarity)
retriever = store.as_retriever(search_kwargs={"k": 3})

history = KernoChatMessageHistory("session-42", kerno)  # collection="langchain_chat_messages"
history.add_user_message("hi")
history.add_ai_message("hello")
print(history.messages)  # in order; usable with RunnableWithMessageHistory
```

## Vector store

* `add_documents` / `add_texts` / `from_texts`, `get_by_ids`, `delete(ids)` (no ids: all),
  `similarity_search[_by_vector|_with_score|_with_relevance_scores]` and `as_retriever`.
* Writing **upserts**: an id that is stored replaces that document, as a new revision of its unit
  (Kerno keeps the old revision in its history). Ids are your `ids`, else `Document.id`, else a uuid4.
* Search is **exact**: it reads the collection and ranks it by cosine similarity, and the score
  of a hit is that cosine (-1 to 1; relevance scores are `(cosine + 1) / 2`). Ties keep the order the
  documents were added.
* `filter={"key": value, ...}` keeps the documents whose metadata equals all of them. Nothing else
  is accepted as a filter, and an unknown argument raises `TypeError`, so nothing is silently ignored.
* **No index.** The server has no vector index yet, so a query costs the size of the collection:
  a query over 2000 vectors of 1536 dimensions took 1.4 s when measured, and the time is linear in
  both, so it suits a few thousand items. The dimension of the query must be that
  of the stored documents, else it raises `ValueError`. There is no maximal-marginal-relevance
  search and no keyword leg (the embedding is a `bytes` field, and the server's keyword index skips a record that is not
  valid UTF-8).

## Chat message history

One unit per message, kept in the order added (unit ids are UUIDv7, which increase within a server
run and, across runs, with the server's clock). `clear()` deletes only the messages of its session.

## Both

A collection is one unit type with one schema for good. Two stores may share a collection, but a
collection that already has another schema raises `ValueError`: pick another `collection`. The
server has no authentication or TLS yet, so neither has this package.

## Tests

```sh
pip install -e ../../clients/python -e '.[test]'
pytest            # from integrations/langchain
```

The tests start the real server: `$KERNO_SERVER_BIN` when set, else the binary
`cargo build -p kerno-server` makes. A missing binary fails the run, it does not skip it. They need
no network and no model: the embeddings are a bag of words defined in the tests.
