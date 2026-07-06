import { defineConfig } from 'vitest/config'

// Standalone Vitest config (kept separate from vite.config.ts so the PWA/build
// plugins don't run under test). The API client tests are plain TS against a
// mocked network (MSW), so the default `node` environment is sufficient.
//
// The `__CAP_*__` build-time literals are `define`d here too: modules pulled in
// transitively (capabilities.ts) reference them, and unlike `vite build` there
// is no bundler substitution step under test.
export default defineConfig({
  define: {
    __CAP_TOKEN__: 'true',
    __CAP_MARKETPLACE__: 'true',
    __CAP_WALLET__: 'true',
    __CAP_REDIRECT_BASE_URL__: '""',
  },
  test: {
    environment: 'node',
    include: ['src/**/*.test.ts'],
  },
})
