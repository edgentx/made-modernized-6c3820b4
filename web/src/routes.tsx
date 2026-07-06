import { Navigate, type RouteObject } from 'react-router-dom'
import AppLayout from './components/AppLayout'

// Core views are always in the bundle → static imports.
import MatchView from './views/MatchView'
import CollectionView from './views/CollectionView'
import ShopView from './views/ShopView'
import LeaderboardView from './views/LeaderboardView'
import StoryView from './views/StoryView'
import NotFoundView from './views/NotFoundView'

// Capability-gated routes. The `__CAP_*__` guards are literal booleans injected
// by `define`, so when a flag is false the branch — and the only `import()`
// reference to that view — is dead-code-eliminated, and Rollup drops the view's
// chunk entirely. The route is thus ABSENT from a native-shell bundle, not
// merely hidden at runtime.
const gatedRoutes: RouteObject[] = []
if (__CAP_TOKEN__) {
  gatedRoutes.push({
    path: 'token',
    lazy: async () => ({ Component: (await import('./views/TokenView')).default }),
  })
}
if (__CAP_MARKETPLACE__) {
  gatedRoutes.push({
    path: 'marketplace',
    lazy: async () => ({ Component: (await import('./views/MarketplaceView')).default }),
  })
}
if (__CAP_WALLET__) {
  gatedRoutes.push({
    path: 'wallet',
    lazy: async () => ({ Component: (await import('./views/WalletView')).default }),
  })
}

export const routes: RouteObject[] = [
  {
    path: '/',
    element: <AppLayout />,
    children: [
      { index: true, element: <Navigate to="/match" replace /> },
      { path: 'match', element: <MatchView /> },
      { path: 'collection', element: <CollectionView /> },
      { path: 'shop', element: <ShopView /> },
      { path: 'leaderboard', element: <LeaderboardView /> },
      { path: 'story', element: <StoryView /> },
      ...gatedRoutes,
      { path: '*', element: <NotFoundView /> },
    ],
  },
]
