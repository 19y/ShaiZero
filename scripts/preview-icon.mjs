// Renders assets/icon-preview.png: every icon size on a light and a dark band,
// small sizes blown up so the tray rendering can be judged at a glance.
//
// Run with: node scripts/preview-icon.mjs
import { deflateSync, inflateSync } from "node:zlib";
import { readFileSync, writeFileSync } from "node:fs";

const ico = readFileSync("assets/shaiwin.ico");
const count = ico.readUInt16LE(4);

function decode(base) {
  const w = ico[base] === 0 ? 256 : ico[base];
  const len = ico.readUInt32LE(base + 8);
  const off = ico.readUInt32LE(base + 12);
  const data = ico.subarray(off, off + len);
  // 64 px and up are stored as PNG frames; the rest are 32 bpp DIBs.
  if (data.toString("hex", 0, 8) === "89504e470d0a1a0a") {
    return { size: w, px: decodePng(data) };
  }
  const header = data.readUInt32LE(0);
  const h = data.readInt32LE(8) / 2;
  if (data.readUInt16LE(14) !== 32) throw new Error("expected a 32 bpp entry");
  const px = Buffer.alloc(w * h * 4);
  for (let y = 0; y < h; y++) {
    for (let x = 0; x < w; x++) {
      const src = header + ((h - 1 - y) * w + x) * 4;
      const dst = (y * w + x) * 4;
      px[dst] = data[src + 2]; // R
      px[dst + 1] = data[src + 1]; // G
      px[dst + 2] = data[src]; // B
      px[dst + 3] = data[src + 3];
    }
  }
  return { size: w, px };
}

/// RGBA PNG frame (8 bit, non-interlaced) back into straight pixels.
function decodePng(buf) {
  let ihdr = null;
  const idat = [];
  let off = 8;
  while (off + 12 <= buf.length) {
    const len = buf.readUInt32BE(off);
    const type = buf.toString("ascii", off + 4, off + 8);
    if (type === "IHDR") ihdr = buf.subarray(off + 8, off + 8 + len);
    else if (type === "IDAT") idat.push(buf.subarray(off + 8, off + 8 + len));
    off += 12 + len;
  }
  const w = ihdr.readUInt32BE(0);
  const h = ihdr.readUInt32BE(4);
  const ch = ihdr[9] === 6 ? 4 : 3;
  const raw = inflateSync(Buffer.concat(idat));
  const stride = w * ch;
  const chan = Buffer.alloc(h * stride);
  let pos = 0;
  for (let y = 0; y < h; y++) {
    const kind = raw[pos++];
    const line = raw.subarray(pos, pos + stride);
    pos += stride;
    const cur = chan.subarray(y * stride, (y + 1) * stride);
    for (let i = 0; i < stride; i++) {
      const left = i >= ch ? cur[i - ch] : 0;
      const above = y ? chan[y * stride - stride + i] : 0;
      let pred;
      if (kind === 0) pred = 0;
      else if (kind === 1) pred = left;
      else if (kind === 2) pred = above;
      else if (kind === 3) pred = (left + above) >> 1;
      else {
        const ul = y && i >= ch ? chan[y * stride - stride + i - ch] : 0;
        const p = left + above - ul;
        const pa = Math.abs(p - left);
        const pb = Math.abs(p - above);
        const pc = Math.abs(p - ul);
        pred = pa <= pb && pa <= pc ? left : pb <= pc ? above : ul;
      }
      cur[i] = line[i] + pred;
    }
  }
  const px = Buffer.alloc(w * h * 4);
  for (let i = 0; i < w * h; i++) {
    px[i * 4] = chan[i * ch];
    px[i * 4 + 1] = chan[i * ch + 1];
    px[i * 4 + 2] = chan[i * ch + 2];
    px[i * 4 + 3] = ch === 4 ? chan[i * ch + 3] : 255;
  }
  return px;
}

const entries = [];
for (let i = 0; i < count; i++) entries.push(decode(6 + i * 16));

// Big enough to see, small enough to read: the tray sizes get the most help.
const SHOW = [
  [16, 8],
  [20, 8],
  [24, 8],
  [32, 4],
  [48, 4],
  [64, 4],
  [128, 2],
  [256, 1],
];

const pad = 20;
const gap = 24;
const cellH = Math.max(...SHOW.map(([size, scale]) => size * scale));
const bandW = pad * 2 + SHOW.reduce((sum, [size, scale]) => sum + size * scale, 0) + gap * (SHOW.length - 1);
const bandH = cellH + pad * 2;
const W = bandW;
const H = bandH * 2;

const out = Buffer.alloc(W * H * 4);
for (let y = 0; y < H; y++) {
  const light = y < bandH;
  for (let x = 0; x < W; x++) {
    const i = (y * W + x) * 4;
    // A taskbar-ish light band and a taskbar-ish dark band.
    out[i] = light ? 243 : 32;
    out[i + 1] = light ? 243 : 32;
    out[i + 2] = light ? 246 : 36;
    out[i + 3] = 255;
  }
}

for (const baseY of [0, bandH]) {
  let x = pad;
  for (const [size, scale] of SHOW) {
    const found = entries.find((e) => e.size === size);
    if (!found) continue;
    const top = baseY + pad + Math.floor((cellH - size * scale) / 2);
    for (let dy = 0; dy < size * scale; dy++) {
      for (let dx = 0; dx < size * scale; dx++) {
        const sx = Math.floor(dx / scale);
        const sy = Math.floor(dy / scale);
        const si = (sy * size + sx) * 4;
        const a = found.px[si + 3] / 255;
        if (a <= 0) continue;
        const o = ((top + dy) * W + (x + dx)) * 4;
        out[o] = found.px[si] * a + out[o] * (1 - a);
        out[o + 1] = found.px[si + 1] * a + out[o + 1] * (1 - a);
        out[o + 2] = found.px[si + 2] * a + out[o + 2] * (1 - a);
      }
    }
    x += size * scale + gap;
  }
}

// PNG encode (filter 0).
const raw = Buffer.alloc((W * 4 + 1) * H);
for (let y = 0; y < H; y++) {
  raw[y * (W * 4 + 1)] = 0;
  out.copy(raw, y * (W * 4 + 1) + 1, y * W * 4, (y + 1) * W * 4);
}
const crcTable = (() => {
  const t = new Int32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    t[n] = c;
  }
  return t;
})();
const crc = (buf) => {
  let c = 0xffffffff;
  for (const byte of buf) c = crcTable[(c ^ byte) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
};
const chunk = (type, data) => {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length);
  const body = Buffer.concat([Buffer.from(type, "ascii"), data]);
  const c = Buffer.alloc(4);
  c.writeUInt32BE(crc(body));
  return Buffer.concat([len, body, c]);
};
const ihdr = Buffer.alloc(13);
ihdr.writeUInt32BE(W, 0);
ihdr.writeUInt32BE(H, 4);
ihdr[8] = 8;
ihdr[9] = 6;
const png = Buffer.concat([
  Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
  chunk("IHDR", ihdr),
  chunk("IDAT", deflateSync(raw, { level: 9 })),
  chunk("IEND", Buffer.alloc(0)),
]);
writeFileSync("assets/icon-preview.png", png);
console.log(`wrote assets/icon-preview.png (${W}x${H})`);
