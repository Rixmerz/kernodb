/** Base class of every error this package throws. */
export class KernoError extends Error {}

/** The server could not be reached, or did not answer in time. */
export class TransportError extends KernoError {}

/**
 * The server answered with an error. `code` is its machine-readable error code (`not_found`,
 * `revision_conflict`, `schema_violation`, ... see the kerno-server README), or `null` when the
 * answer was not a kerno-server error body (a proxy's page, say).
 */
export class ApiError extends KernoError {
  readonly status: number;
  readonly code: string | null;

  constructor(status: number, code: string | null, message: string) {
    super(`${status} ${code ?? 'error'}: ${message}`);
    this.status = status;
    this.code = code;
  }
}
