/**
 * Single normalized error shape for every API failure.
 *
 * Views should never have to distinguish a dropped TCP connection from a 500
 * from a malformed JSON body — they get one {@link ApiError} with a coarse
 * {@link ApiErrorKind} they can branch on and a human-readable `message` they
 * can show. Everything the HTTP layer knows (status, backend error code,
 * retriability) is captured here so failures are handled uniformly.
 */

/** Coarse classification of an API failure, stable enough for view logic. */
export type ApiErrorKind =
  /** The request never got a response (DNS, TCP, CORS, offline). */
  | 'network'
  /** The client aborted the request (timeout or caller-supplied signal). */
  | 'timeout'
  /** A response arrived with a non-2xx status. `status` is populated. */
  | 'http'
  /** A 2xx response arrived but its body could not be parsed as expected. */
  | 'parse'
  /** The client refused to issue the call (e.g. a disabled capability). */
  | 'disabled'

/** JSON error envelope the backend is expected to return on failures. */
export interface ApiErrorBody {
  readonly code?: string
  readonly message?: string
  readonly details?: unknown
  /** Some services nest the payload under `error` — both shapes are accepted. */
  readonly error?: { code?: string; message?: string; details?: unknown }
}

export interface ApiErrorInit {
  readonly kind: ApiErrorKind
  readonly message: string
  readonly status?: number | null
  readonly code?: string | null
  readonly details?: unknown
  readonly retriable?: boolean
  readonly cause?: unknown
}

/**
 * The one error type the client throws. Instances are plain `Error` subclasses
 * so existing `catch`/`instanceof Error` paths keep working, but carry the
 * structured fields views need.
 */
export class ApiError extends Error {
  readonly kind: ApiErrorKind
  /** HTTP status, or `null` for transport-level failures. */
  readonly status: number | null
  /** Machine-readable backend error code, when the body supplied one. */
  readonly code: string | null
  /** Any structured detail the backend attached to the error. */
  readonly details: unknown
  /** Whether retrying the same call could plausibly succeed. */
  readonly retriable: boolean

  constructor(init: ApiErrorInit) {
    super(init.message)
    // `Error`'s `cause` option is ES2022; the project targets the ES2020 lib, so
    // attach it directly rather than via the (untyped-here) constructor option.
    if (init.cause !== undefined) {
      ;(this as { cause?: unknown }).cause = init.cause
    }
    this.name = 'ApiError'
    this.kind = init.kind
    this.status = init.status ?? null
    this.code = init.code ?? null
    this.details = init.details
    this.retriable = init.retriable ?? defaultRetriable(init.kind, init.status ?? null)
  }
}

/** Whether a request that failed with `res` (or threw) is worth retrying. */
export function defaultRetriable(kind: ApiErrorKind, status: number | null): boolean {
  if (kind === 'network' || kind === 'timeout') return true
  if (kind === 'http' && status !== null) {
    // 408 Request Timeout and 429 Too Many Requests are transient; so is any 5xx
    // — except 501 Not Implemented, which will never start working on retry.
    if (status === 408 || status === 429) return true
    return status >= 500 && status !== 501
  }
  return false
}

/** Extract `{ code, message, details }` from either accepted envelope shape. */
function unwrapErrorBody(body: ApiErrorBody | null): {
  code: string | null
  message: string | null
  details: unknown
} {
  const src = body?.error ?? body ?? {}
  return {
    code: src.code ?? null,
    message: src.message ?? null,
    details: src.details,
  }
}

/**
 * Normalize a non-2xx {@link Response} into an {@link ApiError}, reading the
 * backend error envelope when present. Never throws: a body that fails to parse
 * degrades to the bare status line.
 */
export async function errorFromResponse(res: Response): Promise<ApiError> {
  let body: ApiErrorBody | null = null
  try {
    const text = await res.text()
    if (text) body = JSON.parse(text) as ApiErrorBody
  } catch {
    // Non-JSON or empty error body — fall back to the status line below.
  }
  const { code, message, details } = unwrapErrorBody(body)
  return new ApiError({
    kind: 'http',
    status: res.status,
    code,
    details,
    message: message ?? `${res.status} ${res.statusText || 'request failed'}`.trim(),
  })
}

/**
 * Normalize a thrown value from `fetch`/parsing into an {@link ApiError}.
 * A `DOMException` named `AbortError` maps to `timeout`; anything else from the
 * transport maps to `network`.
 */
export function errorFromThrown(err: unknown): ApiError {
  if (err instanceof ApiError) return err
  if (err instanceof DOMException && err.name === 'AbortError') {
    return new ApiError({ kind: 'timeout', message: 'request timed out', cause: err })
  }
  const message = err instanceof Error ? err.message : 'network request failed'
  return new ApiError({ kind: 'network', message, cause: err })
}
