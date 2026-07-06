/**
 * Build-time capability flags.
 *
 * These gate the token / marketplace / wallet flows that a native-shell
 * (Capacitor app-store) build must be able to disable or web-redirect, per the
 * platform policies that forbid in-app crypto/wallet surfaces.
 *
 * The values originate from `VITE_CAP_*` env vars, parsed in vite.config.ts and
 * injected as literal `define` globals (`__CAP_*__`). Reading those literals
 * here means the guards in `routes.tsx` / `AppLayout.tsx` compile to
 * `if (false) …`, so esbuild drops the branch and Rollup prunes the gated
 * view's chunk — the routes are ABSENT from a native build, not merely hidden.
 *
 * An unset flag defaults to ENABLED (see vite.config.ts), so the open-web build
 * ships the full feature set; native-shell builds opt out explicitly:
 *
 *   VITE_CAP_TOKEN=false VITE_CAP_MARKETPLACE=false VITE_CAP_WALLET=false \
 *     npm run build
 *
 * `redirectBaseUrl` lets a shell that disables a flow instead *web-redirect* it
 * (e.g. open the token store in the system browser); empty means "hide".
 */
export interface Capabilities {
  /** In-app token economy views. */
  readonly token: boolean
  /** Player-to-player marketplace / trading views. */
  readonly marketplace: boolean
  /** On-device wallet / balance views. */
  readonly wallet: boolean
  /** When a flow is disabled, external URL to redirect to (or "" to hide). */
  readonly redirectBaseUrl: string
}

export const capabilities: Capabilities = {
  token: __CAP_TOKEN__,
  marketplace: __CAP_MARKETPLACE__,
  wallet: __CAP_WALLET__,
  redirectBaseUrl: __CAP_REDIRECT_BASE_URL__,
}
