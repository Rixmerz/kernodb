import { decodeHit, decodePlan, decodeQueryResult, decodeRelation, decodeSchema, decodeUnit, defined, encodePayload } from './codec.ts';
import { ApiError, TransportError } from './errors.ts';
import type {
  ChangeOptions,
  ClientOptions,
  FieldDef,
  Health,
  Hit,
  ListOptions,
  Page,
  Payload,
  Plan,
  QueryResult,
  Relation,
  RelationsOptions,
  Schema,
  SearchOptions,
  Unit,
} from './types.ts';

const DEFAULT_TIMEOUT_MS = 30_000;

/** `?a=1&b=2` for the parameters that are set, or nothing. */
function queryString(params: Record<string, string | number | undefined>): string {
  const search = new URLSearchParams();
  for (const [key, value] of Object.entries(params)) if (value !== undefined) search.set(key, String(value));
  const text = search.toString();
  return text === '' ? '' : `?${text}`;
}

/** A path segment: a unit type may hold any character, including `/`. */
const segment = encodeURIComponent;

/**
 * A client of one kerno-server.
 *
 * ```ts
 * const kerno = new KernoClient('http://127.0.0.1:8080');
 * const note = await kerno.createUnit('note', 'Rust has no GC');
 * (await kerno.getUnit(note.id)).payload; // 'Rust has no GC'
 * ```
 *
 * Every method throws an `ApiError` when the server refuses the call and a `TransportError` when it
 * cannot be reached.
 */
export class KernoClient {
  readonly baseUrl: string;
  readonly #timeoutMs: number;
  readonly #fetch: typeof fetch;

  constructor(baseUrl = 'http://127.0.0.1:8080', options: ClientOptions = {}) {
    this.baseUrl = baseUrl.replace(/\/+$/, '');
    this.#timeoutMs = options.timeoutMs ?? DEFAULT_TIMEOUT_MS;
    this.#fetch = options.fetch ?? ((...args) => fetch(...args));
  }

  /** The decoded JSON answer of one call; `null` for an empty answer (204). */
  async #request(method: string, path: string, body?: unknown): Promise<any> { // eslint-disable-line @typescript-eslint/no-explicit-any
    const init: RequestInit = { method, headers: { accept: 'application/json' }, signal: AbortSignal.timeout(this.#timeoutMs) };
    if (body !== undefined) {
      init.body = JSON.stringify(body);
      init.headers = { ...init.headers, 'content-type': 'application/json' };
    }
    let status: number;
    let text: string;
    try {
      const response = await this.#fetch(`${this.baseUrl}/v1${path}`, init);
      status = response.status;
      text = await response.text();
    } catch (e) {
      throw new TransportError(`cannot reach ${this.baseUrl}: ${e instanceof Error ? e.message : String(e)}`, { cause: e });
    }
    if (status >= 400) throw apiError(status, text);
    return text === '' ? null : JSON.parse(text);
  }

  // ------------------------------------------------------------------- health

  async health(): Promise<Health> {
    const h = await this.#request('GET', '/health');
    return { status: h.status, version: h.version };
  }

  // -------------------------------------------------------------------- units

  /**
   * Stores a unit. `payload` is a `string` (text), a `Uint8Array`, or a record of fields for a
   * unit type that has a schema.
   */
  async createUnit(type: string, payload: Payload): Promise<Unit> {
    return decodeUnit(await this.#request('POST', '/units', { type, payload: encodePayload(payload) }));
  }

  /** The unit; a deleted one is returned with `lifecycle: 'deleted'`. */
  async getUnit(id: string): Promise<Unit> {
    return decodeUnit(await this.#request('GET', `/units/${segment(id)}`));
  }

  /** One page of active units in id order. */
  async listUnits(options: ListOptions = {}): Promise<Page> {
    const query = queryString({ type: options.type, limit: options.limit, after: options.after });
    const page = await this.#request('GET', `/units${query}`);
    return { units: page.units.map(decodeUnit), next: page.next };
  }

  /** Every active unit, following the pages. */
  async *iterUnits(options: { type?: string; pageSize?: number } = {}): AsyncGenerator<Unit> {
    let after: string | undefined;
    for (;;) {
      const page = await this.listUnits({ type: options.type, limit: options.pageSize, after });
      yield* page.units;
      if (page.next === null) return;
      after = page.next;
    }
  }

  /**
   * A new revision with `payload`. With `expectedRevision` the update is made only if the unit is
   * at that revision, else an `ApiError` with code `revision_conflict` is thrown.
   */
  async updateUnit(id: string, payload: Payload, options: ChangeOptions = {}): Promise<Unit> {
    const query = queryString({ expected_revision: options.expectedRevision });
    return decodeUnit(await this.#request('PUT', `/units/${segment(id)}${query}`, { payload: encodePayload(payload) }));
  }

  async archiveUnit(id: string, options: ChangeOptions = {}): Promise<Unit> {
    const query = queryString({ expected_revision: options.expectedRevision });
    return decodeUnit(await this.#request('POST', `/units/${segment(id)}/archive${query}`));
  }

  async unarchiveUnit(id: string, options: ChangeOptions = {}): Promise<Unit> {
    const query = queryString({ expected_revision: options.expectedRevision });
    return decodeUnit(await this.#request('POST', `/units/${segment(id)}/unarchive${query}`));
  }

  /** Deletes the unit logically. A unit that still has relations is refused (code `conflict`): delete them first. */
  async deleteUnit(id: string, options: ChangeOptions = {}): Promise<void> {
    await this.#request('DELETE', `/units/${segment(id)}${queryString({ expected_revision: options.expectedRevision })}`);
  }

  // ---------------------------------------------------------------- relations

  async relate(from: string, to: string, type: string): Promise<Relation> {
    return decodeRelation(await this.#request('POST', '/relations', { from, to, type }));
  }

  /** The relations of a unit, from both its ends unless `direction` says one. */
  async relations(unitId: string, options: RelationsOptions = {}): Promise<Relation[]> {
    const query = queryString({ direction: options.direction, type: options.type });
    const list = await this.#request('GET', `/units/${segment(unitId)}/relations${query}`);
    return list.relations.map(decodeRelation);
  }

  async deleteRelation(id: string): Promise<void> {
    await this.#request('DELETE', `/relations/${segment(id)}`);
  }

  // ------------------------------------------------------------ keyword index

  /**
   * Turns the keyword (BM25) index of a unit type on or off and returns the new state. The index
   * is off by default (Kerno 1.0.0-alpha.2): `search` finds text only in the types that have it.
   * The type's units are indexed (or cleared) in the same commit.
   */
  async setFulltext(type: string, enabled: boolean): Promise<boolean> {
    const reply = (await this.#request('PUT', `/types/${segment(type)}/fulltext`, { enabled })) as { enabled: boolean };
    return reply.enabled;
  }

  /** Whether a unit type is keyword-indexed. */
  async getFulltext(type: string): Promise<boolean> {
    const reply = (await this.#request('GET', `/types/${segment(type)}/fulltext`)) as { enabled: boolean };
    return reply.enabled;
  }

  // ------------------------------------------------------------------ schemas

  /** Gives a unit type its schema. A type has one for good. */
  async defineSchema(type: string, fields: FieldDef[]): Promise<Schema> {
    const body = { type, fields: fields.map((f) => ({ required: false, unique: false, indexed: false, ...f })) };
    return decodeSchema(await this.#request('POST', '/schemas', body));
  }

  async getSchema(type: string): Promise<Schema> {
    return decodeSchema(await this.#request('GET', `/schemas/${segment(type)}`));
  }

  // -------------------------------------------------------------------- query

  /** Runs a query of the Kerno query language (`docs/query-language.md`). */
  async query(query: string): Promise<QueryResult> {
    return decodeQueryResult(await this.#request('POST', '/query', { query }));
  }

  /** How `query` would run: its access paths with their costs, the one chosen, its joins. */
  async explain(query: string): Promise<Plan> {
    return decodePlan(await this.#request('POST', '/query/explain', { query }));
  }

  // ------------------------------------------------------------------- search

  /** Hybrid retrieval: keywords (`text`), vectors and relations, fused by reciprocal rank. */
  async search(options: SearchOptions): Promise<Hit[]> {
    const { vector, graph, weights } = options;
    const body = defined({
      text: options.text,
      vector: vector && defined({ query: vector.query, vectors: vector.vectors, metric: vector.metric }),
      graph:
        graph &&
        defined({
          seeds: graph.seeds,
          direction: graph.direction,
          relation_types: graph.relationTypes,
          max_depth: graph.maxDepth,
          limit: graph.limit,
          visit_budget: graph.visitBudget,
        }),
      weights: weights && defined({ keyword: weights.keyword, vector: weights.vector, graph: weights.graph }),
      rrf_k: options.rrfK,
      candidates: options.candidates,
      limit: options.limit,
    });
    const answer = await this.#request('POST', '/search', body);
    return answer.hits.map(decodeHit);
  }
}

/** The error of a refused call. The body is the server's error object, unless something else (a proxy) answered. */
function apiError(status: number, text: string): ApiError {
  try {
    const { error } = JSON.parse(text);
    if (typeof error.code === 'string' && typeof error.message === 'string') return new ApiError(status, error.code, error.message);
  } catch {
    // Not JSON, or not an error object: fall through to the generic error.
  }
  return new ApiError(status, null, text.slice(0, 200) || 'no body');
}
