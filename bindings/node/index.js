'use strict'
// The package entry point (hand written; `binding.js` is the napi-generated loader).
//
// Two things happen here before and after the native call.
//
// 1. Arguments are checked against the shapes below, so a value of the wrong JavaScript type
//    (`undefined` for a string, `types` that is not an array, ...) and an option the call does not
//    know both reach the caller as `INVALID_ARGUMENT` with a message in this package's names. napi
//    alone drops unknown keys silently and words the other failures in Rust terms.
// 2. An error that still escapes from napi's own argument conversion (a shape the table does not
//    cover) is `INVALID_ARGUMENT` too. Any other error that carries no Kerno `code` is not the
//    caller's fault: it is `INTERNAL`, never `INVALID_ARGUMENT`. An error with a Kerno `code`
//    passes through.
const native = require('./binding.js')

const KERNO_CODES = new Set([
  'NOT_FOUND', 'DELETED', 'LOCKED', 'CORRUPTION', 'INVALID_ARGUMENT', 'SCHEMA_EXISTS', 'NOT_INDEXED', 'UNKNOWN_TYPE', 'NOT_UNIQUE', 'BAD_ENDPOINT',
  'PAYLOAD_TOO_LARGE', 'CHANGESET_TOO_LARGE', 'READ_ONLY', 'DATABASE_NOT_FOUND', 'HAS_RELATIONS', 'PRUNED', 'SCHEMA', 'CLOSED',
  'HISTORY_UNAVAILABLE', 'KERNO',
])

// The `code` napi gives an error of its own argument conversion.
const NAPI_ARGUMENT_CODES = new Set([
  'InvalidArg', 'StringExpected', 'NumberExpected', 'BooleanExpected', 'BigintExpected', 'ObjectExpected', 'ArrayExpected',
  'FunctionExpected', 'DateExpected', 'ArrayBufferExpected', 'NameExpected', 'SymbolExpected',
])

// ---- argument shapes -----------------------------------------------------------------------
// A kind is one of the strings below or a shape built by `obj`, `arr` and `map`; a trailing `!`
// makes a value required. 'n' and 'x' are checked by the native side (numbers, which it refuses
// with its own message, and values it types itself).
//   s string  b boolean  f number  p string or Buffer  d object of document fields
const obj = (fields) => ({ t: 'obj', fields })
const arr = (of) => ({ t: 'arr', of })
const map = (of) => ({ t: 'map', of })
const req = (kind) => ({ ...(typeof kind === 'string' ? { t: 'str', kind } : kind), required: true })
const needed = (kind) => (typeof kind === 'string' ? (kind.endsWith('!') ? kind : `${kind}!`) : req(kind))

const RULE = obj({ keepLast: 'x', keepFor: 'x', keepSince: 'x' })
const SHAPES = {
  open: [['path', 's!'], ['options', obj({ readOnly: 'b', maxBatchOps: 'n' })]],
  store: [['unitType', 's!'], ['payload', 'p!']],
  read: [['id', 's!']],
  update: [['id', 's!'], ['payload', 'p!'], ['options', obj({ ifChanged: 'b' })]],
  delete: [['id', 's!'], ['options', obj({ cascade: 'b' })]],
  relate: [['from', 's!'], ['to', 's!'], ['relationType', 's!'], ['options', obj({ unique: 'b' })]],
  unrelate: [['relationId', 's!']],
  related: [['id', 's!'], ['direction', 's!'], ['relationType', 's']],
  relations: [['relationType', 's']],
  query: [['filter', obj({ unitType: 's', schemaVersion: 'n', lifecycle: 's' })]],
  commit: [
    [
      'ops',
      req(
        arr(
          obj({
            op: 's!', unitType: 's', payload: 'p', id: 's', ifChanged: 'b', cascade: 'b', revision: 'n', from: 'x', to: 'x',
            relationType: 's', unique: 'b', relationId: 's', field: 's', document: 'd',
          }),
        ),
      ),
    ],
    ['options', obj({ reason: 's', actor: 'p', source: 'p' })],
  ],
  history: [['id', 's!'], ['options', obj({ limit: 'n', withPayload: 'b', fromRevision: 'n', toRevision: 'n', order: 's' })]],
  readAsOf: [['id', 's!']],
  runRetention: [['options', obj({ default: RULE, byType: map(RULE) })]],
  setFulltext: [['unitType', 's!'], ['enabled', 'b!']],
  searchText: [['query', 's!'], ['options', obj({ limit: 'n', types: arr('s') })]],
  fulltext: [['unitType', 's!']],
  setSubstring: [['unitType', 's!'], ['enabled', 'b!']],
  substring: [['unitType', 's!']],
  searchSubstring: [['needle', 'p!'], ['options', obj({ limit: 'n', types: arr('s') })]],
  defineSchema: [
    ['unitType', 's!'],
    ['fields', req(arr(obj({ name: 's!', type: 's!', required: 'b', unique: 'b', indexed: 'b' })))],
  ],
  createIndex: [['unitType', 's!'], ['field', 's!']],
  storeDocument: [['unitType', 's!'], ['document', 'd!']],
  upsert: [['unitType', 's!'], ['field', 's!'], ['document', 'd!']],
  updateDocument: [['id', 's!'], ['document', 'd!']],
  readDocument: [['id', 's!']],
  find: [['unitType', 's!'], ['field', 's!']],
  execute: [['query', 's!']],
  hybridSearch: [
    [
      'query',
      req(
        obj({
          text: 's',
          textTypes: arr('s'),
          vectors: obj({ query: req(arr('f')), items: arr(obj({ id: 's!', vector: req(arr('f')) })), metric: 's' }),
          relations: obj({ seeds: req(arr('s')), types: arr('s'), direction: 's', maxDepth: 'n', visitBudget: 'n' }),
          weights: obj({ keyword: 'n', vector: 'n', relations: 'n' }),
          fusion: 's',
          rrfK: 'n',
          candidates: 'n',
          limit: 'n',
        }),
      ),
    ],
  ],
}

const KIND_NAMES = {
  s: 'a string', b: 'a boolean', f: 'a number', p: 'a string or a Buffer', d: 'an object of document fields', obj: 'an object',
  arr: 'an array', map: 'an object',
}

const isObject = (v) => typeof v === 'object' && v !== null && !Array.isArray(v)
function describe(v) {
  if (v === null) return 'null'
  if (Array.isArray(v)) return 'an array'
  return typeof v === 'object' ? 'an object' : typeof v
}

/** Throws `INVALID_ARGUMENT` when `value` is not of `kind`; `path` names it for the caller. */
function check(path, kind, value) {
  let required = false
  if (typeof kind === 'string' && kind.endsWith('!')) [kind, required] = [kind.slice(0, -1), true]
  else if (typeof kind === 'object') ({ required = false } = kind)
  if (value === undefined || value === null) {
    if (required) throw argumentError(`${path} is required`)
    return
  }
  const tag = typeof kind === 'string' ? kind : kind.t === 'str' ? kind.kind : kind.t
  const wrong = () => argumentError(`${path} must be ${KIND_NAMES[tag]}, not ${describe(value)}`)
  switch (tag) {
    case 'n': case 'x': return
    case 's': if (typeof value !== 'string') throw wrong(); return
    case 'b': if (typeof value !== 'boolean') throw wrong(); return
    case 'f': if (typeof value !== 'number') throw wrong(); return
    case 'p': if (typeof value !== 'string' && !(value instanceof Uint8Array)) throw wrong(); return
    case 'd': if (!isObject(value)) throw wrong(); return
    case 'arr':
      if (!Array.isArray(value)) throw wrong()
      value.forEach((item, i) => check(`${path}[${i}]`, needed(kind.of), item))
      return
    case 'map':
      if (!isObject(value)) throw wrong()
      for (const [key, item] of Object.entries(value)) check(`${path}.${key}`, kind.of, item)
      return
    case 'obj': {
      if (!isObject(value)) throw wrong()
      for (const key of Object.keys(value)) {
        if (!Object.hasOwn(kind.fields, key)) throw argumentError(`${path}: unknown option "${key}" (known: ${Object.keys(kind.fields).join(', ')})`)
      }
      for (const [key, sub] of Object.entries(kind.fields)) check(`${path}.${key}`, sub, value[key])
      return
    }
  }
}

function argumentError(message, cause) {
  const err = new Error(message, cause === undefined ? undefined : { cause })
  err.code = 'INVALID_ARGUMENT'
  return err
}

function validate(method, args) {
  const shape = SHAPES[method]
  if (!shape) return
  shape.forEach(([name, kind], i) => {
    // The options of a call are named `options` in the message, the other arguments by their own name.
    check(name === 'options' ? `${method} options` : `${method}: ${name}`, kind, args[i])
  })
}

function typed(method, fn) {
  return function (...args) {
    try {
      validate(method, args)
      return fn.apply(this, args)
    } catch (e) {
      if (e instanceof Error && KERNO_CODES.has(e.code)) throw e
      // napi's own argument conversion (a wrong type the table above does not cover): the caller's
      // fault. It sets one of its codes, or throws a plain TypeError.
      if (e instanceof Error && (NAPI_ARGUMENT_CODES.has(e.code) || (e instanceof TypeError && e.code === undefined))) {
        throw argumentError(`${method}: an argument has the wrong type`, e)
      }
      const err = new Error(e instanceof Error ? e.message : String(e), { cause: e })
      err.code = 'INTERNAL'
      throw err
    }
  }
}

for (const name of Object.getOwnPropertyNames(native.Db.prototype)) {
  const d = Object.getOwnPropertyDescriptor(native.Db.prototype, name)
  if (name !== 'constructor' && typeof d.value === 'function') Object.defineProperty(native.Db.prototype, name, { ...d, value: typed(name, d.value) })
}

module.exports = { ...native, open: typed('open', native.open) }
