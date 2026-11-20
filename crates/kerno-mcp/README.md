# kerno-mcp

An [MCP](https://modelcontextprotocol.io) server that lets an AI coding agent use a Kerno
database as long-term memory. It speaks JSON-RPC 2.0 over stdio, one message per line,
protocol version `2025-06-18`. It uses only the public API of `kerno-units`.

## Build

```sh
cargo build --release -p kerno-mcp
# the binary is target/release/kerno-mcp
```

## Register it in Claude Code

```sh
claude mcp add kerno -- /path/to/kerno-mcp --db /path/to/dir
```

The database directory comes from `--db <dir>` or, if the flag is absent, from the
environment variable `KERNO_MCP_DB`. It is created if it does not exist. One process at a
time can hold a directory: a second server on the same one exits with a "locked" message.

## Tools

| Tool | What it does |
|---|---|
| `remember` | Store a note: `type`, `text`, optional `tags`. Returns its id (`k1_...`). |
| `relate` | Declare a directed relation `from` -> `to` with a `relation_type`. |
| `recall` | With `from`: follow relations from a unit (`direction`, `relation_types`, `max_depth` <= 4, `limit` <= 1000, `visit_budget` <= 100000). Without it: the newest units with a `type` and/or `tag`. |
| `get` | One unit at its current revision. |
| `history` | The retained revisions of a unit, newest first. |
| `forget` | Logical delete (below). Relations that touch the unit are removed with it. |
| `verify` | Run `verify_units` and return its findings. |

Every unit is named by its public id (`k1_` plus 26 characters, ADR-0016). Ids are opaque,
stable across restarts, and the only handle the tools take.

## Errors

A malformed message, an unknown method or tool, or bad arguments is a JSON-RPC error
(`-32700`, `-32600`, `-32601`, `-32602`) with a message. A well formed call that fails (no such
unit, the engine refused it) is a tool result with `isError: true` and the reason as text.
The server never panics on client input.

## Forgetting is logical

`forget` deletes the unit logically (ADR-0009): it is unreadable at once, but its bytes stay
on disk until the engine compacts, and this server has no way to erase them on demand. Do
not rely on it to remove a secret from the files.

## Limits

- The database must be one `kerno-mcp` created: a unit without a public id has no name the
  tools accept, and a listing that reaches one fails with a message that says so.
- A `tag` filter reads every unit of the type; fine for a personal memory.
- A message is read as one whole line, with no size cap.
- Forgetting a unit with thousands of relations may exceed the engine's change set size and
  fail with its message.

## Test

```sh
cargo test -p kerno-mcp
```

The tests start the real binary and talk to it over its pipes.
