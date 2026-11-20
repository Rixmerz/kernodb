# Feedback from outside the core team

ROADMAP Phase D asks for real use outside the core team, with feedback
addressed. This page says how to give it and what happens next. If you tried
Kerno for an afternoon, that is useful: say what you tried and where it broke.

Open an issue on the repository and copy the template below into it. Fill in
what you know; a short report with the facts beats a long one without them.

## Before you paste

**Do not paste payloads you would not publish.** A unit's payload and metadata
go into issues, logs and exports. Replace real content with a stand-in of the
same size and shape. A report is public. Kerno does not encrypt anything in 1.0.

## Template

```text
### What I was doing
(one or two lines: the goal, not the steps)

### How I used Kerno
- Access path: library (Rust) / CLI / MCP server / Python / Node
- Kerno version or commit:
- OS and architecture:
- Engine: default / other (which)
- Size: roughly how many units, relations, revisions

### What I stored
(unit types, a typical payload size, relations between them, history policy)

### What I could not find, or what went wrong
- Expected:
- Actual:
- Smallest steps that show it (a snippet or CLI commands, with stand-in data):

### Output of `kerno <dir> verify`
(paste it, or say it was not run. A database that fails `verify` is a bug on its own)

### What was missing
(a query you could not write, a relation you could not express, a command you looked for)

### Anything else
(was the documentation enough to get started; where did you stop and guess)
```

## What we do with it

- A report that includes a failing `kerno verify`, a crash, or data that
  vanished is treated as a bug first, before anything else in the queue.
- A report of something you could not find or express goes into the order of
  Phases A to C of the [ROADMAP](../ROADMAP.md), which real use decides.
- Every report gets an answer on the issue, even when it is "not planned"
  (see "Not planned" in the ROADMAP).
- Feedback that changes a promise in [compatibility.md](compatibility.md) is
  raised with the project owner before 1.0, not decided in an issue.

## Contributions and the contributor license

**External code contributions are not accepted yet.** The contributor license
agreement (CLA) that ADR-0019 requires before the first external contribution
exists only as a draft, [legal/cla-draft.md](legal/cla-draft.md). Its text
**awaits the approval of the project owner** (and a lawyer's review, which the
draft itself asks for). It is not in force, and nothing in this repository
approves it. Feedback and issues need no CLA and are welcome now.

When the owner approves a text, the README and `CONTRIBUTING.md` will point to
it, and this section will change.
