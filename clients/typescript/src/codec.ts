/** The JSON forms of the API and their JS values. */

import type { Access, Fields, Hit, Payload, Plan, QueryResult, Relation, Schema, Unit } from './types.ts';

// The answers of our own server are decoded, not validated: a body that does not have the shape
// below is a server bug, and it surfaces as a TypeError at the use.
// ponytail: no runtime schema check of answers; add one if a server other than kerno-server is supported.
type Json = any; // eslint-disable-line @typescript-eslint/no-explicit-any

export function toHex(bytes: Uint8Array): string {
  let out = '';
  for (const b of bytes) out += b.toString(16).padStart(2, '0');
  return out;
}

export function fromHex(text: string): Uint8Array {
  const out = new Uint8Array(text.length / 2);
  for (let i = 0; i < out.length; i++) out[i] = parseInt(text.slice(2 * i, 2 * i + 2), 16);
  return out;
}

function encodeField(value: Fields[string]): Json {
  return value instanceof Uint8Array ? { hex: toHex(value) } : value;
}

function decodeField(value: Json): Fields[string] {
  const isBytes = typeof value === 'object' && value !== null && Object.keys(value).length === 1 && 'hex' in value;
  return isBytes ? fromHex(value.hex) : value;
}

export function encodePayload(payload: Payload): Json {
  if (typeof payload === 'string') return { text: payload };
  if (payload instanceof Uint8Array) return { hex: toHex(payload) };
  if (typeof payload === 'object' && payload !== null) {
    return { fields: Object.fromEntries(Object.entries(payload).map(([k, v]) => [k, encodeField(v)])) };
  }
  throw new TypeError('a payload is a string, a Uint8Array or a record of fields');
}

function decodePayload(payload: Json): Payload | null {
  if (payload === null) return null;
  if ('text' in payload) return payload.text;
  if ('hex' in payload) return fromHex(payload.hex);
  return Object.fromEntries(Object.entries<Json>(payload.fields).map(([k, v]) => [k, decodeField(v)]));
}

export function decodeUnit(u: Json): Unit {
  return {
    id: u.id,
    type: u.type,
    lifecycle: u.lifecycle,
    revision: u.revision,
    createdAtUs: u.created_at_us,
    updatedAtUs: u.updated_at_us,
    payload: decodePayload(u.payload),
  };
}

export function decodeRelation(r: Json): Relation {
  const relation: Relation = { id: r.id, from: r.from, to: r.to, type: r.type, provenance: r.provenance };
  if (r.direction !== undefined) relation.direction = r.direction;
  return relation;
}

export function decodeSchema(s: Json): Schema {
  return { type: s.type, fields: s.fields };
}

export function decodeQueryResult(q: Json): QueryResult {
  return { aliases: q.aliases, rows: q.rows.map((row: Json[]) => row.map(decodeUnit)), access: q.access as Access };
}

export function decodePlan(p: Json): Plan {
  return {
    rootAlias: p.root_alias,
    rootType: p.root_type,
    candidates: p.candidates,
    chosen: p.chosen,
    joins: p.joins.map((j: Json) => ({
      alias: j.alias,
      unitType: j.unit_type,
      relationType: j.relation_type,
      direction: j.direction,
      from: j.from,
    })),
  };
}

export function decodeHit(h: Json): Hit {
  return { id: h.id, score: h.score, ranks: h.ranks, unit: decodeUnit(h.unit) };
}

/** The members of `values` that are not `undefined` (an `undefined` is "use the server's default"; 0 and `false` are values). */
export function defined(values: Record<string, unknown>): Record<string, unknown> {
  return Object.fromEntries(Object.entries(values).filter(([, v]) => v !== undefined));
}
