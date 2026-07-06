/**
 * Generate the PWA raster icons required for installability (Lighthouse wants a
 * 192px and a 512px icon, plus a maskable variant). We render a simple branded
 * tile — a solid background with a centered rounded "card" — into a raw RGBA
 * buffer and encode a valid PNG using only Node's built-in `zlib` (no native
 * deps), so this runs anywhere CI does.
 *
 * Run: `node scripts/gen-icons.mjs` (also wired as the `icons` npm script).
 */
import { deflateSync } from 'node:zlib'
import { writeFileSync, mkdirSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { dirname, resolve } from 'node:path'

const here = dirname(fileURLToPath(import.meta.url))
const iconsDir = resolve(here, '../public/icons')
mkdirSync(iconsDir, { recursive: true })

// CRC-32 (PNG chunk checksums).
const CRC_TABLE = (() => {
  const t = new Uint32Array(256)
  for (let n = 0; n < 256; n++) {
    let c = n
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1
    t[n] = c >>> 0
  }
  return t
})()

function crc32(buf) {
  let c = 0xffffffff
  for (let i = 0; i < buf.length; i++) c = CRC_TABLE[(c ^ buf[i]) & 0xff] ^ (c >>> 8)
  return (c ^ 0xffffffff) >>> 0
}

function chunk(type, data) {
  const len = Buffer.alloc(4)
  len.writeUInt32BE(data.length, 0)
  const typeBuf = Buffer.from(type, 'ascii')
  const body = Buffer.concat([typeBuf, data])
  const crc = Buffer.alloc(4)
  crc.writeUInt32BE(crc32(body), 0)
  return Buffer.concat([len, body, crc])
}

function encodePng(width, height, rgba) {
  const sig = Buffer.from([137, 80, 78, 71, 13, 10, 26, 10])
  const ihdr = Buffer.alloc(13)
  ihdr.writeUInt32BE(width, 0)
  ihdr.writeUInt32BE(height, 4)
  ihdr[8] = 8 // bit depth
  ihdr[9] = 6 // color type: RGBA
  ihdr[10] = 0 // compression
  ihdr[11] = 0 // filter
  ihdr[12] = 0 // interlace
  // Prefix each scanline with filter byte 0.
  const stride = width * 4
  const raw = Buffer.alloc((stride + 1) * height)
  for (let y = 0; y < height; y++) {
    raw[y * (stride + 1)] = 0
    rgba.copy(raw, y * (stride + 1) + 1, y * stride, y * stride + stride)
  }
  const idat = deflateSync(raw)
  return Buffer.concat([
    sig,
    chunk('IHDR', ihdr),
    chunk('IDAT', idat),
    chunk('IEND', Buffer.alloc(0)),
  ])
}

const BG = [11, 13, 18, 255] // #0b0d12
const CARD = [76, 139, 245, 255] // #4c8bf5

// Draw background + a centered rounded-ish card block. `inset` controls the
// safe zone: maskable icons need ~10% padding so nothing is clipped when the
// platform applies a mask.
function render(size, insetRatio) {
  const buf = Buffer.alloc(size * size * 4)
  const inset = Math.floor(size * insetRatio)
  const cardX0 = inset
  const cardX1 = size - inset
  const cardY0 = Math.floor(size * (insetRatio + 0.05))
  const cardY1 = size - Math.floor(size * (insetRatio + 0.05))
  const radius = Math.floor(size * 0.08)
  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      let c = BG
      if (x >= cardX0 && x < cardX1 && y >= cardY0 && y < cardY1) {
        // Round the card corners.
        const nx = Math.min(x - cardX0, cardX1 - 1 - x)
        const ny = Math.min(y - cardY0, cardY1 - 1 - y)
        const inCorner = nx < radius && ny < radius
        const dx = radius - nx
        const dy = radius - ny
        if (!inCorner || dx * dx + dy * dy <= radius * radius) c = CARD
      }
      const i = (y * size + x) * 4
      buf[i] = c[0]
      buf[i + 1] = c[1]
      buf[i + 2] = c[2]
      buf[i + 3] = c[3]
    }
  }
  return encodePng(size, size, buf)
}

const outputs = [
  ['pwa-192.png', render(192, 0.16)],
  ['pwa-512.png', render(512, 0.16)],
  ['pwa-maskable-512.png', render(512, 0.22)],
  ['apple-touch-icon.png', render(180, 0.16)],
]

for (const [name, png] of outputs) {
  writeFileSync(resolve(iconsDir, name), png)
  console.log(`wrote icons/${name} (${png.length} bytes)`)
}
