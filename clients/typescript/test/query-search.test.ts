import assert from 'node:assert/strict';
import { test } from 'node:test';

import { ApiError, type FieldDef, type Fields, type KernoClient, type Unit } from '../src/index.ts';
import { collect, withServer } from './server.ts';

const USER_FIELDS: FieldDef[] = [
  { name: 'email', type: 'text', required: true, unique: true },
  { name: 'age', type: 'int', indexed: true },
  { name: 'score', type: 'float' },
  { name: 'avatar', type: 'bytes' },
  { name: 'admin', type: 'bool' },
];

/** What one rank earns in a leg (the arithmetic of `kerno_units::hybrid`). */
const earns = (weight: number, rrfK: number, rank: number) => Math.floor((weight * 1_000_000_000) / (rrfK + rank));

async function refused(call: Promise<unknown>): Promise<ApiError> {
  try {
    await call;
  } catch (e) {
    assert.ok(e instanceof ApiError, `not an ApiError: ${e}`);
    return e;
  }
  assert.fail('the call was not refused');
}

const user = (kerno: KernoClient, email: string, age: number) => kerno.createUnit('user', { email, age });
const fields = (unit: Unit) => unit.payload as Record<string, unknown>;

test('a schema is defined and read back with its flags', () =>
  withServer(async (kerno) => {
    const schema = await kerno.defineSchema('user', USER_FIELDS);
    const byName = Object.fromEntries(schema.fields.map((f) => [f.name, f]));
    assert.equal(schema.type, 'user');
    assert.deepEqual(byName.email, { name: 'email', type: 'text', required: true, unique: true, indexed: true });
    assert.deepEqual(byName.age, { name: 'age', type: 'int', required: false, unique: false, indexed: true });
    assert.deepEqual(byName.avatar, { name: 'avatar', type: 'bytes', required: false, unique: false, indexed: false });
    assert.deepEqual(byName.score, { name: 'score', type: 'float', required: false, unique: false, indexed: false });
    assert.deepEqual(byName.admin, { name: 'admin', type: 'bool', required: false, unique: false, indexed: false });
    assert.deepEqual(Object.keys(byName).sort(), ['admin', 'age', 'avatar', 'email', 'score']);
    assert.deepEqual(await kerno.getSchema('user'), schema);
  }));

test('schema errors carry their codes', () =>
  withServer(async (kerno) => {
    await kerno.defineSchema('user', USER_FIELDS);
    const again = await refused(kerno.defineSchema('user', [{ name: 'x', type: 'int' }]));
    assert.deepEqual([again.status, again.code], [400, 'invalid_request']);
    const missing = await refused(kerno.getSchema('nothing'));
    assert.deepEqual([missing.status, missing.code], [404, 'not_found']);
    // A unit type is any text: it is escaped in the path.
    await kerno.defineSchema('my type/1', [{ name: 'x', type: 'int' }]);
    assert.equal((await kerno.getSchema('my type/1')).type, 'my type/1');
  }));

test('a typed unit holds fields of every type', () =>
  withServer(async (kerno) => {
    await kerno.defineSchema('user', USER_FIELDS);
    const record = { email: 'ada@x', age: 36, score: 4.5, avatar: new Uint8Array([0, 255]), admin: true };
    const created = await kerno.createUnit('user', record);
    assert.deepEqual(created.payload, record);
    assert.deepEqual((await kerno.getUnit(created.id)).payload, record);
    assert.ok(fields(created).avatar instanceof Uint8Array);
    assert.deepEqual((await kerno.listUnits({ type: 'user' })).units[0]?.payload, record);
  }));

test('an integer written for a float field is a float', () =>
  withServer(async (kerno) => {
    await kerno.defineSchema('user', USER_FIELDS);
    // JavaScript cannot write 5.0: JSON carries 5, and the schema says the field is a float.
    const unit = await kerno.createUnit('user', { email: 'a@x', score: 5 });
    assert.equal(fields(unit).score, 5);
    const refusedFloat = await refused(kerno.createUnit('user', { email: 'b@x', age: 5.5 }));
    assert.equal(refusedFloat.code, 'schema_violation');
  }));

test('the schema refuses what does not fit', () =>
  withServer(async (kerno) => {
    await kerno.defineSchema('user', USER_FIELDS);
    await kerno.createUnit('user', { email: 'ada@x' });
    const bad: Fields[] = [{ age: 3 }, { email: 'b@x', age: 'old' }, { email: 'b@x', zzz: 1 }, { email: 'ada@x' }];
    for (const fieldsOf of bad) {
      const e = await refused(kerno.createUnit('user', fieldsOf));
      assert.deepEqual([e.status, e.code], [422, 'schema_violation'], JSON.stringify(fieldsOf));
    }
    assert.equal((await collect(kerno.iterUnits({ type: 'user' }))).length, 1);
    const free = await refused(kerno.createUnit('free', { a: 1 }));
    assert.deepEqual([free.status, free.code], [400, 'invalid_request']);
  }));

test('a query filters, orders and pages', () =>
  withServer(async (kerno) => {
    await kerno.defineSchema('user', USER_FIELDS);
    for (const [email, age] of [['a@x', 20], ['b@x', 30], ['c@x', 40], ['d@x', 50]] as const) await user(kerno, email, age);
    const ages = async (q: string) => {
      const result = await kerno.query(q);
      assert.deepEqual(result.aliases, ['u']);
      return result.rows.map((row) => fields(row[0]!).age);
    };
    assert.deepEqual(await ages('FIND user AS u ORDER BY u.age DESC'), [50, 40, 30, 20]);
    assert.deepEqual(await ages('FIND user AS u WHERE u.age >= 30 AND u.age < 50 ORDER BY u.age'), [30, 40]);
    assert.deepEqual(await ages('FIND user AS u ORDER BY u.age LIMIT 2 OFFSET 1'), [30, 40]);
    assert.deepEqual(await ages('FIND user AS u WHERE u.email = "zz"'), []);
  }));

test('a query joins through relations', () =>
  withServer(async (kerno) => {
    await kerno.defineSchema('user', USER_FIELDS);
    await kerno.defineSchema('team', [{ name: 'name', type: 'text', unique: true }]);
    const core = await kerno.createUnit('team', { name: 'core' });
    const ada = await user(kerno, 'ada@x', 36);
    const bob = await user(kerno, 'bob@x', 29);
    await kerno.relate(ada.id, core.id, 'member_of');

    const result = await kerno.query('FIND user AS u JOIN team AS t VIA member_of WHERE t.name = "core"');
    assert.deepEqual(result.aliases, ['u', 't']);
    assert.deepEqual(result.rows.map((row) => row.map((u) => u.id)), [[ada.id, core.id]]);
    assert.deepEqual(result.rows[0]?.[1]?.payload, { name: 'core' });
    assert.ok(!result.rows.flat().some((u) => u.id === bob.id));
    assert.equal((await kerno.query('FIND team AS t JOIN user AS u VIA member_of IN')).rows.length, 1);
  }));

test('a query reports the access path it used', () =>
  withServer(async (kerno) => {
    await kerno.defineSchema('user', USER_FIELDS);
    for (let i = 0; i < 8; i++) await user(kerno, `u${i}@x`, 20 + 3 * i);
    assert.deepEqual((await kerno.query('FIND user AS u WHERE u.age = 29')).access, { kind: 'field_index', field: 'age' });
    assert.deepEqual((await kerno.query('FIND user AS u WHERE u.score IS NULL')).access, { kind: 'type_scan' });
  }));

test('explain lists the candidates and the chosen one', () =>
  withServer(async (kerno) => {
    await kerno.defineSchema('user', USER_FIELDS);
    await kerno.defineSchema('team', [{ name: 'name', type: 'text' }]);
    for (let i = 0; i < 8; i++) await user(kerno, `u${i}@x`, 20 + 3 * i);
    const plan = await kerno.explain('FIND user AS u WHERE u.age = 29');
    assert.deepEqual([plan.rootAlias, plan.rootType, plan.chosen, plan.joins], ['u', 'user', 1, []]);
    assert.deepEqual(plan.candidates.map((c) => c.access), [{ kind: 'type_scan' }, { kind: 'field_index', field: 'age' }]);
    assert.ok(plan.candidates[1]!.cost < plan.candidates[0]!.cost);

    const joined = await kerno.explain('FIND user AS u JOIN team AS t VIA member_of JOIN user AS v VIA member_of IN FROM t');
    assert.deepEqual(joined.joins, [
      { alias: 't', unitType: 'team', relationType: 'member_of', direction: 'out', from: 'u' },
      { alias: 'v', unitType: 'user', relationType: 'member_of', direction: 'in', from: 't' },
    ]);
  }));

test('a bad query says why', () =>
  withServer(async (kerno) => {
    await kerno.defineSchema('user', USER_FIELDS);
    const methods: ((query: string) => Promise<unknown>)[] = [(q) => kerno.query(q), (q) => kerno.explain(q)];
    for (const method of methods) {
      const parse = await refused(method('FIND'));
      assert.deepEqual([parse.status, parse.code], [400, 'invalid_request']);
      assert.match(parse.message, /byte 4/);
      assert.match((await refused(method('FIND user WHERE nope = 1'))).message, /declares no such field/);
    }
  }));

test('the keyword leg ranks by text', () =>
  withServer(async (kerno) => {
    const rust = await kerno.createUnit('note', 'rust rust rust ownership');
    const mixed = await kerno.createUnit('note', 'rust and python');
    await kerno.createUnit('note', 'python only');
    const hits = await kerno.search({ text: 'rust' });
    assert.deepEqual(hits.map((h) => h.id), [rust.id, mixed.id]);
    assert.deepEqual(hits[0]?.ranks, { keyword: 1, vector: null, graph: null });
    assert.equal(hits[0]?.score, earns(1, 60, 1));
    assert.equal(hits[1]?.score, earns(1, 60, 2));
    assert.deepEqual(hits[0]?.unit, rust);
  }));

test('a search returns ten hits unless asked for more', () =>
  withServer(async (kerno) => {
    for (let i = 0; i < 12; i++) await kerno.createUnit('note', `common word ${i}`);
    assert.equal((await kerno.search({ text: 'common' })).length, 10);
    assert.equal((await kerno.search({ text: 'common', limit: 3 })).length, 3);
    assert.equal((await kerno.search({ text: 'common', limit: 50 })).length, 12);
  }));

test('the graph leg follows relations from its seeds', () =>
  withServer(async (kerno) => {
    const [a, b, c, d] = [await kerno.createUnit('note', 'a'), await kerno.createUnit('note', 'b'), await kerno.createUnit('note', 'c'), await kerno.createUnit('note', 'd')];
    await kerno.relate(a.id, b.id, 'next');
    await kerno.relate(b.id, c.id, 'next');
    await kerno.relate(c.id, d.id, 'next');
    const reach = async (graph: Parameters<KernoClient['search']>[0]['graph']) => (await kerno.search({ graph })).map((h) => h.id);
    assert.deepEqual(await reach({ seeds: [a.id] }), [b.id, c.id]);
    assert.deepEqual(await reach({ seeds: [a.id], maxDepth: 3 }), [b.id, c.id, d.id]);
    // Both directions is the default: one hop from `b` reaches `a` and `c`, two reach `d` as well.
    assert.deepEqual((await reach({ seeds: [b.id], maxDepth: 1 })).sort(), [a.id, c.id].sort());
    assert.deepEqual((await reach({ seeds: [b.id] })).sort(), [a.id, c.id, d.id].sort());
    assert.deepEqual(await reach({ seeds: [b.id], direction: 'out' }), [c.id, d.id]);
    assert.deepEqual(await reach({ seeds: [b.id], direction: 'in' }), [a.id]);
    assert.deepEqual(await reach({ seeds: [b.id], relationTypes: ['other'] }), []);
    assert.equal((await kerno.search({ graph: { seeds: [a.id] } }))[0]?.ranks.graph, 1);
  }));

test('the vector leg ranks the vectors it carries', () =>
  withServer(async (kerno) => {
    const y = await kerno.createUnit('note', 'y');
    const z = await kerno.createUnit('note', 'z');
    // `y` is long and 45 degrees off the query, `z` is short and exactly on it.
    const vectors = { [y.id]: [5, 5], [z.id]: [0.9, 0] };
    const ids = async (metric?: 'cosine' | 'dot') => (await kerno.search({ vector: { query: [1, 0], vectors, metric } })).map((h) => h.id);
    assert.deepEqual(await ids(), [z.id, y.id]);
    assert.deepEqual(await ids('cosine'), [z.id, y.id]);
    assert.deepEqual(await ids('dot'), [y.id, z.id]);
    assert.equal((await kerno.search({ vector: { query: [1, 0], vectors } }))[0]?.ranks.vector, 1);
  }));

test('the legs are fused and weighted', () =>
  withServer(async (kerno) => {
    const both = await kerno.createUnit('note', 'rust memory');
    const keywordOnly = await kerno.createUnit('note', 'rust');
    const seed = await kerno.createUnit('note', 'seed');
    await kerno.relate(seed.id, both.id, 'about');
    const graph = { seeds: [seed.id] };

    const fused = await kerno.search({ text: 'rust', graph });
    assert.equal(fused[0]?.id, both.id);
    const ranks = fused[0]!.ranks;
    assert.equal(fused[0]?.score, earns(1, 60, ranks.keyword!) + earns(1, 60, ranks.graph!));
    const found = fused.map((h) => h.id);
    assert.ok(found.includes(keywordOnly.id) && !found.includes(seed.id));

    const graphOnly = await kerno.search({ text: 'rust', graph, weights: { keyword: 0 } });
    assert.deepEqual(graphOnly.map((h) => h.id), [both.id]);
    const heavy = await kerno.search({ graph, weights: { graph: 3 }, rrfK: 0, limit: 1 });
    assert.equal(heavy[0]?.score, earns(3, 0, 1));
    const partial = await kerno.search({ text: 'rust', graph, weights: { graph: 2 } });
    assert.equal(partial[0]?.score, earns(1, 60, partial[0]!.ranks.keyword!) + earns(2, 60, partial[0]!.ranks.graph!));
  }));

test('candidates bounds what each leg contributes', () =>
  withServer(async (kerno) => {
    for (let i = 0; i < 101; i++) await kerno.createUnit('note', `bulk ${i}`);
    assert.equal((await kerno.search({ text: 'bulk', limit: 1000 })).length, 100);
    assert.equal((await kerno.search({ text: 'bulk', limit: 1000, candidates: 7 })).length, 7);
  }));

test('a search that cannot run is refused', () =>
  withServer(async (kerno) => {
    const unit = await kerno.createUnit('note', 'a');
    const cases = [
      {},
      { text: 'x', weights: { keyword: 0 } },
      { text: 'x', limit: 0 },
      { graph: { seeds: [] } },
      { graph: { seeds: ['nope'] } },
      { vector: { query: [1, 0], vectors: { [unit.id]: [1] } } },
    ];
    for (const options of cases) {
      const e = await refused(kerno.search(options));
      assert.deepEqual([e.status, e.code], [400, 'invalid_request'], JSON.stringify(options));
    }
  }));
