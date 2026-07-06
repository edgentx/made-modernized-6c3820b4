import { RouterProvider, createHashRouter } from 'react-router-dom'
import { routes } from './routes'

// Hash routing keeps the bundle shell-agnostic: it needs no server-side URL
// rewrite and resolves correctly over the `file://` / `capacitor://` origins a
// native container serves from, so the same `dist/` runs on the web and inside
// Capacitor without code changes.
const router = createHashRouter(routes)

export default function App() {
  return <RouterProvider router={router} />
}
