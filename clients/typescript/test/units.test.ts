import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';

import { ApiError, KernoClient, type Unit } from '../src/index.ts';
import { collect, startServer, stopServer, withServer } from './server.ts';

const packageVersion: string = JSON.parse(readFileSync(new URL('../package.json', import.meta.url), 'utf8')).version;

/** The `ApiError` a call fails with. */
async function refused(call: Promise<unknown>): Promise<ApiError> {
  try {
    await call;
  } catch (e) {
    assert.ok(e instanceof ApiError, `not an ApiError: ${e}`);
    return e;
  }
  assert.fail('the call was not refused');
}

test('health', () =>
  withServer(async (kerno) => {
    assert.deepEqual(await kerno.health(), { status: 'ok', version: packageVersion });
  }));

test('a text unit round-trips', () =>
  withServer(async (kerno) => {
    const created = await kerno.createUnit('note', 'Rust has no GC');
    assert.equal(created.type, 'note');
    assert.equal(created.lifecycle, 'active');
    assert.equal(created.revision, 1);
    assert.equal(created.payload, 'Rust has no GC');
    assert.match(created.id, /^[0-9a-f]{32}$/);
    assert.ok(created.createdAtUs > 0);
    assert.deepEqual(await kerno.getUnit(created.id), created);
  }));

test('bytes round-trip, and bytes that are UTF-8 come back as text', () =>
  withServer(async (kerno) => {
    const raw = new Uint8Array([0, 255, 16]);
    const unit = await kerno.createUnit('blob', raw);
    assert.deepEqual((await kerno.getUnit(unit.id)).payload, raw);
    assert.deepEqual((await kerno.createUnit('blob', Buffer.from([0xff, 0xfe]))).payload, new Uint8Array([0xff, 0xfe]));
    assert.equal((await kerno.createUnit('blob', new TextEncoder().encode('hi'))).payload, 'hi');
  }));

test('a unit is updated into a new revision', () =>
  withServer(async (kerno) => {
    const unit = await kerno.createUnit('note', 'first');
    const updated = await kerno.updateUnit(unit.id, 'second');
    assert.equal(updated.revision, 2);
    assert.equal(updated.payload, 'second');
    assert.deepEqual(await kerno.getUnit(unit.id), updated);
  }));

test('expectedRevision guards every change', () =>
  withServer(async (kerno) => {
    const unit = await kerno.createUnit('note', 'v1');
    const stale = await refused(kerno.updateUnit(unit.id, 'x', { expectedRevision: 7 }));
    assert.equal(stale.status, 409);
    assert.equal(stale.code, 'revision_conflict');
    assert.equal((await kerno.getUnit(unit.id)).revision, 1, 'a refused update changes nothing');

    assert.equal((await kerno.updateUnit(unit.id, 'v2', { expectedRevision: 1 })).revision, 2);
    const changes: ((id: string, options: { expectedRevision: number }) => Promise<unknown>)[] = [
      (id, options) => kerno.archiveUnit(id, options),
      (id, options) => kerno.unarchiveUnit(id, options),
      (id, options) => kerno.deleteUnit(id, options),
    ];
    for (const change of changes) {
      assert.equal((await refused(change(unit.id, { expectedRevision: 1 }))).code, 'revision_conflict');
    }
    assert.equal((await kerno.archiveUnit(unit.id, { expectedRevision: 2 })).revision, 3);
    assert.equal((await kerno.unarchiveUnit(unit.id, { expectedRevision: 3 })).revision, 4);
    await kerno.deleteUnit(unit.id, { expectedRevision: 4 });
  }));

test('archive, unarchive and delete', () =>
  withServer(async (kerno) => {
    const unit = await kerno.createUnit('note', 'keep me');
    assert.equal((await kerno.archiveUnit(unit.id)).lifecycle, 'archived');
    assert.deepEqual(await collect(kerno.iterUnits({ type: 'note' })), []);
    assert.equal((await kerno.unarchiveUnit(unit.id)).lifecycle, 'active');
    assert.equal(await kerno.deleteUnit(unit.id), undefined);
    const tombstone = await kerno.getUnit(unit.id);
    assert.equal(tombstone.lifecycle, 'deleted');
    assert.equal(tombstone.payload, null);
    const update = await refused(kerno.updateUnit(unit.id, 'x'));
    assert.deepEqual([update.status, update.code], [409, 'conflict']);
  }));

test('an unknown unit is not found and a bad id is invalid', () =>
  withServer(async (kerno) => {
    const missing = await refused(kerno.getUnit('00'.repeat(16)));
    assert.deepEqual([missing.status, missing.code], [404, 'not_found']);
    assert.match(missing.message, /not found/);
    const bad = await refused(kerno.getUnit('nope'));
    assert.deepEqual([bad.status, bad.code], [400, 'invalid_request']);
  }));

test('units are listed by type and paged', () =>
  withServer(async (kerno) => {
    const notes: string[] = [];
    for (let i = 0; i < 5; i++) notes.push((await kerno.createUnit('note', `n${i}`)).id);
    notes.sort();
    await kerno.createUnit('task', 't');
    const ids = (units: Unit[]) => units.map((u) => u.id);
    assert.deepEqual(ids((await kerno.listUnits({ type: 'note' })).units), notes);
    assert.equal((await kerno.listUnits()).units.length, 6);

    const first = await kerno.listUnits({ type: 'note', limit: 2 });
    assert.deepEqual([ids(first.units), first.next], [notes.slice(0, 2), notes[1]]);
    const last = await kerno.listUnits({ type: 'note', limit: 2, after: notes[3] });
    assert.deepEqual([ids(last.units), last.next], [notes.slice(4), null]);
    assert.deepEqual(ids(await collect(kerno.iterUnits({ type: 'note', pageSize: 2 }))), notes);
    assert.deepEqual(ids(await collect(kerno.iterUnits({ type: 'note' }))), notes);
    assert.equal((await refused(kerno.listUnits({ limit: 0 }))).code, 'invalid_request');
  }));

test('relations are listed from both ends', () =>
  withServer(async (kerno) => {
    const [a, b, c] = await Promise.all(['a', 'b', 'c'].map((t) => kerno.createUnit('note', t)));
    assert.ok(a && b && c);
    const outgoing = await kerno.relate(b.id, c.id, 'about');
    const incoming = await kerno.relate(a.id, b.id, 'about');
    assert.deepEqual(outgoing, { id: outgoing.id, from: b.id, to: c.id, type: 'about', provenance: 'declared' });

    const pairs = (list: { id: string; direction?: string }[]) => list.map((r) => [r.id, r.direction]);
    assert.deepEqual(pairs(await kerno.relations(b.id, { direction: 'out' })), [[outgoing.id, 'out']]);
    assert.deepEqual(pairs(await kerno.relations(b.id, { direction: 'in' })), [[incoming.id, 'in']]);
    const both = pairs(await kerno.relations(b.id));
    assert.deepEqual(both, [[outgoing.id, 'out'], [incoming.id, 'in']]);
    assert.deepEqual(pairs(await kerno.relations(b.id, { direction: 'both' })), both);
    assert.deepEqual(await kerno.relations(b.id, { type: 'other' }), []);
    await kerno.relate(b.id, c.id, 'cites');
    assert.equal((await kerno.relations(b.id, { direction: 'out', type: 'about' })).length, 1);
  }));

test('a unit with relations cannot be deleted until they are', () =>
  withServer(async (kerno) => {
    const a = await kerno.createUnit('note', 'a');
    const b = await kerno.createUnit('note', 'b');
    const edge = await kerno.relate(a.id, b.id, 'about');
    for (const unit of [a, b]) {
      const error = await refused(kerno.deleteUnit(unit.id));
      assert.deepEqual([error.status, error.code], [409, 'conflict']);
    }
    assert.equal(await kerno.deleteRelation(edge.id), undefined);
    assert.deepEqual(await kerno.relations(a.id), []);
    await kerno.deleteUnit(a.id);
    await kerno.deleteUnit(b.id);
    assert.equal((await refused(kerno.deleteRelation(edge.id))).code, 'not_found');
  }));

test('units survive a graceful stop', () =>
  withServer(async (kerno, server) => {
    const unit = await kerno.createUnit('note', 'durable');
    assert.equal(await stopServer(server), 0);
    const again = await startServer(server.db);
    try {
      assert.deepEqual(await new KernoClient(again.url).getUnit(unit.id), unit);
    } finally {
      assert.equal(await stopServer(again), 0);
    }
  }));

test('the types refuse what the server would', () => {
  const kerno = new KernoClient();
  // These are checked by `tsc --noEmit` (npm test runs it first); the calls are never made.
  const never = (_: () => unknown) => {};
  never(() => {
    // @ts-expect-error a payload is a string, bytes or a record of fields
    void kerno.createUnit('note', 5);
    // @ts-expect-error a direction is out, in or both
    void kerno.relations('id', { direction: 'sideways' });
    // @ts-expect-error a field type is one of five
    void kerno.defineSchema('t', [{ name: 'x', type: 'decimal' }]);
    // @ts-expect-error the metric is cosine or dot
    void kerno.search({ vector: { query: [1], vectors: {}, metric: 'euclid' } });
  });
});
