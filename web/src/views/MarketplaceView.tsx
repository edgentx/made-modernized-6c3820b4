import Placeholder from '../components/Placeholder'

// Capability-gated (VITE_CAP_MARKETPLACE). Absent from native-shell builds.
export default function MarketplaceView() {
  return <Placeholder title="Marketplace" blurb="Player-to-player card trading." />
}
