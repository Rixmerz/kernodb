// Regression tests for the Node findings of the alpha.4 gate (docs/release/alpha.4-validation.md).
import assert from 'node:assert/strict'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { afterEach, beforeEach, test } from 'node:test'

import { open, type Db } from '../index.js'

const MISSING = '00'.repeat(16)

/** The `{ code, message }` of the error `fn` throws; fails the test when it does not throw. */
function thrown(fn: () => unknown): { code?: string; message: string } {
  try {
    fn()
  } catch (e) {
    assert.ok(e instanceof Error)
    return { code: (e as Error & { code?: string }).code, message: e.message }
  }
  assert.fail('expected a throw')
}

/** A call that must be `INVALID_ARGUMENT` with a readable message (no napi or Rust wording). */
function refused(fn: () => unknown, mentions?: RegExp): string {
  const { code, message } = thrown(fn)
  assert.equal(code, 'INVALID_ARGUMENT', message)
  assert.doesNotMatch(message, /Failed to|convert|rust type|AccessMode|Vec<|_/i, message)
  if (mentions) assert.match(message, mentions)
  return message
}

let dir: string
let db: Db
// Calls with arguments the types forbid.
let loose: Record<string, (...a: unknown[]) => unknown>

beforeEach(() => {
  dir = mkdtempSync(join(tmpdir(), 'kernodb-gate-'))
  db = open(dir)
  loose = db as unknown as Record<string, (...a: unknown[]) => unknown>
  // `this` must stay the handle: bind every method.
  for (const name of Object.getOwnPropertyNames(Object.getPrototypeOf(db))) {
    if (name !== 'constructor') loose[name] = (loose[name] as (...a: unknown[]) => unknown).bind(db)
  }
})

afterEach(() => {
  db.close()
  rmSync(dir, { recursive: true, force: true })
})

test('history takes toRevision and order, as Python does', () => {
  const id = db.store('note', 'v1')
  for (const v of ['v2', 'v3', 'v4']) db.update(id, v)
  assert.deepEqual(db.history(id).map((r) => r.revision), [4, 3, 2, 1])
  assert.deepEqual(db.history(id, { order: 'asc' }).map((r) => r.revision), [1, 2, 3, 4])
  assert.deepEqual(db.history(id, { toRevision: 3 }).map((r) => r.revision), [2, 1])
  assert.deepEqual(db.history(id, { order: 'asc', toRevision: 3 }).map((r) => r.revision), [1, 2])
  // paging newest first: the last revision of a page is the next page's toRevision
  const page1 = db.history(id, { limit: 2 })
  const page2 = db.history(id, { limit: 2, toRevision: page1.at(-1)!.revision })
  assert.deepEqual([...page1, ...page2].map((r) => r.revision), [4, 3, 2, 1])
  // paging oldest first: its successor is the next fromRevision
  const up1 = db.history(id, { limit: 2, order: 'asc' })
  const up2 = db.history(id, { limit: 2, order: 'asc', fromRevision: up1.at(-1)!.revision + 1 })
  assert.deepEqual([...up1, ...up2].map((r) => r.revision), [1, 2, 3, 4])
  assert.deepEqual(db.history(id, { fromRevision: 2, toRevision: 2 }), [])
  refused(() => db.history(id, { fromRevision: 3, toRevision: 2 }))
  refused(() => db.history(id, { order: 'sideways' as 'asc' }), /order/)
  refused(() => db.history(id, { toRevision: -1 }), /toRevision/)
})

test('query takes a lifecycle and validates unitType for every one', () => {
  const live = db.store('note', 'live')
  const old = db.store('note', 'archived')
  const gone = db.store('note', 'deleted')
  const other = db.store('task', 'other type')
  db.commit([{ op: 'archive', id: old }])
  db.delete(gone)
  const ids = (units: { id: string }[]) => units.map((u) => u.id).sort()
  assert.deepEqual(ids(db.query()), [live, other].sort())
  assert.deepEqual(ids(db.query({ lifecycle: 'active' })), [live, other].sort())
  assert.deepEqual(ids(db.query({ lifecycle: 'archived' })), [old])
  assert.deepEqual(ids(db.query({ lifecycle: 'deleted' })), [gone])
  assert.equal(db.query({ lifecycle: 'deleted' })[0]!.payload, undefined)
  assert.deepEqual(ids(db.query({ lifecycle: 'all' })), [live, old, gone, other].sort())
  assert.deepEqual(ids(db.query({ lifecycle: 'all', unitType: 'note' })), [live, old, gone].sort())
  assert.deepEqual(ids(db.query({ lifecycle: 'archived', unitType: 'task' })), [])
  for (const lifecycle of ['active', 'archived', 'deleted', 'all'] as const) {
    refused(() => db.query({ lifecycle, unitType: '' }), /unitType/)
    refused(() => db.query({ lifecycle, unitType: 'x'.repeat(65) }), /unitType/)
  }
  refused(() => db.query({ lifecycle: 'gone' as 'all' }), /lifecycle/)
})

test('every options object rejects an unknown key with INVALID_ARGUMENT', () => {
  const id = db.store('note', 'x')
  const other = db.store('note', 'y')
  db.setFulltext('note', true)
  for (const call of [
    () => (open as (...a: unknown[]) => unknown)(dir + '-2', { readOnyl: true }),
    () => loose.update(id, 'z', { ifchanged: true }),
    () => loose.delete(id, { cascad: true }),
    () => loose.relate(id, other, 'r', { uniq: true }),
    () => loose.query({ lifecyle: 'all' }),
    () => loose.commit([], { resaon: 'x' }),
    () => loose.commit([{ op: 'store', unitType: 'a', payload: 'p', paylod: 'q' }]),
    () => loose.history(id, { reversed: true }),
    () => loose.runRetention({ defualt: { keepLast: 1 } }),
    () => loose.runRetention({ default: { keepLst: 1 } }),
    () => loose.runRetention({ byType: { note: { keepLst: 1 } } }),
    () => loose.searchText('x', { max: 3 }),
    () => loose.searchSubstring('x', { max: 3 }),
    () => loose.defineSchema('u', [{ name: 'a', type: 'int', requird: true }]),
    () => loose.hybridSearch({ text: 'x', limt: 3 }),
    () => loose.hybridSearch({ text: 'x', weights: { kw: 3 } }),
    () => loose.hybridSearch({ relations: { seeds: [id], depth: 3 } }),
    () => loose.hybridSearch({ vectors: { query: [1], items: [], dist: 'dot' } }),
  ]) {
    refused(call, /unknown option/)
  }
  // nothing was written by the refused calls
  assert.equal(db.history(id).length, 1)
})

test('Unit carries oldestRevision, which moves up when retention prunes', () => {
  const id = db.store('note', 'v1')
  assert.equal(db.read(id)!.oldestRevision, 1)
  for (const v of ['v2', 'v3', 'v4']) db.update(id, v)
  assert.equal(db.read(id)!.oldestRevision, 1)
  db.runRetention({ default: { keepLast: 2 } })
  const unit = db.read(id)!
  assert.equal(unit.revision, 4)
  assert.equal(unit.oldestRevision, 3)
  assert.equal(db.query({ unitType: 'note' })[0]!.oldestRevision, 3)
  // the number is the PRUNED boundary of history
  assert.equal(db.history(id, { fromRevision: unit.oldestRevision }).length, 2)
  assert.equal(thrown(() => db.history(id, { fromRevision: unit.oldestRevision - 1 })).code, 'PRUNED')
})

test('an as-of read of a compacted deleted unit is HISTORY_UNAVAILABLE, without a policy name', () => {
  const id = db.store('note', 'doomed')
  const seq = db.lastCommitSeq()
  db.update(id, 'x')
  db.delete(id)
  db.compact()
  const e = thrown(() => db.readAsOf(id, seq))
  assert.equal(e.code, 'HISTORY_UNAVAILABLE')
  assert.doesNotMatch(e.message, /policy|Audit/i)
  assert.match(e.message, /compact/)
  assert.equal(thrown(() => db.history(id, { withPayload: true })).code, 'HISTORY_UNAVAILABLE')
  // the revisions are still listed without their payloads
  assert.ok(db.history(id).length >= 2)
})

test('hybrid score is the float sum of weight / (rrfK + rank), with per-leg scores', () => {
  const a = db.store('note', 'alpha beta')
  const b = db.store('note', 'alpha')
  db.setFulltext('note', true)
  const keywordOnly = db.hybridSearch({ text: 'alpha beta' })
  assert.equal(keywordOnly[0]!.id, a)
  const first = keywordOnly[0]!
  assert.ok(Math.abs(first.score - 1 / 61) < 1e-6, `score ${first.score}`)
  assert.ok(Math.abs(first.keywordScore! - first.score) < 1e-12)
  assert.equal(first.vectorScore, undefined)
  assert.equal(first.relationScore, undefined)

  const both = db.hybridSearch({
    text: 'alpha',
    vectors: { query: [1, 0], items: [{ id: b, vector: [1, 0] }] },
    weights: { keyword: 2 },
    rrfK: 10,
  })
  const hit = both.find((h) => h.id === b)!
  assert.ok(hit.keywordScore! > 0 && hit.vectorScore! > 0)
  assert.ok(Math.abs(hit.score - (hit.keywordScore! + hit.vectorScore!)) < 1e-6)
  assert.ok(Math.abs(hit.keywordScore! - 2 / (10 + hit.ranks.keyword!)) < 1e-6)
  assert.ok(Math.abs(hit.vectorScore! - 1 / (10 + hit.ranks.vector!)) < 1e-6)
})

test('a wrong-typed argument is INVALID_ARGUMENT with a readable message', () => {
  const id = db.store('note', 'x')
  refused(() => loose.store(undefined, 'x'), /store: unitType/)
  refused(() => loose.store('note', null), /store: payload/)
  refused(() => loose.read(undefined), /read: id/)
  refused(() => loose.commit(null), /commit: ops/)
  refused(() => loose.commit('x'), /commit: ops/)
  refused(() => loose.commit([null]), /ops\[0\]/)
  refused(() => loose.commit([{ unitType: 'a', payload: 'p' }]), /\.op is required/)
  refused(() => loose.searchSubstring('x', { types: 'note' }), /types/)
  refused(() => loose.searchSubstring(undefined), /needle/)
  refused(() => loose.searchText('x', { types: 'note' }), /types/)
  refused(() => loose.searchText(undefined), /query/)
  refused(() => loose.setFulltext('note'), /enabled/)
  refused(() => loose.setFulltext(undefined, true), /unitType/)
  refused(() => loose.hybridSearch(null), /query/)
  refused(() => loose.hybridSearch({ relations: { seeds: id } }), /seeds/)
  refused(() => loose.hybridSearch({ relations: { seeds: [id], types: 'r' } }), /types/)
  refused(() => loose.hybridSearch({ relations: {} }), /seeds/)
  refused(() => loose.hybridSearch({ text: 'x', textTypes: 'note' }), /textTypes/)
  refused(() => loose.defineSchema('u', 'a'), /fields/)
  refused(() => loose.storeDocument('u', null), /document/)
  refused(() => loose.related(id, undefined), /direction/)
  refused(() => loose.update(id, 'x', 'ifChanged'), /options/)
  refused(() => (open as (...a: unknown[]) => unknown)(undefined), /path/)
})

test('an error that is not an argument failure is never INVALID_ARGUMENT', () => {
  // a Kerno error passes through with its own code; an unrelated Error from native code would be
  // INTERNAL. The first half is observable without a fault injector.
  assert.equal(thrown(() => db.read('zz')).code, 'INVALID_ARGUMENT')
  assert.equal(thrown(() => db.history(MISSING)).code, 'NOT_FOUND')
  db.close()
  assert.equal(thrown(() => db.read(MISSING)).code, 'CLOSED')
  db = open(dir)
})

test('error messages use the Node names', () => {
  db.store('note', 'x')
  assert.match(refused(() => db.searchText('x', { limit: 0 }), /limit/), /1 or more/)
  assert.match(thrown(() => db.searchText('x')).message, /setFulltext/)
  assert.doesNotMatch(thrown(() => db.searchText('x')).message, /set_fulltext/)
  assert.match(thrown(() => db.searchSubstring('x')).message, /setSubstring/)
  refused(() => db.hybridSearch({ text: 'x', textTypes: ['note'], weights: { keyword: 0 } }), /textTypes/)
  db.store('note', 'seed')
  refused(() => db.hybridSearch({ relations: { seeds: [MISSING], maxDepth: 5 } }), /maxDepth/)
  refused(() => db.hybridSearch({ relations: { seeds: [MISSING], maxDepth: 0 } }), /maxDepth/)
  const a = db.store('note', 'a')
  const message = refused(() => db.relate(a, a, 'a\0b'), /relationType/)
  assert.match(message, /NUL/)
  // a read-write handle has nothing to refresh
  const refresh = refused(() => db.refresh(), /readOnly/)
  assert.doesNotMatch(refresh, /AccessMode/)
  // the same NOT_FOUND wording everywhere
  const read = thrown(() => db.history(MISSING)).message
  const upd = thrown(() => db.updateDocument(MISSING, { a: 1 }))
  assert.equal(upd.code, 'NOT_FOUND')
  assert.match(upd.message, /^unit .* not found$/)
  assert.match(read, /not found/)
})

test('numeric messages state the real range and name the Node argument', () => {
  const id = db.store('note', 'x')
  db.setFulltext('note', true)
  db.setSubstring('note', true)
  for (const call of [
    () => db.history(id, { limit: 0 }),
    () => db.history(id, { limit: -1 }),
    () => db.history(id, { limit: 1.5 }),
    () => db.searchText('x', { limit: 0 }),
    () => db.searchSubstring('x', { limit: 0 }),
    () => db.hybridSearch({ text: 'x', limit: 0 }),
  ]) {
    const message = refused(call, /limit/)
    assert.match(message, /1 or more/)
    assert.doesNotMatch(message, /0 or more/)
  }
  for (const maxBatchOps of [0, 2 ** 20 + 1, -1, 1.5]) {
    const message = refused(() => open(dir + '-ops', { maxBatchOps }), /maxBatchOps/)
    assert.match(message, /1 to 1048576/)
  }
  const ok = open(dir + '-ops', { maxBatchOps: 2 ** 20 })
  ok.close()
  rmSync(dir + '-ops', { recursive: true, force: true })
})

test('readDocument of a payload that is not a document throws SCHEMA', () => {
  const id = db.store('note', 'plain text')
  const { code, message } = thrown(() => db.readDocument(id))
  assert.equal(code, 'SCHEMA', message)
  assert.match(message, /the payload is not a document/)
})

test('a relation type refusal says which rule failed', () => {
  const a = db.store('note', 'a')
  assert.match(refused(() => db.relate(a, a, 'a\0b'), /NUL/), /NUL/)
  const long = refused(() => db.relate(a, a, 'r'.repeat(65)), /65 bytes/)
  assert.match(long, /1\.\.=64/)
  assert.doesNotMatch(long, /NUL/)
})

test('retention messages state the range and use the Node names', () => {
  for (const rule of [{ keepLast: 0 }, { keepLast: -1 }]) {
    const message = refused(() => db.runRetention({ default: rule }), /keepLast/)
    assert.match(message, /1 or more/)
    assert.doesNotMatch(message, /KeepLast|keep_last/)
  }
  assert.match(refused(() => db.runRetention({ byType: { note: { keepSince: -1 } } }), /keepSince/), /0 or more/)
})

test('a wrong lifecycle is INVALID_ARGUMENT with lowercase lifecycle names', () => {
  const id = db.store('note', 'a')
  const message = refused(() => db.commit([{ op: 'unarchive', id }]), /active/)
  assert.doesNotMatch(message, /Active|Archived/)
})

test('a pruned readAsOf names only Node calls', () => {
  const id = db.store('note', 'v1')
  for (const v of ['v2', 'v3', 'v4']) db.update(id, v)
  db.runRetention({ default: { keepLast: 1 } })
  for (const call of [() => db.readAsOf(id, 1), () => db.history(id, { fromRevision: 1 })]) {
    const { code, message } = thrown(call)
    assert.equal(code, 'PRUNED', message)
    assert.doesNotMatch(message, /compare|_/)
  }
})

test('a failure that is not an argument failure reaches INTERNAL', () => {
  // A getter that throws an error with no Kerno or napi code while the op is read: index.js
  // passes Kerno codes through, maps napi argument errors to INVALID_ARGUMENT, and anything
  // else to INTERNAL.
  const op = {
    op: 'update',
    get id() {
      throw new RangeError('boom')
    },
    payload: 'x',
  }
  const { code, message } = thrown(() => loose.commit([op]))
  assert.equal(code, 'INTERNAL', message)
  assert.match(message, /boom/)
})
