import Placeholder from '../components/Placeholder'

// Capability-gated (VITE_CAP_WALLET). Absent from native-shell builds.
export default function WalletView() {
  return <Placeholder title="Wallet" blurb="On-device balances and transaction history." />
}
