import assert from 'node:assert/strict'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { afterEach, beforeEach, test } from 'node:test'

import { open, type Db } from '../index.js'

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

test('errors are thrown as Error with a message', () => {
  assert.throws(() => db.store('', 'x'), Error)
  assert.throws(() => db.read('nope'), /invalid id/)
  assert.throws(() => db.relate(db.store('a', 'x'), '00'.repeat(16), 'about'), Error)
  // @ts-expect-error: not a direction
  assert.throws(() => db.related('00'.repeat(16), 'sideways'), /invalid direction/)
  assert.throws(() => open(dir), Error, 'a directory opens once')
})

test('after close every call throws, and close is idempotent', () => {
  db.close()
  assert.throws(() => db.query(), /closed/)
  db.close()
})
