import assert from 'node:assert/strict'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { readdirSync, readFileSync, writeFileSync } from 'node:fs'
import { afterEach, beforeEach, test } from 'node:test'

import { open, type Db } from '../index.js'

const MISSING = '00'.repeat(16)

/** The `code` of the error `fn` throws; fails the test when it does not throw. */
function codeOf(fn: () => unknown): string | undefined {
  try {
    fn()
  } catch (e) {
    assert.ok(e instanceof Error)
    return (e as Error & { code?: string }).code
  }
  assert.fail('expected a throw')
}

let dir: string
let db: Db

beforeEach(() => {
  dir = mkdtempSync(join(tmpdir(), 'kernodb-'))
  db = open(dir)
})

afterEach(() => {
  db.close()
  rmSync(dir, { recursive: true, force: true })
})

test('store and read a string and a Buffer payload', () => {
  const a = db.store('note', 'Rust has no GC')
  const b = db.store('blob', Buffer.from([0, 1, 2, 255]))
  assert.match(a, /^[0-9a-f]{32}$/)

  const note = db.read(a)
  assert.ok(note)
  assert.equal(note.id, a)
  assert.equal(note.unitType, 'note')
  assert.equal(note.lifecycle, 'active')
  assert.equal(note.payload?.toString('utf8'), 'Rust has no GC')
  assert.ok(note.createdAtUs > 0)

  assert.deepEqual(db.read(b)?.payload, Buffer.from([0, 1, 2, 255]))
})

test('read of an unknown id is null', () => {
  assert.equal(db.read('00'.repeat(16)), null)
})

test('relate and related follow an edge in both directions', () => {
  const note = db.store('note', 'x')
  const topic = db.store('topic', 'memory')
  const edge = db.relate(note, topic, 'about')

  const out = db.related(note, 'out')
  assert.deepEqual(out, [{ id: edge, from: note, to: topic, relationType: 'about' }])
  assert.equal(db.related(topic, 'in', 'about').length, 1)
  assert.equal(db.related(topic, 'out').length, 0)
  assert.equal(db.related(note, 'out', 'other').length, 0)
})

test('query filters by type, and with no filter returns every unit', () => {
  const n1 = db.store('note', '1')
  const n2 = db.store('note', '2')
  db.store('topic', 't')

  assert.deepEqual(db.query({ unitType: 'note' }).map((u) => u.id).sort(), [n1, n2].sort())
  assert.equal(db.query().length, 3)
  assert.equal(db.query({ unitType: 'none' }).length, 0)
  assert.equal(db.query({ unitType: 'note', schemaVersion: 2 }).length, 0)
})

test('data survives close and reopen', () => {
  const id = db.store('note', 'kept')
  db.close()
  db = open(dir)
  assert.equal(db.read(id)?.payload?.toString(), 'kept')
})

test('after close every call throws CLOSED, and close is idempotent', () => {
  db.close()
  assert.equal(codeOf(() => db.query()), 'CLOSED')
  assert.equal(codeOf(() => db.store('note', 'x')), 'CLOSED')
  assert.throws(() => db.query(), /closed/)
  db.close()
})

// ---- errors ----

test('an unknown unit is NOT_FOUND', () => {
  assert.equal(codeOf(() => db.update(MISSING, 'x')), 'NOT_FOUND')
  assert.equal(codeOf(() => db.relate(db.store('a', 'x'), MISSING, 'about')), 'NOT_FOUND')
  assert.equal(codeOf(() => db.unrelate(MISSING)), 'NOT_FOUND')
})

test('writing to a deleted unit is DELETED', () => {
  const id = db.store('note', 'x')
  db.delete(id)
  assert.equal(codeOf(() => db.update(id, 'y')), 'DELETED')
})

test('opening a held directory is LOCKED', () => {
  assert.equal(codeOf(() => open(dir)), 'LOCKED')
})

test('a bad id, type name, direction or limit is INVALID_ARGUMENT', () => {
  assert.equal(codeOf(() => db.read('nope')), 'INVALID_ARGUMENT')
  assert.throws(() => db.read('nope'), /invalid id/)
  assert.equal(codeOf(() => db.store('', 'x')), 'INVALID_ARGUMENT')
  // @ts-expect-error: not a direction
  assert.equal(codeOf(() => db.related(MISSING, 'sideways')), 'INVALID_ARGUMENT')
  assert.equal(codeOf(() => db.history(MISSING, { limit: 0 })), 'INVALID_ARGUMENT')
  assert.equal(codeOf(() => db.readAsOf(MISSING, -1)), 'INVALID_ARGUMENT')
})

test('a payload over 1 MiB is PAYLOAD_TOO_LARGE, exactly 1 MiB is stored', () => {
  assert.equal(codeOf(() => db.store('blob', Buffer.alloc(1024 * 1024 + 1))), 'PAYLOAD_TOO_LARGE')
  const id = db.store('blob', Buffer.alloc(1024 * 1024))
  assert.equal(db.read(id)?.payload?.length, 1024 * 1024)
})

test('deleting a unit that has relations is HAS_RELATIONS', () => {
  const a = db.store('note', 'a')
  const b = db.store('note', 'b')
  db.relate(a, b, 'next')
  assert.equal(codeOf(() => db.delete(a)), 'HAS_RELATIONS')
  assert.equal(db.read(a)?.lifecycle, 'active')
})

test('a document the schema refuses is SCHEMA', () => {
  db.defineSchema('user', [{ name: 'email', type: 'text', required: true, unique: true }])
  db.storeDocument('user', { email: 'a@x.io' })
  assert.equal(codeOf(() => db.storeDocument('user', { email: 'a@x.io' })), 'SCHEMA')
  assert.equal(codeOf(() => db.storeDocument('user', { nickname: 'a' })), 'SCHEMA')
  assert.equal(codeOf(() => db.store('user', 'not a document')), 'SCHEMA')
})

test('a damaged file is CORRUPTION', () => {
  for (let i = 0; i < 50; i++) db.store('note', `payload ${i}`)
  db.close()
  const sst = readdirSync(dir).find((f) => f.endsWith('.sst'))
  assert.ok(sst)
  const file = join(dir, sst)
  const bytes = readFileSync(file)
  for (let i = 100; i < 5000; i += 7) bytes[i] ^= 0xff
  writeFileSync(file, bytes)
  assert.equal(codeOf(() => open(dir)), 'CORRUPTION')
  db = open(mkdtempSync(join(tmpdir(), 'kernodb-')))
})

// ---- writes ----

test('update replaces the payload and reports whether a revision was written', () => {
  const id = db.store('note', 'one')
  assert.equal(db.update(id, 'two'), true)
  assert.equal(db.read(id)?.payload?.toString(), 'two')
  assert.equal(db.read(id)?.revision, 2)
  // Same payload: by default a revision is written, with ifChanged it is not.
  assert.equal(db.update(id, 'two'), true)
  assert.equal(db.update(id, 'two', {}), true)
  assert.equal(db.update(id, 'two', { ifChanged: false }), true)
  assert.equal(db.update(id, 'two', { ifChanged: true }), false)
  assert.equal(db.update(id, Buffer.from('three'), { ifChanged: true }), true)
  assert.equal(db.read(id)?.revision, 6)
})

test('delete keeps the history; cascade removes the relations first', () => {
  const a = db.store('note', 'a')
  const b = db.store('note', 'b')
  const edge = db.relate(a, b, 'next')

  db.delete(a, { cascade: true })
  assert.equal(db.read(a)?.lifecycle, 'deleted')
  assert.equal(db.read(a)?.payload, undefined)
  assert.deepEqual(db.related(b, 'in'), [])
  assert.equal(db.relations().some((r) => r.id === edge), false)
  assert.equal(db.history(a)[0]?.kind, 'delete')

  db.delete(b)
  assert.equal(db.read(b)?.lifecycle, 'deleted')
})

test('unrelate removes one relation', () => {
  const a = db.store('note', 'a')
  const b = db.store('note', 'b')
  const keep = db.relate(a, b, 'keep')
  const drop = db.relate(a, b, 'drop')
  db.unrelate(drop)
  assert.deepEqual(db.related(a, 'out').map((r) => r.id), [keep])
})

test('relate with unique reuses the relation, without it adds a parallel one', () => {
  const a = db.store('note', 'a')
  const b = db.store('note', 'b')
  const first = db.relate(a, b, 'next', { unique: true })
  assert.equal(db.relate(a, b, 'next', { unique: true }), first)
  assert.equal(db.relations('next').length, 1)
  const second = db.relate(a, b, 'next')
  assert.notEqual(second, first)
  assert.equal(db.relations('next').length, 2)
  assert.equal(db.relate(a, b, 'next', { unique: false }) === first, false)
})

test('relations lists every live relation, optionally of one type', () => {
  const a = db.store('note', 'a')
  const b = db.store('note', 'b')
  const c = db.store('note', 'c')
  const ab = db.relate(a, b, 'next')
  const bc = db.relate(b, c, 'cites')
  assert.deepEqual(db.relations().map((r) => r.id).sort(), [ab, bc].sort())
  assert.deepEqual(db.relations('cites'), [{ id: bc, from: b, to: c, relationType: 'cites' }])
  assert.deepEqual(db.relations('none'), [])
})

// ---- commit ----

test('commit stores units and wires them with indexes, in one commit', () => {
  const anchor = db.store('topic', 'memory')
  const done = db.commit(
    [
      { op: 'store', unitType: 'note', payload: 'first' },
      { op: 'store', unitType: 'note', payload: Buffer.from('second') },
      { op: 'relate', from: 0, to: 1, relationType: 'next' },
      { op: 'relate', from: 1, to: anchor, relationType: 'about' },
    ],
    { reason: 'import', actor: 'tester', source: Buffer.from('unit-test') },
  )
  assert.equal(done.units.length, 2)
  assert.equal(done.relations.length, 2)
  assert.equal(done.unchanged, 0)
  assert.equal(done.commitSeq, db.lastCommitSeq())
  assert.deepEqual(db.related(done.units[0]!, 'out').map((r) => r.to), [done.units[1]])
  assert.equal(db.read(done.units[1]!)?.payload?.toString(), 'second')
})

test('commit runs update, update ifChanged, archive, unarchive and restore', () => {
  const id = db.store('note', 'v1')
  db.update(id, 'v2')
  const done = db.commit([{ op: 'update', id, payload: 'v2', ifChanged: true }])
  assert.equal(done.unchanged, 1)

  db.commit([{ op: 'archive', id }])
  assert.equal(db.read(id)?.lifecycle, 'archived')
  db.commit([{ op: 'unarchive', id }])
  assert.equal(db.read(id)?.lifecycle, 'active')

  db.commit([{ op: 'update', id, payload: 'v3' }])
  db.commit([{ op: 'restore', id, revision: 1 }])
  assert.equal(db.read(id)?.payload?.toString(), 'v1')
  assert.equal(db.history(id, { limit: 1 })[0]?.kind, 'restore')
})

test('commit runs delete, delete_cascade, relate unique and unrelate', () => {
  const [a, b, c] = ['a', 'b', 'c'].map((x) => db.store('note', x)) as [string, string, string]
  const edge = db.relate(a, b, 'next')

  const unique = db.commit([{ op: 'relate', from: a, to: b, relationType: 'next', unique: true }])
  assert.deepEqual(unique.relations, [edge])

  db.commit([{ op: 'unrelate', relationId: edge }, { op: 'delete', id: a }])
  assert.equal(db.read(a)?.lifecycle, 'deleted')

  db.relate(b, c, 'next')
  db.commit([{ op: 'delete_cascade', id: b }])
  assert.equal(db.read(b)?.lifecycle, 'deleted')
  assert.deepEqual(db.relations(), [])

  db.relate(c, db.store('note', 'd'), 'next')
  db.commit([{ op: 'delete', id: c, cascade: true }])
  assert.equal(db.read(c)?.lifecycle, 'deleted')
})

test('commit is atomic: one bad op writes nothing', () => {
  const seq = db.lastCommitSeq()
  assert.equal(
    codeOf(() =>
      db.commit([
        { op: 'store', unitType: 'note', payload: 'x' },
        { op: 'update', id: MISSING, payload: 'y' },
      ]),
    ),
    'NOT_FOUND',
  )
  assert.equal(db.lastCommitSeq(), seq)
  assert.equal(db.query().length, 0)
})

test('commit refuses an unknown or incomplete op', () => {
  // @ts-expect-error: not an op
  assert.equal(codeOf(() => db.commit([{ op: 'explode' }])), 'INVALID_ARGUMENT')
  assert.equal(codeOf(() => db.commit([{ op: 'store', unitType: 'note' }])), 'INVALID_ARGUMENT')
  assert.equal(codeOf(() => db.commit([{ op: 'restore', id: MISSING }])), 'INVALID_ARGUMENT')
  assert.equal(codeOf(() => db.commit([{ op: 'relate', from: 0, relationType: 'x' }])), 'INVALID_ARGUMENT')
})

test('commit records reason, actor and source with the revision', () => {
  const [id] = db.commit([{ op: 'store', unitType: 'note', payload: 'x' }], { reason: 'why' }).units
  assert.ok(id)
  assert.equal(db.history(id).length, 1)
  assert.equal(codeOf(() => db.commit([{ op: 'store', unitType: 'note', payload: 'x' }], { actor: 'a'.repeat(257) })), 'INVALID_ARGUMENT')
})

test('commit with no ops writes nothing and reports no sequence', () => {
  const done = db.commit([])
  assert.equal(done.commitSeq, undefined)
  assert.deepEqual(done, { units: [], relations: [], unchanged: 0 })
  assert.equal(db.lastCommitSeq(), 0)
})

// ---- history and maintenance ----

test('history lists revisions newest first, with limit and withPayload', () => {
  const id = db.store('note', 'v1')
  db.update(id, 'v2')
  db.update(id, 'v3')

  const all = db.history(id)
  assert.deepEqual(all.map((r) => r.revision), [3, 2, 1])
  assert.deepEqual(all.map((r) => r.kind), ['update', 'update', 'create'])
  assert.equal(all[0]?.payload, undefined)
  assert.match(all[0]!.contentHash, /^[0-9a-f]{64}$/)
  assert.equal(all[0]!.lifecycle, 'active')
  assert.ok(all[0]!.commitSeq > all[2]!.commitSeq)

  assert.deepEqual(db.history(id, { limit: 2 }).map((r) => r.revision), [3, 2])
  assert.equal(db.history(id, { withPayload: true })[0]?.payload?.toString(), 'v3')
  assert.deepEqual(db.history(MISSING), [])
})

test('readAsOf reads the revision in force at a commit', () => {
  const id = db.store('note', 'v1')
  const seq1 = db.lastCommitSeq()
  db.update(id, 'v2')
  const seq2 = db.lastCommitSeq()
  db.delete(id)

  assert.equal(db.readAsOf(id, 0), null)
  assert.equal(db.readAsOf(id, seq1)?.payload?.toString(), 'v1')
  assert.equal(db.readAsOf(id, seq2)?.payload?.toString(), 'v2')
  assert.equal(db.readAsOf(id, seq2)?.revision, 2)
  assert.equal(db.readAsOf(id, db.lastCommitSeq())?.kind, 'delete')
  assert.equal(db.readAsOf(id, 1e9)?.kind, 'delete')
  assert.equal(db.readAsOf(MISSING, seq1), null)
})

test('lastCommitSeq starts at 0 and grows with each commit', () => {
  assert.equal(db.lastCommitSeq(), 0)
  db.store('note', 'x')
  const one = db.lastCommitSeq()
  assert.ok(one > 0)
  db.store('note', 'y')
  assert.ok(db.lastCommitSeq() > one)
})

test('verify reports a clean store with its counts', () => {
  const a = db.store('note', 'a')
  db.update(a, 'b')
  db.relate(a, db.store('note', 'c'), 'next')
  const report = db.verify()
  assert.equal(report.clean, true)
  assert.equal(report.units, 2)
  assert.equal(report.revisions, 3)
  assert.equal(report.relations, 1)
  assert.equal(report.commitSeq, db.lastCommitSeq())
  assert.deepEqual(report.findings, [])
  assert.equal(report.findingsOmitted, 0)
  assert.equal(report.storageError, undefined)
})

test('compact reports the stored size', () => {
  for (let i = 0; i < 20; i++) db.store('note', `payload ${i}`)
  const report = db.compact()
  assert.ok(report.before === undefined || report.before >= 0)
  assert.ok(report.after === undefined || report.after >= 0)
  assert.equal(db.query().length, 20)
})

test('setFulltext switches keyword search on and off per type', () => {
  const note = db.store('note', 'the quick brown fox')
  const keywords = () => db.hybridSearch({ text: 'fox' }).map((h) => h.id)

  // Off by default: no unit is keyword-indexed.
  assert.deepEqual(keywords(), [])
  assert.equal(db.fulltext('note'), false)
  db.setFulltext('note', true)
  assert.equal(db.fulltext('note'), true)
  assert.deepEqual(keywords(), [note])
  // A unit stored after switching on is indexed too.
  const later = db.store('note', 'another fox')
  assert.deepEqual(keywords().sort(), [note, later].sort())
  db.setFulltext('note', false)
  assert.deepEqual(keywords(), [])
})

// ---- schemas, indexes, documents ----

test('storeDocument and readDocument round-trip every value type', () => {
  const id = db.storeDocument('doc', {
    flag: true,
    count: 42,
    ratio: 0.5,
    title: 'héllo',
    blob: Buffer.from([1, 2, 3]),
    big: 9007199254740993n,
  })
  assert.deepEqual(db.readDocument(id), {
    flag: true,
    count: 42,
    ratio: 0.5,
    title: 'héllo',
    blob: Buffer.from([1, 2, 3]),
    big: 9007199254740993n,
  })
  assert.equal(db.readDocument(MISSING), null)
  db.delete(id)
  assert.equal(db.readDocument(id), null)
})

test('updateDocument replaces the document and keeps the history', () => {
  const id = db.storeDocument('doc', { n: 1 })
  db.updateDocument(id, { n: 2, extra: 'x' })
  assert.deepEqual(db.readDocument(id), { n: 2, extra: 'x' })
  assert.equal(db.history(id).length, 2)
  assert.equal(codeOf(() => db.updateDocument(MISSING, { n: 1 })), 'NOT_FOUND')
})

test('defineSchema types the fields: a whole number is a float where the schema says so', () => {
  db.defineSchema('measure', [
    { name: 'value', type: 'float', required: true },
    { name: 'count', type: 'int' },
  ])
  const id = db.storeDocument('measure', { value: 3, count: 7 })
  assert.deepEqual(db.readDocument(id), { value: 3, count: 7 })
  // 7.5 is not an int, and a text is not a float.
  assert.equal(codeOf(() => db.storeDocument('measure', { value: 1, count: 7.5 })), 'INVALID_ARGUMENT')
  assert.equal(codeOf(() => db.storeDocument('measure', { value: 'x' })), 'SCHEMA')
  assert.equal(codeOf(() => db.storeDocument('measure', { count: 1 })), 'SCHEMA')
  assert.equal(codeOf(() => db.defineSchema('bad', [{ name: 'f', type: 'decimal' as 'int' }])), 'INVALID_ARGUMENT')
})

test('a unique field refuses a second document with the same value', () => {
  db.defineSchema('user', [{ name: 'email', type: 'text', unique: true }])
  db.storeDocument('user', { email: 'a@x.io' })
  db.storeDocument('user', { email: 'b@x.io' })
  assert.equal(codeOf(() => db.storeDocument('user', { email: 'a@x.io' })), 'SCHEMA')
  assert.equal(db.find('user', 'email', 'b@x.io').length, 1)
})

test('createIndex and find look units up by an indexed field', () => {
  db.defineSchema('user', [
    { name: 'email', type: 'text', unique: true },
    { name: 'age', type: 'int' },
    { name: 'city', type: 'text', indexed: true },
  ])
  const ada = db.storeDocument('user', { email: 'ada@x.io', age: 36, city: 'London' })
  const bob = db.storeDocument('user', { email: 'bob@x.io', age: 36, city: 'Paris' })
  db.storeDocument('user', { email: 'cy@x.io', age: 20, city: 'Paris' })

  assert.deepEqual(db.find('user', 'email', 'ada@x.io').map((u) => u.id), [ada])
  assert.equal(db.find('user', 'city', 'Paris').length, 2)
  assert.equal(db.find('user', 'city', 'Rome').length, 0)

  // `age` is not indexed yet: createIndex builds it over the existing units.
  assert.equal(codeOf(() => db.find('user', 'age', 36)), 'INVALID_ARGUMENT')
  db.createIndex('user', 'age')
  assert.deepEqual(db.find('user', 'age', 36).map((u) => u.id).sort(), [ada, bob].sort())
  assert.equal(codeOf(() => db.createIndex('user', 'nope')), 'INVALID_ARGUMENT')
})

test('execute runs the query language and returns rows of units', () => {
  db.defineSchema('user', [{ name: 'age', type: 'int', indexed: true }])
  const old = db.storeDocument('user', { age: 70 })
  db.storeDocument('user', { age: 20 })

  const result = db.execute('FIND user AS u WHERE u.age = 70')
  assert.deepEqual(result.aliases, ['u'])
  assert.equal(result.rows.length, 1)
  assert.equal(result.rows[0]?.[0]?.id, old)
  assert.equal(result.rows[0]?.[0]?.unitType, 'user')

  const team = db.store('team', 'ops')
  db.relate(old, team, 'member_of')
  const joined = db.execute('FIND user AS u JOIN team AS t VIA member_of OUT')
  assert.deepEqual(joined.aliases, ['u', 't'])
  assert.deepEqual(joined.rows.map((r) => r.map((u) => u.id)), [[old, team]])
  assert.equal(codeOf(() => db.execute('NOT A QUERY')), 'INVALID_ARGUMENT')
})

// ---- hybridSearch ----

test('hybridSearch keyword leg ranks matching units of a fulltext type', () => {
  db.setFulltext('note', true)
  const fox = db.store('note', 'fox fox fox jumps')
  db.store('note', 'a dog sleeps')
  const hits = db.hybridSearch({ text: 'fox', limit: 5 })
  assert.deepEqual(hits.map((h) => h.id), [fox])
  assert.deepEqual(hits[0]?.ranks, { keyword: 1 })
  assert.ok(hits[0]!.score > 0)
})

test('hybridSearch vector leg ranks the given vectors by similarity', () => {
  const [near, mid, far] = ['a', 'b', 'c'].map((x) => db.store('note', x)) as [string, string, string]
  const hits = db.hybridSearch({
    vectors: {
      query: [1, 0],
      items: [
        { id: far, vector: [-1, 0] },
        { id: near, vector: [1, 0.01] },
        { id: mid, vector: [1, 1] },
      ],
    },
  })
  assert.deepEqual(hits.map((h) => h.id), [near, mid, far])
  assert.deepEqual(hits.map((h) => h.ranks.vector), [1, 2, 3])

  const dot = db.hybridSearch({
    vectors: { query: [1, 0], metric: 'dot', items: [{ id: near, vector: [1, 0] }, { id: far, vector: [5, 0] }] },
  })
  assert.deepEqual(dot.map((h) => h.id), [far, near])
  assert.equal(codeOf(() => db.hybridSearch({ vectors: { query: [1, 0, 0], items: [{ id: near, vector: [1, 0] }] } })), 'INVALID_ARGUMENT')
})

test('hybridSearch relations leg walks from the seeds, which are not hits', () => {
  const [seed, one, two, other] = ['s', '1', '2', 'o'].map((x) => db.store('note', x)) as [string, string, string, string]
  db.relate(seed, one, 'next')
  db.relate(one, two, 'next')
  db.relate(seed, other, 'other')

  const all = db.hybridSearch({ relations: { seeds: [seed] } })
  assert.deepEqual(all.map((h) => h.id).sort(), [one, two, other].sort())
  assert.ok(all.every((h) => h.ranks.relations !== undefined))

  const typed = db.hybridSearch({ relations: { seeds: [seed], types: ['next'], maxDepth: 1 } })
  assert.deepEqual(typed.map((h) => h.id), [one])
  const inbound = db.hybridSearch({ relations: { seeds: [two], direction: 'in', maxDepth: 1 } })
  assert.deepEqual(inbound.map((h) => h.id), [one])
  assert.deepEqual(db.hybridSearch({ relations: { seeds: [two], direction: 'out' } }), [])
})

test('hybridSearch fuses the legs, honours weights, limit and fusion', () => {
  db.setFulltext('note', true)
  const both = db.store('note', 'fox')
  const onlyVector = db.store('note', 'other')
  const query = {
    text: 'fox',
    vectors: { query: [1, 0], items: [{ id: both, vector: [1, 0] }, { id: onlyVector, vector: [1, 0.2] }] },
  }
  const fused = db.hybridSearch(query)
  assert.equal(fused[0]?.id, both)
  assert.deepEqual(fused[0]?.ranks, { keyword: 1, vector: 1 })
  assert.deepEqual(fused[1]?.ranks, { vector: 2 })
  assert.ok(fused[0]!.score > fused[1]!.score)

  assert.equal(db.hybridSearch({ ...query, limit: 1 }).length, 1)
  // Weight 0 switches the vector leg off.
  const textOnly = db.hybridSearch({ ...query, weights: { vector: 0 } })
  assert.deepEqual(textOnly.map((h) => h.id), [both])
  assert.equal(db.hybridSearch({ ...query, rrfK: 1 })[0]!.score > fused[0]!.score, true)
})

test('hybridSearch graph_gated ranks the units the relations leg found first', () => {
  db.setFulltext('note', true)
  const seed = db.store('topic', 'seed')
  const linked = db.store('note', 'unrelated words')
  const keyword = db.store('note', 'fox')
  db.relate(seed, linked, 'about')
  const query = {
    text: 'fox',
    vectors: { query: [1, 0], items: [{ id: keyword, vector: [1, 0] }] },
    relations: { seeds: [seed] },
  }
  // Fused by rank, the unit two legs agree on leads; gated, the walk's hit leads.
  assert.deepEqual(db.hybridSearch(query).map((h) => h.id), [keyword, linked])
  assert.deepEqual(db.hybridSearch({ ...query, fusion: 'graph_gated' }).map((h) => h.id), [linked, keyword])
  assert.deepEqual(db.hybridSearch({ ...query, fusion: 'rrf' }).map((h) => h.id), [keyword, linked])
})

test('hybridSearch refuses a query with no leg or out-of-range bounds', () => {
  assert.equal(codeOf(() => db.hybridSearch({})), 'INVALID_ARGUMENT')
  assert.equal(codeOf(() => db.hybridSearch({ text: 'x', limit: 0 })), 'INVALID_ARGUMENT')
  assert.equal(codeOf(() => db.hybridSearch({ text: 'x', candidates: 5000 })), 'INVALID_ARGUMENT')
  // @ts-expect-error: not a fusion
  assert.equal(codeOf(() => db.hybridSearch({ text: 'x', fusion: 'max' })), 'INVALID_ARGUMENT')
  assert.equal(codeOf(() => db.hybridSearch({ relations: { seeds: [MISSING] } })), 'INVALID_ARGUMENT')
})

// ---- open ----

test('open with readOnly reads, refuses writes, and does not create a database', () => {
  const id = db.store('note', 'kept')
  db.close()

  db = open(dir, { readOnly: true })
  assert.equal(db.read(id)?.payload?.toString(), 'kept')
  assert.ok(codeOf(() => db.store('note', 'nope')))
  db.close()

  const missing = join(dir, 'does-not-exist')
  assert.ok(codeOf(() => open(missing, { readOnly: true })))
  db = open(dir)
})

test('refresh shows a readOnly handle the commits made since it opened', () => {
  const first = db.store('note', 'one')
  const reader = open(dir, { readOnly: true })
  try {
    assert.ok(reader.read(first))
    const second = db.store('note', 'two')
    assert.equal(reader.read(second), null) // not visible before refresh
    const seq = reader.refresh()
    assert.equal(seq, db.lastCommitSeq())
    assert.equal(reader.lastCommitSeq(), seq)
    assert.equal(reader.read(second)?.payload?.toString(), 'two')
    assert.ok(codeOf(() => db.refresh())) // a read-write handle is always current
  } finally {
    reader.close()
  }
})

test('open with maxBatchOps caps the operations of one commit', () => {
  db.close()
  db = open(dir, { maxBatchOps: 2 })
  const ops = (n: number) => Array.from({ length: n }, () => ({ op: 'store' as const, unitType: 'note', payload: 'x' }))
  assert.equal(db.commit(ops(2)).units.length, 2)
  assert.equal(codeOf(() => db.commit(ops(3))), 'INVALID_ARGUMENT')
})
