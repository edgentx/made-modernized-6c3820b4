# MADE PWA (`web/`)

Mobile-first **React + TypeScript** PWA client for the MADE card game
(VForce360 Track B). Vite build, client-side routing, a service worker + web
app manifest for offline / OTA bundles, and a **shell-agnostic** bundle that
runs standalone in a browser and, unchanged, inside a Capacitor native
container.

## Commands

```sh
npm install
npm run dev      # Vite dev server
npm run build    # tsc --noEmit && vite build → dist/ (manifest + service worker)
npm run preview  # serve the production build locally
npm run icons    # regenerate PWA raster icons (public/icons/*.png)
```

## Routing

Hash-based client routing (`createHashRouter`) — no server rewrite needed, so
the same `dist/` resolves over `http(s)://`, `file://`, and `capacitor://`.

Core routes (always present): `/match`, `/collection`, `/shop`,
`/leaderboard`, `/story`. Each is a placeholder filled in by a later story.

## Capability flags (build-time gating)

The token / marketplace / wallet flows are gated so native app-store shells can
disable them (store-policy compliance). Flags are parsed from `VITE_CAP_*` env
vars in `vite.config.ts` and injected as literal `define` globals, so a disabled
flow's route **and its JS chunk are eliminated** from the build — not merely
hidden. Unset ⇒ enabled (full open-web build). See `.env.example`.

```sh
# Native-shell build with the gated flows removed:
VITE_CAP_TOKEN=false VITE_CAP_MARKETPLACE=false VITE_CAP_WALLET=false npm run build
```

## Capacitor

`base: './'` (relative asset URLs) + hash routing keep the bundle
shell-agnostic. `capacitor.config.json` points `webDir` at `dist/`; a native
shell wraps that bundle without code changes.
