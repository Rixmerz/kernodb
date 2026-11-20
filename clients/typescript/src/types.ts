/**
 * The values the client takes and returns. A unit's payload is a JS value: a `string` for text, a
 * `Uint8Array` for raw bytes, a `Fields` record for a unit type that has a schema, `null` for a
 * deleted unit. Bytes that happen to be valid UTF-8 are read back as a `string`: the server cannot
 * tell them apart.
 *
 * JavaScript numbers are doubles: an `int` field beyond 2^53 loses precision on the way.
 */

export type FieldValue = boolean | number | string | Uint8Array;
export type Fields = Record<string, FieldValue>;
export type Payload = string | Uint8Array | Fields;

export type Lifecycle = 'active' | 'archived' | 'deleted';

export interface Unit {
  id: string;
  type: string;
  lifecycle: Lifecycle;
  revision: number;
  createdAtUs: number;
  updatedAtUs: number;
  /** `null` for a deleted unit. */
  payload: Payload | null;
}

export interface Page {
  units: Unit[];
  /** Pass it as `after` to get the next page; `null` on the last one. */
  next: string | null;
}

export interface ListOptions {
  type?: string;
  /** 1 to 1000; the server's default is 100. */
  limit?: number;
  after?: string;
}

/** `expectedRevision` makes a change conditional: it is refused (`revision_conflict`) unless the unit is at that revision. */
export interface ChangeOptions {
  expectedRevision?: number;
}

export type Direction = 'out' | 'in' | 'both';

export interface Relation {
  id: string;
  from: string;
  to: string;
  type: string;
  provenance: string;
  /** Only in a list: `out` when the listed unit is `from`, `in` when it is `to`. */
  direction?: 'out' | 'in';
}

export interface RelationsOptions {
  /** `both` when left out. */
  direction?: Direction;
  type?: string;
}

export type FieldType = 'bool' | 'int' | 'float' | 'text' | 'bytes';

export interface FieldDef {
  name: string;
  type: FieldType;
  required?: boolean;
  /** No two active units hold the same value. Implies `indexed`. */
  unique?: boolean;
  indexed?: boolean;
}

export interface Schema {
  type: string;
  fields: Required<FieldDef>[];
}

export type Access = { kind: 'type_scan' } | { kind: 'field_index'; field: string };

export interface QueryResult {
  aliases: string[];
  /** One unit per alias in each row. */
  rows: Unit[][];
  access: Access;
}

export interface Plan {
  rootAlias: string;
  rootType: string;
  candidates: { access: Access; cost: number }[];
  /** Index into `candidates` of the access path chosen. */
  chosen: number;
  joins: { alias: string; unitType: string; relationType: string; direction: 'out' | 'in'; from: string }[];
}

/**
 * The vector leg of a search. The server keeps no vector index, so the request carries the vectors
 * of the units to rank. `metric` is `cosine` when left out.
 */
export interface VectorLeg {
  query: number[];
  vectors: Record<string, number[]>;
  metric?: 'cosine' | 'dot';
}

/** The graph leg: units reached by following relations from `seeds`. A bound left out takes the server's default. */
export interface GraphLeg {
  seeds: string[];
  direction?: Direction;
  relationTypes?: string[];
  maxDepth?: number;
  limit?: number;
  visitBudget?: number;
}

/** How much each leg counts; 0 switches a leg off, a weight left out is 1. */
export interface Weights {
  keyword?: number;
  vector?: number;
  graph?: number;
}

/** At least one of `text`, `vector` and `graph` must be given. */
export interface SearchOptions {
  text?: string;
  vector?: VectorLeg;
  graph?: GraphLeg;
  weights?: Weights;
  rrfK?: number;
  /** Units each leg contributes. */
  candidates?: number;
  /** Hits returned. */
  limit?: number;
}

export interface Hit {
  id: string;
  /** The sum of what each leg earned, in units of 1/1e9 (see `kerno_units::hybrid`). */
  score: number;
  /** The 1-based rank in each leg that found the unit, `null` for the others. */
  ranks: { keyword: number | null; vector: number | null; graph: number | null };
  unit: Unit;
}

export interface Health {
  status: string;
  version: string;
}

export interface ClientOptions {
  /** Per call, in milliseconds. 30 000 when left out. */
  timeoutMs?: number;
  /** The `fetch` to use; the global one when left out. */
  fetch?: typeof fetch;
}
