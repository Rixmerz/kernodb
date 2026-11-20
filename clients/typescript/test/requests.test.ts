/**
 * What the client sends, and how it reads a server that is not the real one.
 *
 * The tests that talk to the real server are in the other files. These use a stub that records the
 * requests, to check what a real server would hide: which parameters are left out, how a path is
 * escaped, what an answer that is not a kerno-server error becomes.
 */

import assert from 'node:assert/strict';
import { type AddressInfo } from 'node:net';
import { createServer, type Server } from 'node:http';
import { test } from 'node:test';

import { ApiError, KernoClient, TransportError } from '../src/index.ts';
import { encodePayload, fromHex, toHex } from '../src/codec.ts';
import { collect } from './server.ts';

interface Seen {
  method: string;
  path: string;
  body: unknown;
}

interface Answer {
  status: number;
  body: string;
  delayMs?: number;
}

const UNIT = { id: '00'.repeat(16), type: 'note', lifecycle: 'active', revision: 1, created_at_us: 1, updated_at_us: 1, payload: { text: 'x' } };

/** A server that answers from a queue (or, when it is empty, with `fallback`) and remembers what it was asked. */
async function withStub(
  fallback: Answer,
  body: (url: string, seen: Seen[], queue: Answer[]) => Promise<void>,
): Promise<void> {
  const seen: Seen[] = [];
  const queue: Answer[] = [];
  const server: Server = createServer((request, response) => {
    const chunks: Buffer[] = [];
    request.on('data', (c: Buffer) => chunks.push(c));
    request.on('end', () => {
      const text = Buffer.concat(chunks).toString();
      seen.push({ method: request.method!, path: request.url!, body: text === '' ? undefined : JSON.parse(text) });
      const answer = queue.shift() ?? fallback;
      setTimeout(() => response.writeHead(answer.status).end(answer.body), answer.delayMs ?? 0);
    });
  });
  await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve));
  try {
    await body(`http://127.0.0.1:${(server.address() as AddressInfo).port}`, seen, queue);
  } finally {
    server.closeAllConnections();
    await new Promise((resolve) => server.close(resolve));
  }
}

const json = (value: unknown, status = 200): Answer => ({ status, body: JSON.stringify(value) });
const last = (seen: Seen[]) => seen[seen.length - 1]!;

test('a parameter left out is not sent', () =>
  withStub(json({ units: [], next: null }), async (url, seen) => {
    const kerno = new KernoClient(url);
    await kerno.listUnits();
    assert.equal(last(seen).path, '/v1/units');
    await kerno.listUnits({ type: 'a b', limit: 5, after: 'ff' });
    assert.equal(last(seen).path, '/v1/units?type=a+b&limit=5&after=ff');
  }));

test('a revision of zero is still sent', () =>
  withStub(json(UNIT), async (url, seen, queue) => {
    const kerno = new KernoClient(url);
    await kerno.updateUnit('ab', 'x');
    assert.equal(last(seen).path, '/v1/units/ab');
    await kerno.updateUnit('ab', 'x', { expectedRevision: 0 });
    assert.equal(last(seen).path, '/v1/units/ab?expected_revision=0');
    await kerno.archiveUnit('ab', { expectedRevision: 0 });
    assert.equal(last(seen).path, '/v1/units/ab/archive?expected_revision=0');
    await kerno.unarchiveUnit('ab', { expectedRevision: 3 });
    assert.equal(last(seen).path, '/v1/units/ab/unarchive?expected_revision=3');
    queue.push({ status: 204, body: '' });
    assert.equal(await kerno.deleteUnit('ab', { expectedRevision: 0 }), undefined);
    assert.deepEqual([last(seen).method, last(seen).path], ['DELETE', '/v1/units/ab?expected_revision=0']);
  }));

test('iterUnits follows next until it is null', () =>
  withStub(json({ units: [], next: null }), async (url, seen, queue) => {
    queue.push(json({ units: [UNIT], next: 'aa' }), json({ units: [UNIT, UNIT], next: 'bb' }), json({ units: [], next: null }));
    const units = await collect(new KernoClient(url).iterUnits({ type: 'note', pageSize: 2 }));
    assert.equal(units.length, 3);
    assert.deepEqual(seen.map((s) => s.path), [
      '/v1/units?type=note&limit=2',
      '/v1/units?type=note&limit=2&after=aa',
      '/v1/units?type=note&limit=2&after=bb',
    ]);
  }));

test('a path segment is escaped whole', () =>
  withStub(json({ type: 'a/b c', fields: [] }), async (url, seen, queue) => {
    const kerno = new KernoClient(url);
    await kerno.getSchema('a/b c');
    assert.equal(last(seen).path, '/v1/schemas/a%2Fb%20c');
    queue.push(json({ relations: [] }));
    await kerno.relations('ab', { direction: 'out', type: 'x y' });
    assert.equal(last(seen).path, '/v1/units/ab/relations?direction=out&type=x+y');
  }));

test('a unit and a relation are sent with the server names', () =>
  withStub(json({ id: 'r', from: 'a', to: 'b', type: 'about', provenance: 'declared' }, 201), async (url, seen, queue) => {
    const kerno = new KernoClient(url);
    const relation = await kerno.relate('a', 'b', 'about');
    assert.deepEqual(last(seen).body, { from: 'a', to: 'b', type: 'about' });
    assert.deepEqual(relation, { id: 'r', from: 'a', to: 'b', type: 'about', provenance: 'declared' });
    assert.ok(!('direction' in relation), 'a created relation has no direction');
    queue.push(json(UNIT, 201));
    await kerno.createUnit('note', { a: 1, b: new Uint8Array([2]) });
    assert.deepEqual(last(seen).body, { type: 'note', payload: { fields: { a: 1, b: { hex: '02' } } } });
    queue.push(json({ type: 't', fields: [] }, 201));
    await kerno.defineSchema('t', [{ name: 'x', type: 'int', unique: true }]);
    assert.deepEqual(last(seen).body, { type: 't', fields: [{ name: 'x', type: 'int', required: false, unique: true, indexed: false }] });
  }));

test('a search sends only what was given', () =>
  withStub(json({ hits: [] }), async (url, seen) => {
    const kerno = new KernoClient(url);
    assert.deepEqual(await kerno.search({ text: 'rust' }), []);
    assert.deepEqual(last(seen).body, { text: 'rust' });
    await kerno.search({
      text: 't',
      vector: { query: [1], vectors: { u: [1] } },
      graph: { seeds: ['s'], direction: 'out', maxDepth: 3, relationTypes: ['r'], limit: 4, visitBudget: 5 },
      weights: { keyword: 0, graph: 2 },
      rrfK: 0,
      candidates: 5,
      limit: 1,
    });
    assert.deepEqual(last(seen).body, {
      text: 't',
      vector: { query: [1], vectors: { u: [1] } },
      graph: { seeds: ['s'], direction: 'out', relation_types: ['r'], max_depth: 3, limit: 4, visit_budget: 5 },
      weights: { keyword: 0, graph: 2 },
      rrf_k: 0,
      candidates: 5,
      limit: 1,
    });
    await kerno.search({ vector: { query: [1], vectors: {}, metric: 'dot' } });
    assert.deepEqual(last(seen).body, { vector: { query: [1], vectors: {}, metric: 'dot' } });
  }));

test('a server error becomes an ApiError', () =>
  withStub(json({ error: { code: 'not_found', message: 'unit x not found' } }, 404), async (url) => {
    const error = await new KernoClient(url).getUnit('x').catch((e: unknown) => e);
    assert.ok(error instanceof ApiError);
    assert.deepEqual([error.status, error.code], [404, 'not_found']);
    assert.equal(error.message, '404 not_found: unit x not found');
  }));

for (const body of ['<html>Bad Gateway</html>', '', '{"error": 3}', '{"detail": "x"}', '[1]', 'null']) {
  test(`an answer that is not a server error still throws an ApiError: ${JSON.stringify(body)}`, () =>
    withStub({ status: 502, body }, async (url) => {
      const error = await new KernoClient(url).health().catch((e: unknown) => e);
      assert.ok(error instanceof ApiError);
      assert.deepEqual([error.status, error.code], [502, null]);
      assert.ok(error.message.length > 0);
    }));
}

test('an unreachable server is a TransportError', async () => {
  const error = await new KernoClient('http://127.0.0.1:1', { timeoutMs: 2000 }).health().catch((e: unknown) => e);
  assert.ok(error instanceof TransportError);
  assert.match(error.message, /127\.0\.0\.1:1/);
});

test('a slow server times out as a TransportError', () =>
  withStub({ status: 200, body: '{}', delayMs: 1000 }, async (url) => {
    const error = await new KernoClient(url, { timeoutMs: 200 }).health().catch((e: unknown) => e);
    assert.ok(error instanceof TransportError);
  }));

test('the base URL may end with a slash, and fetch can be replaced', async () => {
  const urls: string[] = [];
  const fetcher: typeof fetch = async (input) => {
    urls.push(String(input));
    return new Response(JSON.stringify({ status: 'ok', version: '1' }));
  };
  const kerno = new KernoClient('http://example.test///', { fetch: fetcher });
  assert.equal((await kerno.health()).version, '1');
  assert.deepEqual(urls, ['http://example.test/v1/health']);
});

test('payloads encode as the server expects', async () => {
  assert.deepEqual(encodePayload('hi'), { text: 'hi' });
  assert.deepEqual(encodePayload(new Uint8Array([0, 255])), { hex: '00ff' });
  assert.deepEqual(encodePayload(Buffer.from([1])), { hex: '01' });
  assert.deepEqual(encodePayload({ a: 1, b: new Uint8Array([2]), c: 'x', d: false }), { fields: { a: 1, b: { hex: '02' }, c: 'x', d: false } });
  assert.throws(() => encodePayload(5 as never), TypeError);
  assert.throws(() => encodePayload(null as never), TypeError);
  await assert.rejects(new KernoClient('http://127.0.0.1:1').createUnit('n', 5 as never), TypeError);
  assert.equal(toHex(new Uint8Array([0, 1, 171, 255])), '0001abff');
  assert.deepEqual(fromHex('0001ABff'), new Uint8Array([0, 1, 171, 255]));
  assert.deepEqual(fromHex(''), new Uint8Array());
});

test('a unit decodes each kind of payload', () =>
  withStub(json(UNIT), async (url, _seen, queue) => {
    const kerno = new KernoClient(url);
    const payloadOf = async (payload: unknown) => {
      queue.push(json({ ...UNIT, payload }));
      return (await kerno.getUnit('x')).payload;
    };
    assert.equal(await payloadOf({ text: 'x' }), 'x');
    assert.deepEqual(await payloadOf({ hex: '00ff' }), new Uint8Array([0, 255]));
    assert.deepEqual(await payloadOf({ fields: { a: 1, b: { hex: '02' }, c: { hex: '0', x: 1 }, d: null } }), {
      a: 1,
      b: new Uint8Array([2]),
      c: { hex: '0', x: 1 },
      d: null,
    });
    assert.equal(await payloadOf(null), null);
  }));
