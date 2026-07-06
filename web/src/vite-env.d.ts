/// <reference types="vite/client" />
/// <reference types="vite-plugin-pwa/client" />

// Build-time capability constants injected by `define` in vite.config.ts.
// They are replaced with literal `true`/`false` (or a string) at build time,
// so guards written against them are dead-code-eliminated.
declare const __CAP_TOKEN__: boolean
declare const __CAP_MARKETPLACE__: boolean
declare const __CAP_WALLET__: boolean
declare const __CAP_REDIRECT_BASE_URL__: string
