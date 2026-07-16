/**
 * MatchConnection transport tests — the client→server command wire.
 *
 * The reconciler predicts locally; the connection is what actually reaches the
 * authoritative server. This asserts `send()` ships the structured envelope the
 * server's `ClientMessage` parses (`{ type:"action", matchId, command, payload }`)
 * rather than the bare command `kind` the scaffold once accepted — the fix that
 * closes the client↔server command drift and activates the online command path.
 */
import { describe, expect, it, vi } from 'vitest'
import { MatchConnection, type ConnectionHandlers } from './connection'
import type { MatchAction } from './model'

// A minimal fake WebSocket in the OPEN state whose `send` records each frame.
class FakeSocket {
  static readonly OPEN = 1
  readyState = FakeSocket.OPEN
  readonly sent: string[] = []
  send(frame: string): void {
    this.sent.push(frame)
  }
  close(): void {}
}

/** Wire a MatchConnection whose live socket is a spyable FakeSocket. */
function makeTestConnection(matchId = 'm-42'): { conn: MatchConnection; socket: FakeSocket } {
  const handlers: ConnectionHandlers = { onMessage: vi.fn(), onStatus: vi.fn() }
  const conn = new MatchConnection(handlers, matchId)
  const socket = new FakeSocket()
  // Inject the fake as the connection's live socket (bypassing the real open()).
  ;(conn as unknown as { socket: FakeSocket }).socket = socket
  return { conn, socket }
}

describe('MatchConnection.send', () => {
  it('ships the structured envelope, not the bare kind', () => {
    const { conn, socket } = makeTestConnection('m-42')
    const action: MatchAction = { kind: 'AttackCmd', seat: 'A', attackerId: 'A-atk', targetRef: 'boss:B' }

    const ok = conn.send(action)

    expect(ok).toBe(true)
    expect(socket.sent).toHaveLength(1)
    const env = JSON.parse(socket.sent[0])
    expect(env).toMatchObject({
      type: 'action',
      command: 'AttackCmd',
      payload: { seat: 'A', attackerId: 'A-atk', targetRef: 'boss:B' },
    })
    // The command carries no bare `kind` — it moved into `command`.
    expect(env.payload.kind).toBeUndefined()
    expect(env.matchId).toBe('m-42')
  })

  it('returns false and sends nothing when the socket is not open', () => {
    const { conn, socket } = makeTestConnection()
    socket.readyState = 0 // CONNECTING

    const ok = conn.send({ kind: 'EndTurnCmd', seat: 'A' })

    expect(ok).toBe(false)
    expect(socket.sent).toHaveLength(0)
  })
})
