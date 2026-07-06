import { NavLink, Outlet } from 'react-router-dom'

interface NavItem {
  to: string
  label: string
}

// Core navigation — always present.
const coreNav: NavItem[] = [
  { to: '/match', label: 'Match' },
  { to: '/collection', label: 'Deck' },
  { to: '/shop', label: 'Shop' },
  { to: '/leaderboard', label: 'Ranks' },
  { to: '/story', label: 'Story' },
]

// Capability-gated nav. Each entry is guarded by an inlined build flag, so the
// whole `.push` is dead-code-eliminated when the flag is false in a shell build.
const gatedNav: NavItem[] = []
if (__CAP_TOKEN__) gatedNav.push({ to: '/token', label: 'Tokens' })
if (__CAP_MARKETPLACE__) gatedNav.push({ to: '/marketplace', label: 'Market' })
if (__CAP_WALLET__) gatedNav.push({ to: '/wallet', label: 'Wallet' })

const navItems = [...coreNav, ...gatedNav]

/**
 * App shell: a persistent bottom tab bar (mobile-first) wrapping the routed
 * <Outlet>. Nav entries mirror the capability-gated route table.
 */
export default function AppLayout() {
  return (
    <div className="app">
      <main className="app__content">
        <Outlet />
      </main>
      <nav className="app__nav" aria-label="Primary">
        {navItems.map((item) => (
          <NavLink
            key={item.to}
            to={item.to}
            className={({ isActive }) =>
              isActive ? 'app__nav-link app__nav-link--active' : 'app__nav-link'
            }
          >
            {item.label}
          </NavLink>
        ))}
      </nav>
    </div>
  )
}
