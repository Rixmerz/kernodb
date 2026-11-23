'use strict'
// The package entry point (hand written; `binding.js` is the napi-generated loader).
//
// napi throws its own errors for a value of the wrong JavaScript type (`undefined` or `null` for a
// string, a non-object where an object is required, ...): a plain `TypeError`, or an `Error`
// whose `code` is `StringExpected`, `InvalidArg` and so on, before the engine is reached. Every
// call is wrapped here so that such a value reaches the caller as the documented
// `INVALID_ARGUMENT` error instead. An error that already carries a Kerno `code` passes through.
const native = require('./binding.js')

const KERNO_CODES = new Set([
  'NOT_FOUND', 'DELETED', 'LOCKED', 'CORRUPTION', 'INVALID_ARGUMENT', 'SCHEMA_EXISTS', 'NOT_INDEXED', 'UNKNOWN_TYPE', 'NOT_UNIQUE', 'BAD_ENDPOINT',
  'PAYLOAD_TOO_LARGE', 'CHANGESET_TOO_LARGE', 'READ_ONLY', 'DATABASE_NOT_FOUND', 'HAS_RELATIONS', 'PRUNED', 'SCHEMA', 'CLOSED', 'KERNO',
])

function typed(fn) {
  return function (...args) {
    try {
      return fn.apply(this, args)
    } catch (e) {
      if (e instanceof Error && KERNO_CODES.has(e.code)) throw e
      const err = new Error(e instanceof Error ? e.message : String(e), { cause: e })
      err.code = 'INVALID_ARGUMENT'
      throw err
    }
  }
}

for (const name of Object.getOwnPropertyNames(native.Db.prototype)) {
  const d = Object.getOwnPropertyDescriptor(native.Db.prototype, name)
  if (name !== 'constructor' && typeof d.value === 'function') Object.defineProperty(native.Db.prototype, name, { ...d, value: typed(d.value) })
}

module.exports = { ...native, open: typed(native.open) }
