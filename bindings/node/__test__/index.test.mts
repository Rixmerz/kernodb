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
  assert.deepEqual(done, { units: [], relations: [], unchanged: 0, upserts: [] })
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
  assert.equal(codeOf(() => db.history(MISSING)), 'NOT_FOUND') // never existed: not an empty history
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

test('runRetention prunes old revisions and readAsOf reports PRUNED', () => {
  const note = db.store('note', 'v1')
  const firstSeq = db.lastCommitSeq()
  for (const v of ['v2', 'v3', 'v4']) db.update(note, v)
  const log = db.store('log', 'l1')
  db.update(log, 'l2')
  const before = db.lastCommitSeq()

  // Nothing is pruned before retention runs; a refused rule writes nothing.
  assert.equal(db.readAsOf(note, firstSeq)?.payload?.toString(), 'v1')
  const badRules = [
    {}, { keepLast: null }, { keepLast: 0 }, { keepLast: 1, keepFor: 1 }, { keepFor: -1 }, { keepFor: NaN },
    { keepLast: -2 }, { keepLast: 2 ** 60 }, { keepLast: 1.5 }, { keepLast: NaN }, { keepLast: Infinity }, { keepSince: 1.5 },
    { keepSince: -1 }, { keepLast: '2' }, { keepFor: 'x' }, 3,
  ]
  for (const bad of badRules) {
    assert.equal(codeOf(() => db.runRetention({ byType: { note: bad as never } })), 'INVALID_ARGUMENT', JSON.stringify(bad))
    assert.equal(codeOf(() => db.runRetention({ default: bad as never })), 'INVALID_ARGUMENT', JSON.stringify(bad))
  }
  for (const bad of ['x', 3, { byType: 'x' }]) {
    assert.equal(codeOf(() => db.runRetention(bad as never)), 'INVALID_ARGUMENT', JSON.stringify(bad))
  }
  assert.equal(db.lastCommitSeq(), before)
  // `null` is the same as no rule: nothing to prune.
  for (const none of [null, undefined, {}, { default: null, byType: null }, { byType: { note: null } }]) {
    assert.equal(db.runRetention(none).unitsRetained, 0)
  }
  assert.equal(db.lastCommitSeq(), before)

  // `note` keeps its newest two revisions; `log` has no rule and keeps everything.
  const report = db.runRetention({ byType: { note: { keepLast: 2 } } })
  assert.equal(report.unitsRetained, 1)
  assert.equal(report.revisionsPruned, 2)
  assert.equal(report.commitSeq, db.lastCommitSeq())
  assert.deepEqual(db.history(note).map((r) => r.revision), [4, 3])
  assert.deepEqual(db.history(log).map((r) => r.revision), [2, 1])
  assert.equal(codeOf(() => db.readAsOf(note, firstSeq)), 'PRUNED')
  assert.equal(codeOf(() => db.history(note, { fromRevision: 2 })), 'PRUNED')
  assert.deepEqual(db.history(note, { fromRevision: 3 }).map((r) => r.revision), [4, 3])
  assert.equal(codeOf(() => db.history(note, { fromRevision: 2.5 })), 'INVALID_ARGUMENT')
  assert.equal(db.read(note)?.payload?.toString(), 'v4')
  assert.equal(db.verify().clean, true)

  // Idempotent: a second run commits nothing.
  const seq = db.lastCommitSeq()
  const again = db.runRetention({ byType: { note: { keepLast: 2 } } })
  assert.equal(again.unitsRetained, 0)
  assert.equal(again.commitSeq, undefined)
  assert.equal(db.lastCommitSeq(), seq)

  // A duration rule on every type: 0 seconds keeps only the revision in force now.
  db.runRetention({ default: { keepFor: 0 } })
  assert.deepEqual(db.history(note).map((r) => r.revision), [4])
  assert.deepEqual(db.history(log).map((r) => r.revision), [2])
  assert.equal(db.verify().clean, true)

  // `keepSince` keeps what answers as-of reads from that commit on.
  const memo = db.store('memo', 'm1')
  const mid = db.lastCommitSeq()
  db.update(memo, 'm2')
  db.update(memo, 'm3')
  db.runRetention({ byType: { memo: { keepSince: mid } } })
  assert.deepEqual(db.history(memo).map((r) => r.revision), [3, 2, 1])
  db.runRetention({ byType: { memo: { keepSince: db.lastCommitSeq() } } })
  assert.deepEqual(db.history(memo).map((r) => r.revision), [3])
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

  // Off by default: no unit is keyword-indexed, so a keyword search can never match.
  assert.equal(codeOf(keywords), 'NOT_INDEXED')
  assert.equal(db.fulltext('note'), false)
  db.setFulltext('note', true)
  assert.equal(db.fulltext('note'), true)
  assert.deepEqual(keywords(), [note])
  // A unit stored after switching on is indexed too.
  const later = db.store('note', 'another fox')
  assert.deepEqual(keywords().sort(), [note, later].sort())
  db.setFulltext('note', false)
  assert.equal(codeOf(keywords), 'NOT_INDEXED') // nothing is keyword-indexed again
})

// ---- searchSubstring ----

test('searchSubstring matches like LIKE and follows every write', () => {
  const a = db.store('sym', 'pkg.mod.ParseHeader(self) -> Header')
  const b = db.store('sym', 'abc bcd') // every trigram of "abcd", but not "abcd"
  const c = db.store('sym', Buffer.from([0xff, 0x00, 0x72, 0x61, 0x77])) // \xff \0 "raw"
  assert.equal(db.substring('sym'), false)
  assert.deepEqual(db.searchSubstring('parse'), [a]) // off by default: every type is scanned
  db.setSubstring('sym', true) // indexes the units the type already has
  db.setSubstring('sym', true) // same state: nothing written
  assert.equal(db.substring('sym'), true)
  assert.deepEqual(db.searchSubstring('parseheader'), [a]) // ASCII letters in either case
  assert.deepEqual(db.searchSubstring('abcd'), []) // the payload is checked, not only the trigrams
  assert.deepEqual(db.searchSubstring('bc'), [b])
  assert.deepEqual(db.searchSubstring('a'), [a, b, c].sort()) // a one-byte needle
  assert.deepEqual(db.searchSubstring(Buffer.from([0xff, 0x00, 0x72])), [c]) // bytes, 0x00 included
  assert.deepEqual(db.searchSubstring('-> '), [a])
  assert.equal(db.searchSubstring('a', { limit: 2 }).length, 2)
  db.update(a, 'renamed')
  assert.deepEqual(db.searchSubstring('parse'), [])
  assert.deepEqual(db.searchSubstring('renamed'), [a])
  db.commit([{ op: 'archive', id: b }])
  assert.deepEqual(db.searchSubstring('bcd'), [])
  db.commit([{ op: 'unarchive', id: b }])
  assert.deepEqual(db.searchSubstring('bcd'), [b])
  db.delete(c)
  assert.deepEqual(db.searchSubstring('raw'), [])
  assert.equal(db.verify().clean, true)
  db.close()
  db = open(dir) // the option and the entries hold across opens
  assert.deepEqual(db.searchSubstring('renamed'), [a])
  db.setSubstring('sym', false)
  assert.deepEqual(db.searchSubstring('renamed'), [a]) // no type indexed: scanned
  assert.equal(db.verify().clean, true)
})

test('searchSubstring scans types without the index, and answers [] when it finds nothing', () => {
  const note = db.store('note', 'shared words')
  const memo = db.store('memo', 'shared words')
  const plain = db.store('plain', 'shared words') // not indexed
  db.setSubstring('note', true)
  db.setSubstring('memo', true)
  assert.deepEqual(db.searchSubstring('shared'), [note, memo].sort())
  assert.deepEqual(db.searchSubstring('shared', { types: ['memo'] }), [memo])
  assert.deepEqual(db.searchSubstring('absent'), []) // indexed, nothing matches: a real empty answer
  assert.deepEqual(db.searchSubstring('shared', { types: ['plain'] }), [plain]) // named: scanned
  assert.equal(codeOf(() => db.searchSubstring('shared', { types: ['ghost'] })), 'UNKNOWN_TYPE')
  for (const call of [
    () => db.searchSubstring('shared', { limit: 0 }),
    () => db.searchSubstring('shared', { limit: -1 }),
    () => db.searchSubstring('shared', { limit: 1.5 }),
    () => db.searchSubstring(''),
    () => db.searchSubstring('x'.repeat(1025)),
    () => db.searchSubstring('shared', { types: [] }),
  ]) {
    assert.equal(codeOf(call), 'INVALID_ARGUMENT')
  }
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
  assert.equal(codeOf(() => db.find('user', 'age', 36)), 'NOT_INDEXED')
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

test('reads with a name no unit or relation can have throw', () => {
  const a = db.store('note', 'a')
  const b = db.store('note', 'b')
  db.relate(a, b, 'about')
  assert.deepEqual(db.query({ unitType: 'ghost' }), []) // a valid name nothing holds: a real empty answer
  assert.deepEqual(db.related(a, 'out', 'ghost'), [])
  assert.equal(codeOf(() => db.related(MISSING, 'out')), 'NOT_FOUND') // never existed: not "no relations"
  assert.equal(codeOf(() => db.related(MISSING, 'in', 'about')), 'NOT_FOUND')
  const gone = db.store('note', 'gone')
  db.delete(gone)
  assert.deepEqual(db.related(gone, 'out'), []) // a deleted unit is known
  assert.deepEqual(db.relations('ghost'), [])
  for (const bad of ['', 'x'.repeat(100), 'bad\0name']) {
    assert.equal(codeOf(() => db.related(a, 'out', bad)), 'INVALID_ARGUMENT')
    assert.equal(codeOf(() => db.related(a, 'in', bad)), 'INVALID_ARGUMENT')
    assert.equal(codeOf(() => db.relations(bad)), 'INVALID_ARGUMENT')
  }
  for (const bad of ['', 'x'.repeat(100)]) {
    assert.equal(codeOf(() => db.query({ unitType: bad })), 'INVALID_ARGUMENT')
  }
})

// ---- searchText ----

test('searchText throws when it could never match, and answers [] when it merely finds nothing', () => {
  db.store('note', 'needle in a haystack')
  assert.equal(codeOf(() => db.searchText('needle')), 'NOT_INDEXED') // off by default
  db.setFulltext('note', true)
  assert.equal(db.searchText('needle').length, 1)
  assert.deepEqual(db.searchText('absent'), []) // indexed, nothing matches: a real empty answer
  assert.equal(codeOf(() => db.searchText('needle', { limit: 0 })), 'INVALID_ARGUMENT')
  assert.equal(codeOf(() => db.searchText('needle', { limit: 1.5 })), 'INVALID_ARGUMENT')
  assert.equal(codeOf(() => db.searchText('')), 'INVALID_ARGUMENT')
  assert.equal(codeOf(() => db.searchText(' ,, ')), 'INVALID_ARGUMENT')
  assert.equal(codeOf(() => db.searchText('needle', { types: [] })), 'INVALID_ARGUMENT')
  db.setFulltext('note', false)
  assert.equal(codeOf(() => db.searchText('needle')), 'NOT_INDEXED')
})

test('searchText restricted to unit types', () => {
  db.setFulltext('note', true)
  db.setFulltext('memo', true)
  const note = db.store('note', 'shared words here')
  const memo = db.store('memo', 'shared words there')
  db.store('plain', 'shared words nowhere') // not indexed
  assert.deepEqual(db.searchText('shared').map((h) => h.id).sort(), [note, memo].sort())
  assert.deepEqual(db.searchText('shared', { types: ['note'] }).map((h) => h.id), [note])
  assert.deepEqual(db.searchText('shared', { types: ['memo'] }).map((h) => h.id), [memo])
  const both = db.searchText('shared', { types: ['note', 'memo'] })
  assert.deepEqual(both, db.searchText('shared'))
  // a unit scores the same with or without the restriction
  const only = db.searchText('shared', { types: ['note'] })[0]!
  assert.equal(only.score, db.searchText('shared').find((h) => h.id === note)!.score)
  assert.equal(db.searchText('shared', { limit: 1, types: ['memo'] }).length, 1)
  assert.equal(codeOf(() => db.searchText('shared', { types: ['ghost'] })), 'UNKNOWN_TYPE')
  assert.equal(codeOf(() => db.searchText('shared', { types: ['note', 'ghost'] })), 'UNKNOWN_TYPE')
  assert.equal(codeOf(() => db.searchText('shared', { types: ['plain'] })), 'NOT_INDEXED') // exists, not indexed
  // hybridSearch shares the semantics through textTypes
  assert.deepEqual(db.hybridSearch({ text: 'shared', textTypes: ['note'] }).map((h) => h.id), [note])
  assert.equal(codeOf(() => db.hybridSearch({ text: 'shared', textTypes: ['ghost'] })), 'UNKNOWN_TYPE')
  assert.equal(
    codeOf(() => db.hybridSearch({ vectors: { query: [1], items: [{ id: note, vector: [1] }] }, textTypes: ['note'] })),
    'INVALID_ARGUMENT', // textTypes without a keyword leg
  )
  // a types list that is not an array of strings is INVALID_ARGUMENT, not a raw napi error
  for (const bad of [[1], ['note', null], 'note', 5, {}]) {
    assert.equal(codeOf(() => db.searchText('shared', { types: bad as never })), 'INVALID_ARGUMENT')
    assert.equal(codeOf(() => db.hybridSearch({ text: 'shared', textTypes: bad as never })), 'INVALID_ARGUMENT')
  }
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
  assert.equal(codeOf(() => db.commit(ops(3))), 'CHANGESET_TOO_LARGE')
})

test('a numeric option that is a fraction, a negative or NaN is refused, never truncated or wrapped', () => {
  const a = db.store('note', 'v1')
  db.update(a, 'v2')
  const b = db.store('note', 'x')
  // 2**33 is a whole number: valid for a sequence, too big for a 32-bit option.
  for (const n of [1.5, -1, NaN, Infinity, 2 ** 33]) {
    const wide = n === 2 ** 33
    const why = String(n)
    assert.equal(codeOf(() => db.history(a, { limit: n })), 'INVALID_ARGUMENT', `history limit ${why}`)
    if (!wide) assert.equal(codeOf(() => db.readAsOf(a, n)), 'INVALID_ARGUMENT', `readAsOf ${why}`)
    assert.equal(codeOf(() => db.query({ schemaVersion: n })), 'INVALID_ARGUMENT', `schemaVersion ${why}`)
    assert.equal(codeOf(() => open(`${dir}-never`, { maxBatchOps: n })), 'INVALID_ARGUMENT', `maxBatchOps ${why}`)
    if (!wide) assert.equal(codeOf(() => db.commit([{ op: 'restore', id: a, revision: n }])), 'INVALID_ARGUMENT', `restore ${why}`)
    assert.equal(
      codeOf(() => db.commit([{ op: 'store', unitType: 'n', payload: 'x' }, { op: 'relate', from: 0, to: n, relationType: 'r' }])),
      wide ? 'BAD_ENDPOINT' : 'INVALID_ARGUMENT', // 2**33 is a whole number, just not the position of an op
      `relate end ${why}`,
    )
    for (const key of ['limit', 'rrfK', 'candidates'] as const) {
      assert.equal(codeOf(() => db.hybridSearch({ text: 'x', [key]: n })), 'INVALID_ARGUMENT', `hybrid ${key} ${why}`)
    }
    assert.equal(codeOf(() => db.hybridSearch({ text: 'x', weights: { keyword: n } })), 'INVALID_ARGUMENT', `weight ${why}`)
    assert.equal(
      codeOf(() => db.hybridSearch({ relations: { seeds: [b], maxDepth: n } })),
      'INVALID_ARGUMENT',
      `maxDepth ${why}`,
    )
    assert.equal(
      codeOf(() => db.hybridSearch({ relations: { seeds: [b], visitBudget: n } })),
      'INVALID_ARGUMENT',
      `visitBudget ${why}`,
    )
  }
  // A whole number is still accepted, and nothing was written by the refusals.
  assert.equal(db.history(a, { limit: 1 }).length, 1)
  assert.equal(db.readAsOf(a, db.lastCommitSeq())?.revision, 2)
})

function defineUsers(): void {
  db.defineSchema('user', [
    { name: 'email', type: 'text', unique: true },
    { name: 'name', type: 'text' },
    { name: 'age', type: 'int', indexed: true },
  ])
}

const kinds = (id: string) => db.history(id).map((r) => r.kind).reverse()

test('upsert creates, updates, and leaves an unchanged unit alone', () => {
  defineUsers()
  const made = db.upsert('user', 'email', { email: 'a@x.io', name: 'Ada' })
  assert.equal(made.outcome, 'created')
  assert.deepEqual(db.upsert('user', 'email', { email: 'a@x.io', name: 'Ada' }), { id: made.id, outcome: 'unchanged' })
  assert.deepEqual(kinds(made.id), ['create'])
  assert.deepEqual(db.upsert('user', 'email', { email: 'a@x.io', name: 'Ada L.' }), { id: made.id, outcome: 'updated' })
  assert.deepEqual(kinds(made.id), ['create', 'update'])
  assert.equal(db.readDocument(made.id)?.name, 'Ada L.')
  const other = db.upsert('user', 'email', { email: 'b@x.io', name: 'Bob' })
  assert.notEqual(other.id, made.id)
  assert.equal(other.outcome, 'created')
  assert.equal(db.find('user', 'email', 'a@x.io').length, 1)
})

test('upsert refuses a field that is not unique and a document without the key', () => {
  defineUsers()
  const doc = { email: 'a@x.io', name: 'Ada', age: 36 }
  for (const field of ['age', 'name', 'nope']) {
    assert.equal(codeOf(() => db.upsert('user', field, doc)), 'NOT_UNIQUE', field)
  }
  assert.equal(codeOf(() => db.upsert('bare', 'email', doc)), 'NOT_UNIQUE')
  assert.equal(codeOf(() => db.upsert('user', 'email', { name: 'Ada' })), 'SCHEMA')
  const ops = [
    { op: 'store' as const, unitType: 'note', payload: 'kept out' },
    { op: 'upsert' as const, unitType: 'user', field: 'name', document: doc },
  ]
  assert.equal(codeOf(() => db.commit(ops)), 'NOT_UNIQUE')
  assert.equal(db.query().length, 0)
  assert.equal(codeOf(() => db.commit([{ op: 'upsert', unitType: 'user', field: 'email' }])), 'INVALID_ARGUMENT')
})

test('upsert in a commit is atomic and one key twice is one unit', () => {
  defineUsers()
  const note = db.store('note', 'v1')
  const out = db.commit([
    { op: 'upsert', unitType: 'user', field: 'email', document: { email: 'a@x.io', name: 'first' } },
    { op: 'upsert', unitType: 'user', field: 'email', document: { email: 'b@x.io', name: 'Bob' } },
    { op: 'upsert', unitType: 'user', field: 'email', document: { email: 'a@x.io', name: 'second' } },
    { op: 'update', id: note, payload: 'v2' },
  ])
  const [a, b, again] = out.upserts
  assert.deepEqual(a, again)
  assert.notEqual(a.id, b.id)
  assert.equal(a.outcome, 'created')
  assert.deepEqual(out.units, [])
  assert.equal(out.commitSeq, db.lastCommitSeq())
  assert.equal(db.readDocument(a.id)?.name, 'second')
  assert.deepEqual(kinds(a.id), ['create'])
  assert.equal(db.find('user', 'email', 'a@x.io').length, 1)

  // An op after the upsert fails: the upsert is not visible either.
  const seq = db.lastCommitSeq()
  const bad = [
    { op: 'upsert' as const, unitType: 'user', field: 'email', document: { email: 'c@x.io' } },
    { op: 'update' as const, id: MISSING, payload: 'x' },
  ]
  assert.equal(codeOf(() => db.commit(bad)), 'NOT_FOUND')
  assert.equal(db.find('user', 'email', 'c@x.io').length, 0)
  assert.equal(db.lastCommitSeq(), seq)
})

test('upsert keeps one unit across a close and reopen', () => {
  defineUsers()
  const first = db.upsert('user', 'email', { email: 'a@x.io', name: 'Ada' })
  db.upsert('user', 'email', { email: 'a@x.io', name: 'Ada L.' })
  db.close()
  db = open(dir)
  assert.deepEqual(db.upsert('user', 'email', { email: 'a@x.io', name: 'Ada L.' }), { id: first.id, outcome: 'unchanged' })
  assert.equal(db.find('user', 'email', 'a@x.io').length, 1)
  assert.deepEqual(kinds(first.id), ['create', 'update'])
})

// ---- gaps an outside user found in 1.0.0-alpha.3 ----

test('a relate end is the position of an earlier store or upsert op', () => {
  db.defineSchema('file', [{ name: 'path', type: 'text', required: true, unique: true }])
  db.defineSchema('sym', [{ name: 'qn', type: 'text', required: true, unique: true }])
  // store A, upsert g, store B, relate(0, 1) links A to the upserted unit, not to B.
  const out = db.commit([
    { op: 'store', unitType: 'free', payload: 'A' },
    { op: 'upsert', unitType: 'file', field: 'path', document: { path: 'g.py' } },
    { op: 'store', unitType: 'free', payload: 'B' },
    { op: 'relate', from: 0, to: 1, relationType: 'meant_for_g' },
  ])
  assert.deepEqual(db.related(out.units[0], 'out', 'meant_for_g').map((r) => r.to), [out.upserts[0].id])
  // two typed upserts and a relation between them in one commit.
  const typed = db.commit([
    { op: 'upsert', unitType: 'file', field: 'path', document: { path: 'a.py' } },
    { op: 'upsert', unitType: 'sym', field: 'qn', document: { qn: 'a.f' } },
    { op: 'relate', from: 0, to: 1, relationType: 'defines' },
  ])
  assert.deepEqual(db.related(typed.upserts[0].id, 'out').map((r) => r.to), [typed.upserts[1].id])
})

test('a wrong relate position is BAD_ENDPOINT and writes nothing', () => {
  const unit = db.store('free', 'kept')
  const seq = db.lastCommitSeq()
  const store = { op: 'store' as const, unitType: 'free', payload: 'x' }
  const relate = (from: number, to: number) => ({ op: 'relate' as const, from, to, relationType: 'r' })
  for (const ops of [
    [store, relate(0, 2), store], // a later op
    [store, relate(0, 1)], // the relation itself
    [{ op: 'update' as const, id: unit, payload: 'y' }, store, relate(0, 1)], // an update creates no unit
    [store, relate(0, 0), relate(1, 0)], // nor does a relation
    [store, relate(0, 99)], // past the end
  ]) {
    assert.equal(codeOf(() => db.commit(ops)), 'BAD_ENDPOINT')
    assert.equal(db.lastCommitSeq(), seq)
  }
})

test('compact reports the size at call time', () => {
  db.defineSchema('row', [{ name: 'body', type: 'text' }])
  for (let i = 0; i < 100; i++) db.storeDocument('row', { body: `${i} `.repeat(200) })
  const first = db.compact()
  assert.ok((first.before ?? 0) > 5000, `before ${first.before}`) // was 0: the units were still in memory
  for (let i = 0; i < 1000; i++) db.storeDocument('row', { body: `${i} `.repeat(200) })
  const second = db.compact()
  assert.ok((second.before ?? 0) > 5 * (first.after ?? 0), `before ${second.before}, previous after ${first.after}`)
})

test('the indexed text limit is 254 bytes and the error names it', () => {
  db.defineSchema('lim', [{ name: 'key', type: 'text', required: true, unique: true }])
  db.upsert('lim', 'key', { key: 'a'.repeat(254) })
  assert.throws(() => db.upsert('lim', 'key', { key: 'a'.repeat(255) }), /254 bytes/)
})

test('distinct cases throw distinct codes', () => {
  db.defineSchema('tag', [{ name: 'label', type: 'text' }])
  assert.equal(codeOf(() => db.defineSchema('tag', [{ name: 'label', type: 'text' }, { name: 'x', type: 'int' }])), 'SCHEMA_EXISTS')
  assert.equal(codeOf(() => db.upsert('tag', 'label', { label: 'a' })), 'NOT_UNIQUE')
  assert.equal(codeOf(() => db.find('tag', 'label', 'a')), 'NOT_INDEXED')
  db.close()
  db = open(dir, { maxBatchOps: 2 })
  const ops = Array.from({ length: 3 }, () => ({ op: 'store' as const, unitType: 'note', payload: 'x' }))
  assert.equal(codeOf(() => db.commit(ops)), 'CHANGESET_TOO_LARGE')
  db.store('note', 'kept')
  const reader = open(dir, { readOnly: true })
  try {
    assert.equal(codeOf(() => reader.store('note', 'no')), 'READ_ONLY')
    assert.equal(codeOf(() => reader.commit([{ op: 'store', unitType: 'note', payload: 'x' }])), 'READ_ONLY')
  } finally {
    reader.close()
  }
  assert.equal(codeOf(() => open(`${dir}-missing`, { readOnly: true })), 'DATABASE_NOT_FOUND')
  assert.equal(codeOf(() => db.refresh()), 'INVALID_ARGUMENT') // a handle that is not read-only
})

test('defineSchema with the same fields is idempotent', () => {
  const fields = [
    { name: 'email', type: 'text' as const, required: true, unique: true },
    { name: 'name', type: 'text' as const },
  ]
  db.defineSchema('person', fields)
  const seq = db.lastCommitSeq()
  db.defineSchema('person', fields)
  db.defineSchema('person', [...fields].reverse())
  assert.equal(db.lastCommitSeq(), seq)
  assert.equal(codeOf(() => db.defineSchema('person', [{ name: 'email', type: 'text' }])), 'SCHEMA_EXISTS')
})

test('searches that can never match throw', () => {
  db.defineSchema('lim', [{ name: 'key', type: 'text', required: true, unique: true }])
  db.upsert('lim', 'key', { key: 'a'.repeat(254) })
  assert.equal(db.find('lim', 'key', 'a'.repeat(254)).length, 1)
  assert.equal(codeOf(() => db.find('lim', 'key', 'b'.repeat(300))), 'INVALID_ARGUMENT')
  assert.equal(codeOf(() => db.hybridSearch({ text: 'ada' })), 'NOT_INDEXED')
  db.setFulltext('lim', true)
  assert.deepEqual(db.hybridSearch({ text: 'ada' }), [])
})

test('a field value no document can hold throws INVALID_ARGUMENT, never a raw napi error', () => {
  db.defineSchema('ev', [{ name: 'i', type: 'int' }])
  const bad: unknown[] = [null, 2n ** 64n, 2n ** 63n, -(2n ** 63n) - 1n, [1.5], { a: 1 }, 2 ** 70, NaN]
  for (const value of bad) {
    assert.equal(codeOf(() => db.storeDocument('ev', { i: value as never })), 'INVALID_ARGUMENT', String(value))
  }
  db.storeDocument('ev', { i: 2n ** 63n - 1n })
  db.storeDocument('ev', { i: -(2n ** 63n) })
})

test('a pruned read says which number is a revision and which a commit', () => {
  const id = db.store('cfg', 'v0')
  const first = db.lastCommitSeq()
  for (let i = 1; i < 5; i++) db.update(id, `v${i}`)
  db.runRetention({ default: { keepLast: 2 } })
  assert.throws(() => db.readAsOf(id, first), new RegExp(`commit ${first}.*revision 4`))
})
