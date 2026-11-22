// Pre-release conformance and boundary tests for the Node binding.
//
// Every claim of the README, of the type docs in index.d.ts and of the shared docs is executed here,
// each documented limit at the limit and one past it. The boundary tests feed odd values to every
// call: each must succeed as documented or throw an Error with a documented `code`, never crash,
// panic or answer silently. Table-driven, with a seeded generator for the properties: the package
// has no property-testing dependency (fast-check would need a lockfile change).
import assert from 'node:assert/strict'
import { execFileSync } from 'node:child_process'
import { copyFileSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { pathToFileURL } from 'node:url'
import { afterEach, beforeEach, test } from 'node:test'

import { open, type CommitOp, type Db } from '../index.js'

const MISSING = '00'.repeat(16)
const MIB = 1024 * 1024
const ROOT = new URL('../../../', import.meta.url)
const NODE_README = readFileSync(new URL('bindings/node/README.md', ROOT), 'utf8')

let dir: string
let db: Db

beforeEach(() => {
  dir = mkdtempSync(join(tmpdir(), 'kernodb-conf-'))
  db = open(join(dir, 'db'))
})

afterEach(() => {
  db.close()
  rmSync(dir, { recursive: true, force: true })
})

type Code = string | undefined

/** The `code` of the error `fn` throws ('ok' when it does not throw). */
function outcome(fn: () => unknown): string {
  try {
    fn()
  } catch (e) {
    assert.ok(e instanceof Error, `a thrown value that is not an Error: ${String(e)}`)
    const code = (e as Error & { code?: Code }).code
    return code ?? `no-code:${e.constructor.name}`
  }
  return 'ok'
}

/** The `code` of the error `fn` throws; fails when it does not throw. */
function codeOf(fn: () => unknown): string {
  const out = outcome(fn)
  assert.notEqual(out, 'ok', 'expected a throw')
  return out
}

/** Calls a method with arguments the types forbid. */
function loose(target: Db): Record<string, (...args: unknown[]) => unknown> {
  return target as unknown as Record<string, (...args: unknown[]) => unknown>
}

const stored = (payload: string | Buffer = 'x', type = 'note'): string => db.store(type, payload)
const ops = (n: number, payload = 'x'): CommitOp[] => Array.from({ length: n }, () => ({ op: 'store', unitType: 'n', payload }))

// ---------------------------------------------------------------- README examples

test('the README quickstart runs as written', () => {
  const block = /```js\n(import \{ open \}[\s\S]*?)```/.exec(NODE_README)?.[1]
  assert.ok(block, 'the quickstart block is in the README')
  const script = block
    .replace("'@rixmerz/kernodb'", JSON.stringify(pathToFileURL(new URL('bindings/node/index.js', ROOT).pathname).href))
    .replace("'./memory-db'", JSON.stringify(join(dir, 'memory-db')))
  const file = join(dir, 'memory.mjs')
  writeFileSync(file, script)
  const out = execFileSync(process.execPath, [file], { encoding: 'utf8' })
  assert.equal(out.split('\n')[0], 'note Rust has no GC')
  assert.equal(out.match(/relationType: 'about'/g)?.length, 2, 'the edge, seen from both ends')
  assert.equal(out.trimEnd().split('\n').at(-1), '1')
})

test('the commit example returns the documented report', () => {
  const { units, commitSeq, relations, unchanged, upserts } = db.commit(
    [
      { op: 'store', unitType: 'note', payload: 'first' },
      { op: 'store', unitType: 'note', payload: 'second' },
      { op: 'relate', from: 0, to: 1, relationType: 'next' },
    ],
    { reason: 'import', actor: 'importer' },
  )
  assert.equal(units.length, 2)
  assert.equal(relations.length, 1)
  assert.equal(unchanged, 0)
  assert.deepEqual(upserts, [])
  assert.equal(commitSeq, db.lastCommitSeq())
  assert.deepEqual(db.related(units[0]!, 'out').map((r) => r.to), [units[1]])
  assert.equal(db.commit([]).commitSeq, undefined, 'nothing written: no sequence')
})

test('every documented op works in one commit', () => {
  db.defineSchema('user', [{ name: 'email', type: 'text', unique: true }])
  const [a, b, c, d, e, f, g] = ['a', 'b', 'c', 'd', 'e', 'f', 'g'].map((p) => stored(p))
  db.relate(a!, b!, 'r')
  const rel = db.relate(a!, e!, 'r')
  db.commit([{ op: 'archive', id: g! }])
  const out = db.commit([
    { op: 'store', unitType: 'note', payload: 's' },
    { op: 'update', id: a!, payload: 'a2' },
    { op: 'update', id: e!, payload: 'e', ifChanged: true },
    { op: 'delete', id: d! },
    { op: 'delete_cascade', id: b! },
    { op: 'unrelate', relationId: rel },
    { op: 'relate', from: a!, to: e!, relationType: 't' },
    { op: 'relate', from: a!, to: e!, relationType: 't', unique: true },
    { op: 'archive', id: f! },
    { op: 'unarchive', id: g! },
    { op: 'restore', id: c!, revision: 1 },
    { op: 'upsert', unitType: 'user', field: 'email', document: { email: 'x@y' } },
  ])
  assert.equal(out.units.length, 1)
  assert.equal(out.upserts.length, 1)
  assert.equal(out.upserts[0]?.outcome, 'created')
  assert.equal(out.unchanged, 1)
  assert.equal(db.read(d!)?.lifecycle, 'deleted')
  assert.equal(db.read(f!)?.lifecycle, 'archived')
  assert.equal(db.read(g!)?.lifecycle, 'active')
  assert.equal(db.related(a!, 'out', 't').length, 1, 'a unique relate reuses the relation made earlier in the commit')
})

test('a unit is written by one op of a commit; a refused commit writes nothing', () => {
  const id = stored('x')
  const seq = db.lastCommitSeq()
  assert.equal(
    codeOf(() => db.commit([{ op: 'update', id, payload: 'y' }, { op: 'update', id, payload: 'z' }])),
    'INVALID_ARGUMENT',
  )
  assert.equal(db.lastCommitSeq(), seq)
  assert.equal(db.read(id)?.payload?.toString(), 'x')
})

test('the upsert example: created, updated, unchanged, history newest first', () => {
  db.defineSchema('user', [{ name: 'email', type: 'text', unique: true }, { name: 'name', type: 'text' }])
  const { id, outcome: first } = db.upsert('user', 'email', { email: 'a@example.com', name: 'Ada' })
  assert.equal(first, 'created')
  assert.equal(db.upsert('user', 'email', { email: 'a@example.com', name: 'Ada' }).outcome, 'unchanged')
  assert.equal(db.upsert('user', 'email', { email: 'a@example.com', name: 'Ada L.' }).outcome, 'updated')
  assert.deepEqual(db.history(id).map((r) => r.kind), ['update', 'create'])
})

test('upsert in a commit: one key is one unit, the last document wins; errors; only active units hold a key', () => {
  db.defineSchema('user', [{ name: 'email', type: 'text', unique: true }, { name: 'name', type: 'text' }])
  const out = db.commit([
    { op: 'upsert', unitType: 'user', field: 'email', document: { email: 'k', name: 'first' } },
    { op: 'upsert', unitType: 'user', field: 'email', document: { email: 'k', name: 'last' } },
  ])
  assert.deepEqual(out.units, [])
  assert.equal(new Set(out.upserts.map((u) => u.id)).size, 1)
  assert.equal(out.upserts[0]?.outcome, 'created')
  assert.deepEqual(db.readDocument(out.upserts[0]!.id), { email: 'k', name: 'last' })
  assert.equal(codeOf(() => db.upsert('user', 'name', { name: 'x' })), 'NOT_UNIQUE')
  assert.equal(codeOf(() => db.upsert('untyped', 'email', { email: 'x' })), 'NOT_UNIQUE')
  assert.equal(codeOf(() => db.upsert('user', 'email', { name: 'no key' })), 'SCHEMA')
  const first = db.upsert('user', 'email', { email: 'j' }).id
  db.delete(first)
  assert.notEqual(db.upsert('user', 'email', { email: 'j' }).id, first, 'a deleted unit holds no key')
  const second = db.upsert('user', 'email', { email: 'm' }).id
  db.commit([{ op: 'archive', id: second }])
  assert.notEqual(db.upsert('user', 'email', { email: 'm' }).id, second, 'an archived unit holds no key')
})

test('relate ends: a unit id, or the position of an earlier store or upsert op', () => {
  db.defineSchema('file', [{ name: 'path', type: 'text', unique: true }])
  const existing = stored('e')
  const out = db.commit([
    { op: 'store', unitType: 'free', payload: 'A' },
    { op: 'upsert', unitType: 'file', field: 'path', document: { path: 'g' } },
    { op: 'relate', from: 0, to: 1, relationType: 'meant_for' },
    { op: 'relate', from: existing, to: 0, relationType: 'meant_for' },
  ])
  assert.equal(out.relations.length, 2)
  assert.equal(db.related(out.units[0]!, 'out')[0]?.to, out.upserts[0]?.id)
  const again = db.commit([
    { op: 'upsert', unitType: 'file', field: 'path', document: { path: 'g' } },
    { op: 'store', unitType: 'free', payload: 'C' },
    { op: 'relate', from: 1, to: 0, relationType: 'of' },
  ])
  assert.equal(again.upserts[0]?.outcome, 'unchanged')
  assert.equal(db.related(again.units[0]!, 'out')[0]?.to, again.upserts[0]?.id, 'an unchanged upsert is a valid end')
})

test('a wrong relate index is BAD_ENDPOINT, a malformed one INVALID_ARGUMENT, and nothing is written', () => {
  const unit = stored('kept')
  const seq = db.lastCommitSeq()
  const store: CommitOp = { op: 'store', unitType: 'free', payload: 'x' }
  const relate = (from: number | string, to: number | string): CommitOp => ({ op: 'relate', from, to, relationType: 'r' })
  for (const list of [
    [store, relate(0, 2), store],
    [store, relate(0, 1)],
    [{ op: 'update', id: unit, payload: 'y' } as CommitOp, store, relate(0, 1)],
    [store, relate(0, 0), relate(1, 0)],
    [store, { op: 'delete', id: unit } as CommitOp, relate(0, 1)],
    [store, relate(0, 99)],
    [store, relate(0, 2 ** 53)],
  ]) {
    assert.equal(codeOf(() => db.commit(list)), 'BAD_ENDPOINT', JSON.stringify(list.map((o) => o.op)))
    assert.equal(db.lastCommitSeq(), seq)
  }
  for (const end of [-1, 1.5, Number.NaN, Number.POSITIVE_INFINITY, 2 ** 64]) {
    assert.equal(codeOf(() => db.commit([store, relate(0, end)])), 'INVALID_ARGUMENT', String(end))
  }
  assert.equal(codeOf(() => db.commit([store, { op: 'relate', from: 0, relationType: 'r' }])), 'INVALID_ARGUMENT')
  assert.equal(db.query({ unitType: 'free' }).length, 0)
})

test('history pages, readAsOf and PRUNED through both history and readAsOf', () => {
  const id = stored('v1')
  const seqs = [db.lastCommitSeq()]
  for (const p of ['v2', 'v3', 'v4', 'v5']) {
    db.update(id, p)
    seqs.push(db.lastCommitSeq())
  }
  assert.deepEqual(db.history(id).map((r) => r.revision), [5, 4, 3, 2, 1])
  assert.deepEqual(db.history(id, { limit: 2 }).map((r) => r.revision), [5, 4])
  assert.deepEqual(db.history(id, { fromRevision: 3 }).map((r) => r.revision), [5, 4, 3])
  assert.ok(db.history(id, { withPayload: true }).every((r) => r.payload !== undefined))
  assert.ok(db.history(id).every((r) => r.payload === undefined))
  assert.equal(db.readAsOf(id, seqs[1]!)?.payload?.toString(), 'v2')
  assert.equal(db.readAsOf(id, 0), null, 'null before creation')
  assert.equal(db.readAsOf(id, 2 ** 53)?.payload?.toString(), 'v5', 'a sequence past the last commit reads now')
  assert.equal(codeOf(() => db.history(MISSING)), 'NOT_FOUND')
  const report = db.runRetention({ byType: { note: { keepLast: 2 } } })
  assert.deepEqual(Object.keys(report).sort(), ['bytesLogicallyFreed', 'commitSeq', 'payloadsDropped', 'revisionsPruned', 'unitsRetained'])
  assert.equal(report.revisionsPruned, 3)
  assert.equal(codeOf(() => db.history(id, { fromRevision: 1 })), 'PRUNED')
  assert.equal(codeOf(() => db.readAsOf(id, seqs[0]!)), 'PRUNED')
  assert.deepEqual(db.history(id).map((r) => r.revision), [5, 4])
  assert.equal(db.runRetention({ byType: { note: { keepLast: 2 } } }).revisionsPruned, 0, 'idempotent')
  assert.equal(db.runRetention().revisionsPruned, 0)
  assert.equal(db.runRetention({ default: null, byType: null }).revisionsPruned, 0)
  assert.equal(db.runRetention({ default: { keepFor: 3600 } }).revisionsPruned, 0)
})

test('a bad retention rule is INVALID_ARGUMENT and writes nothing', () => {
  stored('x')
  const seq = db.lastCommitSeq()
  for (const rule of [{ keepLast: 0 }, { keepLast: -1 }, { keepLast: 1.5 }, { keepFor: -1 }, { keepFor: Number.NaN }, {}, { keepLast: 1, keepFor: 1 }]) {
    assert.equal(codeOf(() => db.runRetention({ default: rule })), 'INVALID_ARGUMENT', JSON.stringify(rule))
  }
  assert.equal(db.lastCommitSeq(), seq)
})

test('compact reports before at call time', () => {
  const ids = Array.from({ length: 20 }, () => stored('x'.repeat(10_000)))
  for (const id of ids) db.delete(id)
  const { before, after } = db.compact()
  assert.ok(before !== undefined && before > 0 && after !== undefined && after <= before)
})

test('verify reports a clean store', () => {
  stored('x')
  const report = db.verify()
  assert.equal(report.clean, true)
  assert.deepEqual(report.findings, [])
  assert.equal(report.findingsOmitted, 0)
  assert.equal(report.storageError, undefined)
  assert.equal(report.units, 1)
  assert.equal(report.commitSeq, db.lastCommitSeq())
})

test('searchText raises instead of answering empty; hybridSearch combines the three legs', () => {
  assert.equal(codeOf(() => db.searchText('x')), 'NOT_INDEXED')
  assert.equal(codeOf(() => db.hybridSearch({ text: 'x' })), 'NOT_INDEXED')
  db.setFulltext('note', true)
  assert.equal(db.fulltext('note'), true)
  assert.equal(db.fulltext('other'), false)
  const gc = stored('Rust has no garbage collector')
  const other = stored('Python has a garbage collector')
  const topic = stored('memory', 'topic')
  db.relate(gc, topic, 'about')
  assert.equal(db.searchText('garbage collector').length, 2)
  assert.equal(db.searchText('nothingmatches').length, 0, 'can match, finds nothing: []')
  assert.equal(db.searchText('garbage', { types: ['note'] }).length, 2)
  assert.equal(db.searchText('garbage', { limit: 1 }).length, 1)
  assert.equal(codeOf(() => db.searchText('garbage', { types: ['topic'] })), 'NOT_INDEXED')
  assert.equal(codeOf(() => db.searchText('garbage', { types: ['never-used'] })), 'UNKNOWN_TYPE')
  for (const bad of [{ limit: 0 }, { limit: -1 }, { types: [] }]) {
    assert.equal(codeOf(() => db.searchText('garbage', bad)), 'INVALID_ARGUMENT', JSON.stringify(bad))
  }
  for (const q of ['', '   ', '!!! ???']) assert.equal(codeOf(() => db.searchText(q)), 'INVALID_ARGUMENT', JSON.stringify(q))
  const hits = db.hybridSearch({
    text: 'garbage collector',
    vectors: { query: [1, 0], items: [{ id: gc, vector: [1, 0] }, { id: other, vector: [0, 1] }] },
    relations: { seeds: [topic], types: ['about'], maxDepth: 1 },
    weights: { keyword: 2 },
    limit: 5,
  })
  assert.equal(hits[0]?.id, gc)
  assert.deepEqual(Object.keys(hits[0]!.ranks).sort(), ['keyword', 'relations', 'vector'])
  assert.equal(hits.find((h) => h.id === other)?.ranks.relations, undefined)
  assert.ok(!hits.some((h) => h.id === topic), 'seeds are not hits')
  assert.equal(codeOf(() => db.hybridSearch({ textTypes: ['note'] })), 'INVALID_ARGUMENT', 'textTypes without text')
  assert.equal(codeOf(() => db.hybridSearch({})), 'INVALID_ARGUMENT', 'at least one leg must run')
  assert.equal(codeOf(() => db.hybridSearch({ text: 'garbage', weights: { keyword: 0 } })), 'INVALID_ARGUMENT', 'a 0 weight switches the leg off')
  const fused = db.hybridSearch({ text: 'garbage', relations: { seeds: [topic] }, fusion: 'graph_gated' })
  assert.equal(fused[0]?.id, gc, 'graph_gated ranks the units the relations leg found first')
  for (const metric of ['cosine', 'dot'] as const) {
    assert.equal(db.hybridSearch({ vectors: { query: [1, 0], items: [{ id: gc, vector: [1, 0] }], metric } })[0]?.id, gc)
  }
})

test('hybridSearch ranges: limit and candidates 1..1000, weights 0..1000, maxDepth 1..4, visitBudget up to 100000', () => {
  db.setFulltext('note', true)
  const seed = stored('hello world')
  const ok = (q: Parameters<Db['hybridSearch']>[0]): void => void db.hybridSearch(q)
  for (const limit of [1, 1000]) ok({ text: 'hello', limit })
  for (const candidates of [1, 1000]) ok({ text: 'hello', candidates })
  for (const keyword of [1, 1000]) ok({ text: 'hello', weights: { keyword } })
  for (const maxDepth of [1, 4]) ok({ relations: { seeds: [seed], maxDepth } })
  ok({ relations: { seeds: [seed], visitBudget: 100_000 } })
  for (const bad of [{ limit: 0 }, { limit: 1001 }, { candidates: 0 }, { candidates: 1001 }, { weights: { keyword: 1001 } }]) {
    assert.equal(codeOf(() => db.hybridSearch({ text: 'hello', ...bad })), 'INVALID_ARGUMENT', JSON.stringify(bad))
  }
  for (const bad of [{ maxDepth: 0 }, { maxDepth: 5 }, { visitBudget: 100_001 }]) {
    assert.equal(codeOf(() => db.hybridSearch({ relations: { seeds: [seed], ...bad } })), 'INVALID_ARGUMENT', JSON.stringify(bad))
  }
  assert.equal(codeOf(() => db.hybridSearch({ vectors: { query: [1, 0], items: [{ id: seed, vector: [1] }] } })), 'INVALID_ARGUMENT', 'one dimension')
})

test('schemas: idempotent define, SCHEMA_EXISTS, numbers, bigint, indexes', () => {
  const fields = [{ name: 'a', type: 'text' as const }, { name: 'b', type: 'int' as const }]
  db.defineSchema('t', fields)
  const seq = db.lastCommitSeq()
  db.defineSchema('t', [...fields].reverse())
  assert.equal(db.lastCommitSeq(), seq, 'the same schema in any order writes nothing')
  assert.equal(codeOf(() => db.defineSchema('t', [{ name: 'a', type: 'int' }])), 'SCHEMA_EXISTS')
  db.createIndex('t', 'a')
  assert.equal(codeOf(() => db.defineSchema('t', fields)), 'SCHEMA_EXISTS', 'indexing later changed the schema')
  assert.equal(codeOf(() => db.find('t', 'b', 1)), 'NOT_INDEXED')
  assert.equal(codeOf(() => db.createIndex('t', 'nope')), 'INVALID_ARGUMENT')
  db.defineSchema('m', [{ name: 'i', type: 'int', indexed: true }, { name: 'f', type: 'float' }, { name: 'y', type: 'bytes' }])
  const id = db.storeDocument('m', { i: 5, f: 5, y: Buffer.from([0, 255]) })
  assert.deepEqual(db.readDocument(id), { i: 5, f: 5, y: Buffer.from([0, 255]) })
  assert.equal(codeOf(() => db.storeDocument('m', { i: 1.5 })), 'INVALID_ARGUMENT', 'an int field refuses a fraction')
  assert.equal(db.readDocument(db.storeDocument('m', { f: 1.5 }))?.f, 1.5)
  assert.equal(db.readDocument(db.storeDocument('m', { i: 2n ** 60n }))?.i, 2n ** 60n, 'a bigint beyond 2^53 comes back as a bigint')
  assert.equal(db.readDocument(db.storeDocument('m', { i: 2 ** 53 }))?.i, 2 ** 53)
  assert.deepEqual(db.find('m', 'i', 5).map((u) => u.id), [id])
  assert.equal(db.readDocument(MISSING), null)
  const gone = db.storeDocument('m', { i: 9 })
  db.delete(gone)
  assert.equal(db.readDocument(gone), null, 'null for a deleted unit')
  assert.equal(codeOf(() => db.storeDocument('m', { i: 2n ** 63n })), 'INVALID_ARGUMENT')
  assert.equal(codeOf(() => db.storeDocument('m', { i: -(2n ** 63n) - 1n })), 'INVALID_ARGUMENT')
  assert.equal(db.readDocument(db.storeDocument('m', { i: 2n ** 63n - 1n }))?.i, 2n ** 63n - 1n)
  assert.equal(db.readDocument(db.storeDocument('m', { i: -(2n ** 63n) }))?.i, -(2n ** 63n))
  const dirty = { i: null } as unknown as Record<string, number>
  assert.equal(codeOf(() => db.storeDocument('m', dirty)), 'INVALID_ARGUMENT', 'null is a value no document can hold')
})

test('execute: aliases and rows, and the query language refusals', () => {
  db.defineSchema('user', [{ name: 'email', type: 'text', unique: true }, { name: 'age', type: 'int', indexed: true }])
  db.defineSchema('team', [{ name: 'name', type: 'text' }])
  const a = db.storeDocument('user', { email: 'a@x', age: 31 })
  const b = db.storeDocument('user', { email: 'b@x', age: 20 })
  const t = db.storeDocument('team', { name: 'core' })
  db.relate(a, t, 'member_of')
  db.relate(a, t, 'member_of')
  const out = db.execute('FIND user AS u JOIN team AS t VIA member_of WHERE u.age >= 30 ORDER BY u.age DESC LIMIT 10')
  assert.deepEqual(out.aliases, ['u', 't'])
  assert.deepEqual(out.rows.map((r) => r.map((u) => u.id)), [[a, t]], 'parallel relations give one row')
  assert.deepEqual(db.execute('FIND user AS u ORDER BY u.age LIMIT 1 OFFSET 1').rows.map((r) => r[0]?.id), [a])
  assert.deepEqual(db.execute('find user as u where u.age = 20').rows.map((r) => r[0]?.id), [b], 'keywords are case-insensitive')
  for (const q of ['', 'FIND', 'FIND user AS u WHERE u.age = 5.0', 'FIND user AS u WHERE u.nope = 1', 'FIND ghost AS g WHERE g.x = 1', 'FIND user AS u LIMIT -1', '\0']) {
    assert.equal(codeOf(() => db.execute(q)), 'INVALID_ARGUMENT', JSON.stringify(q))
  }
  const cond = 'u.age = 20'
  assert.equal(db.execute(`FIND user AS u WHERE ${Array(256).fill(cond).join(' OR ')}`).rows.length, 1, '256 conditions')
  assert.equal(codeOf(() => db.execute(`FIND user AS u WHERE ${Array(257).fill(cond).join(' OR ')}`)), 'INVALID_ARGUMENT')
  assert.equal(db.execute(`FIND user AS u WHERE ${'('.repeat(63)}${cond}${')'.repeat(63)}`).rows.length, 1, 'nesting 64 deep')
  assert.equal(codeOf(() => db.execute(`FIND user AS u WHERE ${'('.repeat(65)}${cond}${')'.repeat(65)}`)), 'INVALID_ARGUMENT')
  const head = 'FIND user AS u WHERE u.email = "'
  const exact = `${head}${'z'.repeat(16 * 1024 - head.length - 1)}"`
  assert.equal(Buffer.byteLength(exact), 16 * 1024)
  assert.equal(db.execute(exact).rows.length, 0, 'a query text of exactly 16 KiB')
  assert.equal(codeOf(() => db.execute(`${exact} `)), 'INVALID_ARGUMENT')
})

test('the units a read, query and find return are as index.d.ts and the coverage table say', () => {
  db.defineSchema('p', [{ name: 'k', type: 'int', indexed: true }])
  const live = db.storeDocument('p', { k: 1 })
  const shelved = db.storeDocument('p', { k: 2 })
  db.commit([{ op: 'archive', id: shelved }])
  const gone = stored('gone', 'q')
  db.delete(gone)
  assert.equal(db.read(shelved)?.lifecycle, 'archived')
  assert.equal(db.read(gone)?.lifecycle, 'deleted')
  assert.equal(db.read(gone)?.payload, undefined, 'a deleted unit has no payload')
  // docs/bindings-coverage.md: "Node returns active units only, like the facade"
  assert.deepEqual(db.query({ unitType: 'p' }).map((u) => u.id), [live])
  assert.deepEqual(db.find('p', 'k', 2), [])
  assert.deepEqual(db.find('p', 'k', 1).map((u) => u.id), [live])
})

// ---------------------------------------------------------------- documented limits, at the limit and one past it

test('payload limit is 1 MiB', () => {
  const id = db.store('t', Buffer.alloc(MIB))
  assert.equal(db.read(id)?.payload?.length, MIB)
  for (const over of [Buffer.alloc(MIB + 1), 'x'.repeat(MIB + 1)]) assert.equal(codeOf(() => db.store('t', over)), 'PAYLOAD_TOO_LARGE')
  assert.equal(codeOf(() => db.update(id, Buffer.alloc(MIB + 1))), 'PAYLOAD_TOO_LARGE')
  assert.equal(codeOf(() => db.commit([{ op: 'store', unitType: 't', payload: Buffer.alloc(MIB + 1) }])), 'PAYLOAD_TOO_LARGE')
  assert.equal(db.update(id, Buffer.alloc(MIB, 1)), true)
})

test('unit type is 1 to 64 bytes of UTF-8', () => {
  const makers = [(n: number) => 'a'.repeat(n), (n: number) => 'é'.repeat(n >> 1) + 'a'.repeat(n & 1), (n: number) => '😀'.repeat(n >> 2) + 'a'.repeat(n & 3)]
  for (const make of makers) {
    const t64 = make(64)
    assert.equal(Buffer.byteLength(t64), 64)
    assert.equal(db.read(db.store(t64, 'p'))?.unitType, t64)
    db.store(make(1), 'p')
    for (const n of [0, 65]) {
      assert.equal(codeOf(() => db.store(make(n), 'p')), 'INVALID_ARGUMENT', `${n} bytes`)
      assert.equal(codeOf(() => db.commit([{ op: 'store', unitType: make(n), payload: 'p' }])), 'INVALID_ARGUMENT')
    }
    assert.equal(db.query({ unitType: t64 }).length, 1)
  }
})

test('relation type is 1 to 64 bytes with no NUL', () => {
  const [a, b] = [stored('a'), stored('b')]
  db.relate(a!, b!, 'r'.repeat(64))
  db.relate(a!, b!, 'r')
  for (const bad of ['', 'r'.repeat(65), 'a\0b', '\0', 'é'.repeat(33)]) {
    assert.equal(codeOf(() => db.relate(a!, b!, bad)), 'INVALID_ARGUMENT', JSON.stringify(bad))
    assert.equal(codeOf(() => db.commit([{ op: 'relate', from: a!, to: b!, relationType: bad }])), 'INVALID_ARGUMENT')
  }
  assert.deepEqual(db.related(a!, 'out', 'r'.repeat(64)).map((r) => r.relationType), ['r'.repeat(64)])
})

test('commit metadata: reason 1024 bytes, actor and source 256 bytes', () => {
  const one: CommitOp[] = [{ op: 'store', unitType: 'n', payload: 'x' }]
  db.commit(one, { reason: 'r'.repeat(1024), actor: 'a'.repeat(256), source: Buffer.alloc(256, 0xff) })
  assert.equal(codeOf(() => db.commit(one, { reason: 'r'.repeat(1025) })), 'INVALID_ARGUMENT')
  assert.equal(codeOf(() => db.commit(one, { actor: 'a'.repeat(257) })), 'INVALID_ARGUMENT')
  assert.equal(codeOf(() => db.commit(one, { source: Buffer.alloc(257) })), 'INVALID_ARGUMENT')
  assert.equal(codeOf(() => db.commit(one, { reason: '' })), 'INVALID_ARGUMENT')
})

for (const kind of ['text', 'bytes'] as const) {
  const cases: Array<[string, string, string]> = [
    ['254 a', 'a'.repeat(254), 'a'.repeat(255)],
    ['127 é', 'é'.repeat(127), 'é'.repeat(127) + 'a'],
    ['127 NUL counts twice', '\0'.repeat(127), '\0'.repeat(128)],
    ['126 NUL + 2', '\0'.repeat(126) + 'ab', '\0'.repeat(127) + 'a'],
  ]
  for (const [label, good, bad] of cases) {
    test(`an indexed ${kind} value holds 254 bytes, a NUL counts twice (${label})`, () => {
      const conv = (s: string): string | Buffer => (kind === 'bytes' ? Buffer.from(s) : s)
      db.defineSchema('t', [{ name: 'k', type: kind, indexed: true }])
      const id = db.storeDocument('t', { k: conv(good) })
      assert.deepEqual(db.find('t', 'k', conv(good)).map((u) => u.id), [id])
      assert.equal(codeOf(() => db.storeDocument('t', { k: conv(bad) })), 'SCHEMA')
      assert.equal(codeOf(() => db.updateDocument(id, { k: conv(bad) })), 'SCHEMA')
      assert.equal(codeOf(() => db.find('t', 'k', conv(bad))), 'INVALID_ARGUMENT', 'no unit can hold it: not an empty list')
    })
  }
}

test('a commit holds 10,000 ops by default, maxBatchOps changes it', () => {
  assert.equal(db.commit(ops(10_000)).units.length, 10_000)
  const seq = db.lastCommitSeq()
  assert.equal(codeOf(() => db.commit(ops(10_001))), 'CHANGESET_TOO_LARGE')
  assert.equal(db.lastCommitSeq(), seq)
  assert.equal(db.query({ unitType: 'n' }).length, 10_000, 'a refused commit writes nothing')
  const small = open(join(dir, 'small'), { maxBatchOps: 3 })
  assert.equal(small.commit(ops(3)).units.length, 3)
  assert.equal(codeOf(() => small.commit(ops(4))), 'CHANGESET_TOO_LARGE')
  small.close()
  open(join(dir, 'big'), { maxBatchOps: 2 ** 20 }).close()
  for (const bad of [0, 2 ** 20 + 1, -1, 1.5, Number.NaN, Number.POSITIVE_INFINITY, 2 ** 64]) {
    assert.equal(codeOf(() => open(join(dir, 'bad'), { maxBatchOps: bad })), 'INVALID_ARGUMENT', String(bad))
  }
})

test('a commit over the engine byte limit is refused whole', () => {
  const seq = db.lastCommitSeq()
  const big = Buffer.alloc(MIB - 1, 1)
  assert.equal(codeOf(() => db.commit(Array.from({ length: 300 }, () => ({ op: 'store', unitType: 'n', payload: big }) as CommitOp))), 'CHANGESET_TOO_LARGE')
  assert.equal(db.lastCommitSeq(), seq)
})

test('counters are whole numbers, exact up to 2^53', () => {
  const id = stored()
  assert.equal(db.history(id, { limit: 2 ** 32 - 1 }).length, 1, 'a page size is at most 2^32 - 1')
  assert.equal(codeOf(() => db.history(id, { limit: 2 ** 32 })), 'INVALID_ARGUMENT')
  assert.ok(db.readAsOf(id, 2 ** 53))
  assert.equal(db.history(id, { fromRevision: 2 ** 53 }).length, 0)
  for (const bad of [-1, 2 ** 53 + 2, 2 ** 64, 1.5, Number.NaN, Number.POSITIVE_INFINITY, Number.NEGATIVE_INFINITY]) {
    assert.equal(codeOf(() => db.readAsOf(id, bad)), 'INVALID_ARGUMENT', `readAsOf ${bad}`)
    assert.equal(codeOf(() => db.history(id, { limit: bad })), 'INVALID_ARGUMENT', `limit ${bad}`)
    assert.equal(codeOf(() => db.history(id, { fromRevision: bad })), 'INVALID_ARGUMENT', `fromRevision ${bad}`)
  }
  assert.equal(codeOf(() => db.history(id, { limit: 0 })), 'INVALID_ARGUMENT')
})

// ---------------------------------------------------------------- errors, closed and read-only handles

test('every code of the README error table is thrown by its documented trigger', () => {
  const a = stored('a')
  const b = stored('b')
  db.relate(a, b, 'r')
  const seen = new Map<string, string>()
  const note = (expected: string, fn: () => unknown): void => {
    assert.equal(codeOf(fn), expected)
    seen.set(expected, 'seen')
  }
  note('NOT_FOUND', () => db.unrelate(MISSING))
  note('LOCKED', () => open(join(dir, 'db')))
  note('HAS_RELATIONS', () => db.delete(a))
  note('INVALID_ARGUMENT', () => db.read('nope'))
  note('PAYLOAD_TOO_LARGE', () => db.store('t', Buffer.alloc(MIB + 1)))
  note('CHANGESET_TOO_LARGE', () => db.commit(ops(10_001)))
  note('DATABASE_NOT_FOUND', () => open(join(dir, 'nothing'), { readOnly: true }))
  note('NOT_INDEXED', () => db.find('ghost', 'f', 1))
  note('NOT_UNIQUE', () => db.upsert('ghost', 'f', { f: 1 }))
  note('BAD_ENDPOINT', () => db.commit([{ op: 'relate', from: 0, to: 1, relationType: 'r' }]))
  note('SCHEMA_EXISTS', () => {
    db.defineSchema('s', [{ name: 'a', type: 'text' }])
    db.defineSchema('s', [{ name: 'a', type: 'int' }])
  })
  note('SCHEMA', () => db.storeDocument('s', { a: 1 }))
  note('UNKNOWN_TYPE', () => {
    db.setFulltext('note', true)
    db.searchText('a', { types: ['never-used'] })
  })
  const deleted = stored('d')
  db.delete(deleted)
  note('DELETED', () => db.update(deleted, 'x'))
  const pruned = stored('p1')
  db.update(pruned, 'p2')
  db.update(pruned, 'p3')
  db.runRetention({ default: { keepLast: 1 } })
  note('PRUNED', () => db.history(pruned, { fromRevision: 1 }))
  db.close()
  note('CLOSED', () => db.read(a))
  const ro = open(join(dir, 'db'), { readOnly: true })
  note('READ_ONLY', () => ro.store('n', 'x'))
  ro.close()
  assert.ok(!/(?:^|\n)\| `(?:[A-Z_]+)` \|[^\n]*\n/.test('') && seen.size >= 15)
  // every code the README table lists is one of the codes thrown above, or CORRUPTION / KERNO (see the corruption test)
  const listed = [...NODE_README.matchAll(/^\| `([A-Z_]+)` \| `\w+` \|/gm)].map((m) => m[1]!)
  assert.ok(listed.length >= 19)
  for (const code of listed) assert.ok(seen.has(code) || ['CORRUPTION', 'KERNO'].includes(code), `${code} has no trigger in this test`)
  db = open(join(dir, 'db'))
})

test('after close every call throws CLOSED and close is idempotent', () => {
  const id = stored()
  db.close()
  const calls: Array<() => unknown> = [
    () => db.store('n', 'x'), () => db.read(id), () => db.update(id, 'x'), () => db.delete(id), () => db.relate(id, id, 'r'),
    () => db.unrelate(id), () => db.related(id, 'out'), () => db.relations(), () => db.query(), () => db.commit([]),
    () => db.commit(ops(1)), () => db.history(id), () => db.readAsOf(id, 1), () => db.lastCommitSeq(), () => db.refresh(),
    () => db.verify(), () => db.compact(), () => db.runRetention(), () => db.setFulltext('n', true), () => db.fulltext('n'),
    () => db.searchText('x'), () => db.defineSchema('s', [{ name: 'a', type: 'text' }]), () => db.createIndex('s', 'a'),
    () => db.storeDocument('s', { a: 1 }), () => db.updateDocument(id, { a: 1 }), () => db.readDocument(id),
    () => db.upsert('s', 'a', { a: 1 }), () => db.find('s', 'a', 1), () => db.execute('FIND s AS x'), () => db.hybridSearch({ text: 'x' }),
  ]
  for (const call of calls) assert.equal(codeOf(call), 'CLOSED')
  db.close()
  db = open(join(dir, 'db')) // the directory was released
})

const WRITES: Record<string, (d: Db, id: string) => unknown> = {
  store: (d) => d.store('n', 'x'),
  update: (d, id) => d.update(id, 'x'),
  delete: (d, id) => d.delete(id),
  relate: (d, id) => d.relate(id, id, 'r'),
  unrelate: (d) => d.unrelate(MISSING),
  commit: (d) => d.commit(ops(1)),
  setFulltext: (d) => d.setFulltext('n', true),
  defineSchema: (d) => d.defineSchema('s', [{ name: 'a', type: 'text' }]),
  createIndex: (d) => d.createIndex('s', 'a'),
  storeDocument: (d) => d.storeDocument('s', { a: 'x' }),
  updateDocument: (d, id) => d.updateDocument(id, { a: 'x' }), // reads the unit first: an unknown id is NOT_FOUND
  upsert: (d) => d.upsert('s', 'a', { a: 'x' }),
  runRetention: (d) => d.runRetention({ default: { keepLast: 1 } }),
  compact: (d) => d.compact(),
}

for (const [name, write] of Object.entries(WRITES)) {
  test(`${name} on a readOnly handle throws READ_ONLY`, () => {
    db.defineSchema('s', [{ name: 'a', type: 'text', unique: true }])
    const seed = stored('seed', 'n')
    db.update(seed, 'seed 2')
    db.update(seed, 'seed 3') // three revisions: retention has something to prune
    db.close()
    const ro = open(join(dir, 'db'), { readOnly: true })
    try {
      const seq = ro.lastCommitSeq()
      assert.equal(codeOf(() => write(ro, seed)), 'READ_ONLY')
      assert.equal(ro.lastCommitSeq(), seq)
      assert.equal(ro.query({ unitType: 'n' }).length, 1)
      assert.equal(ro.verify().clean, true)
    } finally {
      ro.close()
      db = open(join(dir, 'db'))
    }
  })
}

test('a no-op commit or retention on a readOnly handle writes nothing and does not throw', () => {
  stored()
  db.close()
  const ro = open(join(dir, 'db'), { readOnly: true })
  assert.equal(ro.commit([]).commitSeq, undefined)
  assert.equal(ro.runRetention({ default: { keepLast: 1 } }).revisionsPruned, 0)
  ro.close()
  db = open(join(dir, 'db'))
})

test('a readOnly handle reads beside the writer; refresh picks up its commits; a read-write handle refuses refresh', () => {
  const first = stored('one')
  const reader = open(join(dir, 'db'), { readOnly: true })
  try {
    const second = stored('two')
    assert.equal(reader.read(second), null, 'reads the commits made up to the open')
    assert.equal(reader.refresh(), db.lastCommitSeq())
    assert.equal(reader.read(second)?.payload?.toString(), 'two')
    assert.equal(reader.read(first)?.payload?.toString(), 'one')
    assert.equal(codeOf(() => db.refresh()), 'INVALID_ARGUMENT')
    assert.equal(codeOf(() => open(join(dir, 'nothing'), { readOnly: true })), 'DATABASE_NOT_FOUND')
  } finally {
    reader.close()
  }
})

test('a damaged block throws CORRUPTION and never returns wrong data', () => {
  const expected = new Map<string, Buffer>()
  for (let i = 0; i < 100; i++) {
    const payload = Buffer.from(`${i}:`.repeat(1500))
    expected.set(db.store('n', payload), payload)
  }
  db.compact()
  db.close()
  const table = readdirSync(join(dir, 'db')).find((f) => f.endsWith('.sst'))
  assert.ok(table)
  const path = join(dir, 'db', table)
  const raw = readFileSync(path)
  raw[Math.floor(raw.length / 2)]! ^= 0xff
  copyFileSync(path, path + '.bak')
  writeFileSync(path, raw)
  const damaged = open(join(dir, 'db'))
  let corrupt = 0
  for (const [id, payload] of expected) {
    const code = outcome(() => assert.deepEqual(damaged.read(id)?.payload, payload))
    if (code === 'CORRUPTION') corrupt++
    else assert.equal(code, 'ok', 'a read never returns wrong data')
  }
  assert.ok(corrupt >= 1)
  const report = damaged.verify()
  assert.equal(report.clean, false)
  assert.ok(report.storageError)
  damaged.close()
  db = open(join(dir, 'other'))
})

test('a reader of the same process never sees half of a change set', () => {
  for (let i = 0; i < 150; i++) {
    db.commit([{ op: 'store', unitType: 'pair', payload: `${i}a` }, { op: 'store', unitType: 'pair', payload: `${i}b` }])
    const seen = db.query({ unitType: 'pair' }).map((u) => u.payload?.toString() ?? '')
    const as = new Set(seen.filter((p) => p.endsWith('a')).map((p) => p.slice(0, -1)))
    const bs = new Set(seen.filter((p) => p.endsWith('b')).map((p) => p.slice(0, -1)))
    assert.deepEqual([...as].sort(), [...bs].sort())
  }
})

// ---------------------------------------------------------------- boundary fuzzing: tables and a seeded generator

const show = (v: unknown): string => (typeof v === 'symbol' || typeof v === 'function' ? typeof v : typeof v === 'bigint' ? `${v}n` : (JSON.stringify(v) ?? String(v)))
const STRINGS: unknown[] = [
  undefined, null, 0, -1, 1, 1.5, Number.NaN, 2n ** 63n, true, [], {}, Symbol('s'), (): number => 1, Buffer.alloc(0), Buffer.from([0xff]),
  '', ' ', 'a', 'a'.repeat(64), 'a'.repeat(65), 'a'.repeat(254), 'a'.repeat(255), 'a'.repeat(256), 'a'.repeat(5000), '\0', 'a\0b', 'é',
  'é'.repeat(200), '😀', '😀'.repeat(17), 'é', '‮​﻿', '\u{10ffff}', '\ud800', '\udfff', 'a\ud800b',
]
const NUMBERS: unknown[] = [
  undefined, null, 0, -1, 1, 2 ** 31, 2 ** 32, 2 ** 53, 2 ** 53 + 2, 2 ** 63, 2 ** 64, -(2 ** 63), 1.5, Number.NaN, Number.POSITIVE_INFINITY,
  Number.NEGATIVE_INFINITY, -0, true, false, '1', Buffer.from('1'), [], {}, 2n ** 63n, 2n ** 64n, Symbol('n'),
]
// what napi itself throws for a value of the wrong JS type (before the engine is reached)
const WRONG_TYPE = new Set(['StringExpected', 'BooleanExpected', 'ArrayExpected', 'ObjectExpected', 'InvalidArg', 'NumberExpected', 'GenericFailure'])
const KERNO_CODES = new Set([
  'NOT_FOUND', 'DELETED', 'LOCKED', 'CORRUPTION', 'INVALID_ARGUMENT', 'SCHEMA_EXISTS', 'NOT_INDEXED', 'UNKNOWN_TYPE', 'NOT_UNIQUE', 'BAD_ENDPOINT',
  'PAYLOAD_TOO_LARGE', 'CHANGESET_TOO_LARGE', 'READ_ONLY', 'DATABASE_NOT_FOUND', 'HAS_RELATIONS', 'PRUNED', 'SCHEMA', 'CLOSED', 'KERNO',
])

/** `fn` either works, throws a documented code, or (for a wrong JS type, `strict` off) throws what napi throws for it. */
function survives(label: string, fn: () => unknown, strict: boolean): void {
  const out = outcome(fn)
  if (out === 'ok' || KERNO_CODES.has(out)) return
  // a value that is not an object where an object is required: napi's own TypeError, with no code
  if (!strict && (WRONG_TYPE.has(out) || out === 'no-code:TypeError')) return
  assert.fail(`${label} threw ${out}`)
}

test('every call survives every odd string, with a documented code', () => {
  db.defineSchema('user', [{ name: 'email', type: 'text', unique: true }, { name: 'age', type: 'int', indexed: true }])
  const a = stored('a')
  const b = stored('b')
  db.setFulltext('note', true)
  const calls: Array<[string, (v: unknown) => unknown]> = [
    ['store.type', (v) => loose(db).store(v, 'p')],
    ['store.payload', (v) => loose(db).store('t', v)],
    ['read', (v) => loose(db).read(v)],
    ['update.id', (v) => loose(db).update(v, 'p')],
    ['update.payload', (v) => loose(db).update(a, v)],
    ['delete', (v) => loose(db).delete(v)],
    ['relate.from', (v) => loose(db).relate(v, b, 'r')],
    ['relate.type', (v) => loose(db).relate(a, b, v)],
    ['related.id', (v) => loose(db).related(v, 'out')],
    ['related.dir', (v) => loose(db).related(a, v)],
    ['related.type', (v) => loose(db).related(a, 'out', v)],
    ['relations', (v) => loose(db).relations(v)],
    ['unrelate', (v) => loose(db).unrelate(v)],
    ['query.type', (v) => loose(db).query({ unitType: v })],
    ['history.id', (v) => loose(db).history(v)],
    ['readAsOf.id', (v) => loose(db).readAsOf(v, 1)],
    ['setFulltext', (v) => loose(db).setFulltext(v, true)],
    ['fulltext', (v) => loose(db).fulltext(v)],
    ['searchText.query', (v) => loose(db).searchText(v)],
    ['searchText.types', (v) => loose(db).searchText('a', { types: [v] })],
    ['defineSchema.type', (v) => loose(db).defineSchema(v, [{ name: 'f', type: 'text' }])],
    ['defineSchema.name', (v) => loose(db).defineSchema('sch', [{ name: v, type: 'text' }])],
    ['defineSchema.fieldType', (v) => loose(db).defineSchema('sch2', [{ name: 'f', type: v }])],
    ['createIndex', (v) => loose(db).createIndex('user', v)],
    ['storeDocument.type', (v) => loose(db).storeDocument(v, { a: 1 })],
    ['storeDocument.value', (v) => loose(db).storeDocument('doc', { k: v })],
    ['upsert.field', (v) => loose(db).upsert('user', v, { email: 'a' })],
    ['upsert.value', (v) => loose(db).upsert('user', 'email', { email: v })],
    ['find.type', (v) => loose(db).find(v, 'age', 1)],
    ['find.field', (v) => loose(db).find('user', v, 1)],
    ['find.text', (v) => loose(db).find('user', 'email', v)],
    ['find.int', (v) => loose(db).find('user', 'age', v)],
    ['execute', (v) => loose(db).execute(v)],
    ['hybrid.text', (v) => loose(db).hybridSearch({ text: v })],
    ['hybrid.textTypes', (v) => loose(db).hybridSearch({ text: 'a', textTypes: [v] })],
    ['hybrid.seeds', (v) => loose(db).hybridSearch({ relations: { seeds: [v] } })],
    ['hybrid.vectorId', (v) => loose(db).hybridSearch({ vectors: { query: [1], items: [{ id: v, vector: [1] }] } })],
    ['hybrid.metric', (v) => loose(db).hybridSearch({ vectors: { query: [1], items: [{ id: a, vector: [1] }], metric: v } })],
    ['hybrid.fusion', (v) => loose(db).hybridSearch({ text: 'a', fusion: v })],
    ['commit.type', (v) => loose(db).commit([{ op: 'store', unitType: v, payload: 'p' }])],
    ['commit.payload', (v) => loose(db).commit([{ op: 'store', unitType: 't', payload: v }])],
    ['commit.id', (v) => loose(db).commit([{ op: 'update', id: v, payload: 'p' }])],
    ['commit.opname', (v) => loose(db).commit([{ op: v }])],
    ['commit.reason', (v) => loose(db).commit(ops(1), { reason: v })],
    ['commit.actor', (v) => loose(db).commit(ops(1), { actor: v })],
    ['commit.source', (v) => loose(db).commit(ops(1), { source: v })],
    ['commit.relationType', (v) => loose(db).commit([...ops(1), { op: 'relate', from: 0, to: 0, relationType: v }])],
    ['commit.relateEnd', (v) => loose(db).commit([...ops(1), { op: 'relate', from: v, to: 0, relationType: 'r' }])],
    ['commit.upsertValue', (v) => loose(db).commit([{ op: 'upsert', unitType: 'user', field: 'email', document: { email: v } }])],
    ['runRetention.byType', (v) => loose(db).runRetention({ byType: { [typeof v === 'symbol' ? 's' : String(v)]: { keepLast: 1 } } })],
    ['open.path', (v) => { const h = loose({ open } as unknown as Db).open(typeof v === 'string' ? join(dir, 'p', v) : v); if (h) (h as Db).close() }],
  ]
  for (const [label, call] of calls) for (const v of STRINGS) survives(`${label}(${show(v)})`, () => call(v), false)
})

test('every numeric option accepts a whole number or throws INVALID_ARGUMENT, whatever it is given', () => {
  db.defineSchema('user', [{ name: 'age', type: 'int', indexed: true }])
  const a = stored('a')
  db.setFulltext('note', true)
  let n = 0
  const calls: Array<[string, (v: unknown) => unknown]> = [
    ['history.limit', (v) => loose(db).history(a, { limit: v })],
    ['history.fromRevision', (v) => loose(db).history(a, { fromRevision: v })],
    ['readAsOf.seq', (v) => loose(db).readAsOf(a, v)],
    ['query.schemaVersion', (v) => loose(db).query({ schemaVersion: v })],
    ['searchText.limit', (v) => loose(db).searchText('a', { limit: v })],
    ['hybrid.limit', (v) => loose(db).hybridSearch({ text: 'a', limit: v })],
    ['hybrid.candidates', (v) => loose(db).hybridSearch({ text: 'a', candidates: v })],
    ['hybrid.rrfK', (v) => loose(db).hybridSearch({ text: 'a', rrfK: v })],
    ['hybrid.weight', (v) => loose(db).hybridSearch({ text: 'a', weights: { keyword: v } })],
    ['hybrid.maxDepth', (v) => loose(db).hybridSearch({ relations: { seeds: [a], maxDepth: v } })],
    ['hybrid.visitBudget', (v) => loose(db).hybridSearch({ relations: { seeds: [a], visitBudget: v } })],
    ['commit.restoreRevision', (v) => loose(db).commit([{ op: 'restore', id: a, revision: v }])],
    ['commit.relateEnd', (v) => loose(db).commit([...ops(1), { op: 'relate', from: 0, to: v, relationType: 'r' }])],
    ['retention.keepLast', (v) => loose(db).runRetention({ default: { keepLast: v } })],
    ['retention.keepSince', (v) => loose(db).runRetention({ default: { keepSince: v } })],
    ['open.maxBatchOps', (v) => loose({ open } as unknown as Db).open(join(dir, `n${n++}`), { maxBatchOps: v })],
  ]
  for (const [label, call] of calls) {
    for (const v of NUMBERS) {
      if (label === 'commit.relateEnd' && (typeof v === 'string' || Buffer.isBuffer(v))) continue // a string end is a unit id
      const out = outcome(() => call(v))
      const label2 = `${label}(${show(v)})`
      assert.ok(out === 'ok' || KERNO_CODES.has(out), `${label2} threw ${out}: a numeric option never leaks a raw napi error`)
      if (v === undefined || v === 0 || v === 1) continue
      if (typeof v !== 'number' || !Number.isInteger(v) || v < 0 || v > 2 ** 53) {
        assert.ok(out !== 'ok' || ['retention.keepSince', 'query.schemaVersion'].includes(label) && v === 0, `${label2} was accepted`)
      }
    }
  }
})

/** A small seeded generator: runs are reproducible, a failure prints its seed. */
function rng(seed: number): () => number {
  let s = seed >>> 0
  return () => {
    s = (Math.imul(s, 1664525) + 1013904223) >>> 0
    return s / 2 ** 32
  }
}

test('property: any payload and unit type round-trip, or the type is refused exactly when it is not 1 to 64 bytes', () => {
  const alphabet = ['a', 'é', '日', '😀', '\0', ' ', '́', '\u{10ffff}', 'z']
  for (let seed = 1; seed <= 120; seed++) {
    const r = rng(seed)
    const pick = (max: number): string => Array.from({ length: Math.floor(r() * max) }, () => alphabet[Math.floor(r() * alphabet.length)]).join('')
    const unitType = pick(40)
    const payload = r() < 0.5 ? pick(300) : Buffer.from(Array.from({ length: Math.floor(r() * 400) }, () => Math.floor(r() * 256)))
    const bytes = Buffer.byteLength(unitType)
    const label = `seed ${seed}: ${JSON.stringify(unitType)}`
    if (bytes >= 1 && bytes <= 64) {
      const id = db.store(unitType, payload)
      assert.deepEqual(db.read(id)?.payload, Buffer.from(payload), label)
      assert.equal(db.update(id, payload, { ifChanged: true }), false, label)
      assert.equal(db.history(id, { withPayload: true })[0]?.payload?.toString('hex'), Buffer.from(payload).toString('hex'), label)
    } else {
      assert.equal(codeOf(() => db.store(unitType, payload)), 'INVALID_ARGUMENT', label)
      assert.equal(codeOf(() => db.query({ unitType })), 'INVALID_ARGUMENT', label)
    }
  }
})

test('property: an indexed value is accepted exactly when its encoding fits 254 bytes', () => {
  const alphabet = ['a', 'é', '\0', '😀']
  for (let seed = 1; seed <= 120; seed++) {
    const r = rng(seed * 7919)
    const kind = r() < 0.5 ? 'text' : 'bytes'
    const text = Array.from({ length: Math.floor(r() * 260) }, () => alphabet[Math.floor(r() * (kind === 'text' ? 4 : 3))]).join('')
    const raw = Buffer.from(text)
    const value = kind === 'text' ? text : raw
    const name = `fz_${kind}`
    db.defineSchema(name, [{ name: 'k', type: kind, indexed: true }])
    const fits = raw.length + raw.filter((b) => b === 0).length <= 254
    const label = `seed ${seed}: ${kind} ${raw.length} bytes`
    if (fits) assert.ok(db.find(name, 'k', value).length >= 0 && db.storeDocument(name, { k: value }), label)
    else {
      assert.equal(codeOf(() => db.storeDocument(name, { k: value })), 'SCHEMA', label)
      assert.equal(codeOf(() => db.find(name, 'k', value)), 'INVALID_ARGUMENT', label)
    }
  }
})

test('property: a mixed batch commits whole or not at all', () => {
  db.defineSchema('fz_user', [{ name: 'email', type: 'text', unique: true }])
  const keeps = Array.from({ length: 15 }, () => db.store('fz_keep', 'k'))
  let next = 0
  for (let seed = 1; seed <= 80; seed++) {
    const r = rng(seed * 104729)
    const list: CommitOp[] = []
    const made: number[] = []
    let broken = false
    const used = next
    for (let step = Math.floor(r() * 12); step > 0; step--) {
      const i = list.length
      const kind = Math.floor(r() * 8)
      if (kind === 0) { list.push({ op: 'store', unitType: 'fz_store', payload: `p${i}` }); made.push(i) }
      else if (kind === 1 && next < keeps.length) list.push({ op: 'update', id: keeps[next++]!, payload: `u${i}` })
      else if (kind === 2) { list.push({ op: 'upsert', unitType: 'fz_user', field: 'email', document: { email: `e${i % 3}` } }); made.push(i) }
      else if (kind === 3 && made.length) list.push({ op: 'relate', from: made[0]!, to: made.at(-1)!, relationType: 'fz_rel' })
      else if (kind === 4 && made.length && next < keeps.length) list.push({ op: 'relate', from: keeps[next++]!, to: made.at(-1)!, relationType: 'fz_rel' })
      else if (kind === 5) { list.push({ op: 'relate', from: i, to: i + 5, relationType: 'fz_rel' }); broken = true }
      else if (kind === 6) { list.push({ op: 'store', unitType: 'x'.repeat(65), payload: 'p' }); broken = true }
    }
    const before = [db.lastCommitSeq(), db.query({ unitType: 'fz_store' }).length, db.relations().length]
    const label = `seed ${seed}`
    if (broken) {
      assert.ok(KERNO_CODES.has(codeOf(() => db.commit(list))), label)
      next = used
      assert.deepEqual([db.lastCommitSeq(), db.query({ unitType: 'fz_store' }).length, db.relations().length], before, label)
    } else {
      const out = db.commit(list)
      assert.equal(out.units.length, list.filter((o) => o.op === 'store').length, label)
      assert.equal(db.relations().length, before[2]! + list.filter((o) => o.op === 'relate').length, label)
    }
  }
})

test('unusual Unicode round-trips; a lone surrogate becomes U+FFFD, as in Buffer.from', () => {
  for (const text of ['é', '‮​﻿', '\u{10ffff}', '\0', 'a\0b', '日本語', '👨‍👩‍👧‍👦', '�']) {
    const id = db.store(text, text)
    const unit = db.read(id)
    assert.equal(unit?.unitType, text)
    assert.equal(unit?.payload?.toString(), text)
    assert.deepEqual(db.query({ unitType: text }).map((u) => u.id), [id])
  }
  for (const lone of ['\ud800', '\udfff', 'a\udc00']) {
    assert.deepEqual(db.read(db.store('t', lone))?.payload, Buffer.from(lone), lone)
  }
  db.defineSchema('u', [{ name: 'k', type: 'text', indexed: true }])
  db.storeDocument('u', { k: '😀'.repeat(63) + 'ab' })
  assert.equal(codeOf(() => db.storeDocument('u', { k: '😀'.repeat(63) + 'abc' })), 'SCHEMA')
})

test('a path that is a file is a Kerno error; an empty one is INVALID_ARGUMENT and creates nothing', () => {
  const file = join(dir, 'file')
  writeFileSync(file, 'x')
  assert.ok(KERNO_CODES.has(codeOf(() => open(file))))
  const work = mkdtempSync(join(dir, 'cwd-'))
  const before = process.cwd()
  process.chdir(work)
  try {
    assert.equal(codeOf(() => open('')), 'INVALID_ARGUMENT')
    assert.equal(codeOf(() => open('', { readOnly: true })), 'INVALID_ARGUMENT')
    assert.deepEqual(readdirSync(work), [], 'an empty path must not create a database in the working directory')
  } finally {
    process.chdir(before)
  }
})

// ---------------------------------------------------------------- docs/bindings-coverage.md

test('the coverage table names the Node calls that exist', () => {
  const table = readFileSync(new URL('docs/bindings-coverage.md', ROOT), 'utf8')
  const rows = table.split('\n').filter((l) => l.startsWith('| `')).map((l) => l.split('|'))
  assert.ok(rows.length > 30)
  const methods = new Set(Object.getOwnPropertyNames(Object.getPrototypeOf(db)))
  for (const cells of rows) {
    const node = (cells[3] ?? '').trim()
    if (node === 'none') {
      assert.match(cells[4] ?? '', /Gap/)
      continue
    }
    for (const m of node.matchAll(/\bdb\.(\w+)\(/g)) assert.ok(methods.has(m[1]!), `the coverage table lists db.${m[1]}() for Node, which does not exist`)
  }
  for (const gap of ['dropIndex', 'schema', 'findRange', 'explain', 'compare', 'searchTextTypes']) assert.ok(!methods.has(gap), gap)
})
