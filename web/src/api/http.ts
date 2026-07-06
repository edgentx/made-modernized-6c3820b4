/**
 * The transport core shared by every typed client method.
 *
 * Responsibilities kept in one place so the resource methods in `client.ts` stay
 * declarative:
 *
 *  - URL + query-string assembly against the configured REST base,
 *  - JSON encode/decode with `credentials: 'include'` (the edge reads the
 *    session cookie; the PWA never attaches tokens itself),
 *  - normalization of every failure into an {@link ApiError},
 *  - a per-request timeout composed with any caller-supplied `AbortSignal`,
 *  - retry with exponential backoff + jitter for transient failures, and
 *  - the S-79 hook: a `401` invokes `onUnauthorized` (the login redirect) and is
 *    never retried.
 *
 * The engine is a factory, not a singleton, so tests can inject a fake `sleep`
 * (no real backoff waits) and a spy `onUnauthorized` (no `window` dependency).
 */
import { ApiError, errorFromResponse, errorFromThrown } from './errors'

/** Query-string values; `undefined`/`null` entries are omitted. */
export type QueryValue = string | number | boolean | undefined | null
export type QueryParams = Record<string, QueryValue>

export interface RequestOptions {
  readonly method?: 'GET' | 'POST' | 'PUT' | 'PATCH' | 'DELETE'
  /** Query params appended to the URL. */
  readonly query?: QueryParams
  /** JSON request body; serialized with `JSON.stringify`. */
  readonly body?: unknown
  /** Caller abort signal, composed with the per-request timeout. */
  readonly signal?: AbortSignal
  /** Override the client's default retry count for this call. */
  readonly maxRetries?: number
}

export interface RetryConfig {
  /** Max *additional* attempts after the first (0 disables retry). */
  readonly maxRetries: number
  /** Base backoff delay in ms; grows exponentially per attempt. */
  readonly baseDelayMs: number
  /** Ceiling on any single backoff delay in ms. */
  readonly maxDelayMs: number
}

export interface HttpClientConfig {
  /** REST base URL including the version prefix, no trailing slash. */
  readonly baseUrl: string
  /** Invoked on a 401 before the error is thrown — the S-79 login redirect. */
  readonly onUnauthorized: () => void
  /** Per-request timeout in ms (0 disables). */
  readonly timeoutMs?: number
  /** Default retry policy for transient failures. */
  readonly retry?: Partial<RetryConfig>
  /** Injectable for tests; defaults to global `fetch`. */
  readonly fetchImpl?: typeof fetch
  /** Injectable for tests; defaults to a real `setTimeout` delay. */
  readonly sleep?: (ms: number) => Promise<void>
}

const DEFAULT_RETRY: RetryConfig = { maxRetries: 2, baseDelayMs: 200, maxDelayMs: 2000 }
const DEFAULT_TIMEOUT_MS = 15000

export interface HttpClient {
  request<T>(path: string, opts?: RequestOptions): Promise<T>
  readonly baseUrl: string
}

export function createHttpClient(config: HttpClientConfig): HttpClient {
  const doFetch = config.fetchImpl ?? globalThis.fetch.bind(globalThis)
  const sleep = config.sleep ?? realSleep
  const timeoutMs = config.timeoutMs ?? DEFAULT_TIMEOUT_MS
  const retry: RetryConfig = { ...DEFAULT_RETRY, ...config.retry }

  async function request<T>(path: string, opts: RequestOptions = {}): Promise<T> {
    const url = buildUrl(config.baseUrl, path, opts.query)
    const maxRetries = opts.maxRetries ?? retry.maxRetries
    const init = buildInit(opts)

    let attempt = 0
    // Loop: attempt 0 is the initial try; up to `maxRetries` retries follow.
    for (;;) {
      try {
        const res = await withTimeout(doFetch, url, init, opts.signal, timeoutMs)
        return await handleResponse<T>(res)
      } catch (raw) {
        const err = errorFromThrown(raw)
        // A 401 means the edge dropped the session mid-use: fire the login
        // redirect hook and surface immediately — retrying can't help.
        if (err.status === 401) {
          config.onUnauthorized()
          throw err
        }
        if (!err.retriable || attempt >= maxRetries || isAborted(opts.signal)) {
          throw err
        }
        await sleep(backoffDelay(attempt, retry))
        attempt += 1
      }
    }
  }

  return { request, baseUrl: config.baseUrl }
}

/** Decode a response: parse 2xx JSON (204 => undefined), normalize non-2xx. */
async function handleResponse<T>(res: Response): Promise<T> {
  if (!res.ok) {
    throw await errorFromResponse(res)
  }
  if (res.status === 204) {
    return undefined as T
  }
  const text = await res.text()
  if (!text) return undefined as T
  try {
    return JSON.parse(text) as T
  } catch (err) {
    throw new ApiError({
      kind: 'parse',
      status: res.status,
      message: 'failed to parse response body as JSON',
      cause: err,
    })
  }
}

/** Assemble a URL from base + path + query, skipping empty query values. */
export function buildUrl(baseUrl: string, path: string, query?: QueryParams): string {
  const joined = `${baseUrl}${path.startsWith('/') ? path : `/${path}`}`
  if (!query) return joined
  const params = new URLSearchParams()
  for (const [key, value] of Object.entries(query)) {
    if (value === undefined || value === null) continue
    params.append(key, String(value))
  }
  const qs = params.toString()
  return qs ? `${joined}?${qs}` : joined
}

/** Build the `fetch` init: JSON headers, credentials, serialized body. */
function buildInit(opts: RequestOptions): RequestInit {
  const headers: Record<string, string> = { Accept: 'application/json' }
  const init: RequestInit = {
    method: opts.method ?? 'GET',
    credentials: 'include',
    headers,
  }
  if (opts.body !== undefined) {
    headers['Content-Type'] = 'application/json'
    init.body = JSON.stringify(opts.body)
  }
  return init
}

/**
 * Run `fetch` under a timeout that is composed with the caller's signal: either
 * source aborting cancels the request. Returns the response or throws (an
 * `AbortError` that {@link errorFromThrown} maps to `timeout`).
 */
async function withTimeout(
  doFetch: typeof fetch,
  url: string,
  init: RequestInit,
  callerSignal: AbortSignal | undefined,
  timeoutMs: number,
): Promise<Response> {
  if (timeoutMs <= 0 && !callerSignal) {
    return doFetch(url, init)
  }
  const controller = new AbortController()
  const onCallerAbort = () => controller.abort()
  callerSignal?.addEventListener('abort', onCallerAbort)
  const timer =
    timeoutMs > 0 ? setTimeout(() => controller.abort(), timeoutMs) : undefined
  try {
    return await doFetch(url, { ...init, signal: controller.signal })
  } finally {
    if (timer !== undefined) clearTimeout(timer)
    callerSignal?.removeEventListener('abort', onCallerAbort)
  }
}

/** Exponential backoff with full jitter, capped at `maxDelayMs`. */
export function backoffDelay(attempt: number, retry: RetryConfig): number {
  const exp = Math.min(retry.maxDelayMs, retry.baseDelayMs * 2 ** attempt)
  return Math.round(Math.random() * exp)
}

function isAborted(signal: AbortSignal | undefined): boolean {
  return signal?.aborted ?? false
}

function realSleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms))
}
