import Placeholder from '../components/Placeholder'

// Capability-gated (VITE_CAP_TOKEN). Absent from native-shell builds.
export default function TokenView() {
  return <Placeholder title="Tokens" blurb="In-app token economy and rewards." />
}
