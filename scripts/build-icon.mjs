// Draws assets/shaiwin.ico: the chosen "B light" mark — a dark rounded badge,
// a broken white ring with the top sector lifted out in amber, and a white
// pivot dot.  The numbers below are the ones the B sheet was drawn with in
// icon_proposals/render_icons.ps1 (DrawB), so the .ico and the preview match.
//
// The art is rasterised at every size the shell asks for, never scaled from a
// big one: 16 px and 24 px (the 150% DPI SM_CXSMICON) get drawings of their own,
// with a thicker ring and wider gaps, because an 8° gap at 16 px is a third of a
// pixel and the ring reads as a solid circle. 64 px and up go into the container
// as PNG; Windows has decoded PNG icon frames since Vista, and the bytes are far
// smaller than an equivalent DIB.
//
// Rendering goes through Edge's headless screenshot mode, because it is the one
// SVG rasteriser that is already on every Windows box this builds on. The PNGs
// come back straight-alpha, so packing them into a DIB is a channel shuffle.
//
// Run with: node scripts/build-icon.mjs
// Env: EDGE=/path/to/msedge.exe to override the browser lookup.
import { execFileSync } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { deflateSync, inflateSync } from "node:zlib";

// --- geometry ----------------------------------------------------------------

const CX = 512;
const CY = 512;
const BADGE_INSET = 0;
const BADGE_RADIUS = 225;

const SEGMENTS = 6;
const STEP = 60;
const START = -114;

const AMBER = "#DD9A4E";
const PIVOT = "#F7F9FC";
const BADGE = "#262C38";
const RING = "#F7F9FC";

/// One annular slice of the ring, angles in degrees clockwise from the top
/// (SVG's y axis grows downwards, so -114 is the upper-left of the lifted
/// sector).
function sectorPath(rOut, rIn, a0, a1) {
  const rad0 = (a0 * Math.PI) / 180;
  const rad1 = (a1 * Math.PI) / 180;
  const p = (r, a) => `${(CX + r * Math.cos(a)).toFixed(2)} ${(CY + r * Math.sin(a)).toFixed(2)}`;
  return (
    `M${p(rIn, rad0)} ` +
    `L${p(rOut, rad0)} ` +
    `A${rOut} ${rOut} 0 0 1 ${p(rOut, rad1)} ` +
    `L${p(rIn, rad1)} ` +
    `A${rIn} ${rIn} 0 0 0 ${p(rIn, rad0)} Z`
  );
}

/// One drawing. The three variants differ only in these numbers: the ring
/// thickens and the seams widen as the size drops, so the white ring keeps its
/// edge against the dark badge and the gaps stay a visible pixel.
function dial(o) {
  const body = [
    `<rect x="${BADGE_INSET}" y="${BADGE_INSET}" width="${1024 - BADGE_INSET * 2}" ` +
      `height="${1024 - BADGE_INSET * 2}" rx="${BADGE_RADIUS}" fill="${o.badge}" ` +
      `stroke="${o.edge}" stroke-width="${o.edgeWidth}"/>`,
  ];
  for (let k = 0; k < SEGMENTS; k++) {
    const a0 = START + k * STEP;
    body.push(`<path d="${sectorPath(o.Rout, o.Rin, a0, a0 + o.span)}" fill="${RING}"/>`);
  }
  // The lifted sector: same inner edge as the ring, further out and wider than
  // the slot it covers, so it reads as raised rather than as one more segment.
  body.push(`<path d="${sectorPath(o.amberR, o.Rin - 6, o.amberA0, o.amberA1)}" fill="${AMBER}"/>`);
  body.push(`<circle cx="${CX}" cy="${CY}" r="${o.hubR}" fill="${o.badge}"/>`);
  body.push(`<circle cx="${CX}" cy="${CY}" r="${o.pivotR}" fill="${PIVOT}"/>`);
  return body.join("\n  ");
}

function svg(options) {
  return (
    `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1024 1024" ` +
    `width="1024" height="1024">\n  ${dial(options)}\n</svg>\n`
  );
}

// 48 / 64 / 128 / 256: the full drawing, straight from DrawB in
// icon_proposals/render_icons.ps1 (0.395 / 0.235 / 52°, amber 0.445 at
// -111°..-69°, white dot 0.062).
const LARGE = {
  badge: BADGE,
  edge: "none",
  edgeWidth: 0,
  Rout: 404,
  Rin: 241,
  span: 52,
  amberR: 456,
  amberA0: -111,
  amberA1: -69,
  hubR: 247,
  pivotR: 64,
};

// 24 / 32: a heavier ring and wider seams, so the white still separates at
// 24 px; the badge stays the same dark plate.
const MID = {
  badge: BADGE,
  edge: "none",
  edgeWidth: 0,
  Rout: 420,
  Rin: 246,
  span: 48,
  amberR: 462,
  amberA0: -114,
  amberA1: -66,
  hubR: 252,
  pivotR: 66,
};

// 16: the ring is two thirds of the radius, each seam is 16° (1.5 px at this
// size), and the lifted sector is widened to 52° so it stays one shape instead
// of an amber blur.
const SMALL = {
  badge: BADGE,
  edge: "none",
  edgeWidth: 0,
  Rout: 424,
  Rin: 252,
  span: 44,
  amberR: 462,
  amberA0: -116,
  amberA1: -64,
  hubR: 258,
  pivotR: 68,
};

const VARIANTS = [
  { name: "small", text: svg(SMALL), sizes: [16] },
  { name: "mid", text: svg(MID), sizes: [24, 32] },
  { name: "large", text: svg(LARGE), sizes: [48, 64, 128, 256] },
];

// --- rendering ---------------------------------------------------------------

const EDGE_CANDIDATES = [
  process.env.EDGE,
  "C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe",
  "C:/Program Files/Microsoft/Edge/Application/msedge.exe",
].filter(Boolean);

const edge = EDGE_CANDIDATES.find((p) => existsSync(p));
if (!edge) {
  throw new Error(
    "找不到 Edge，用 EDGE=<msedge.exe 的路径> 指定一个。" +
      "Edge 只用来把 SVG 光栅化，任何 Chromium 内核的浏览器都能顶替。",
  );
}

const scratch = mkdtempSync(join(tmpdir(), "shaiwin-icon-"));

/// Rasterises one SVG at one size. The transparent background flag is what
/// keeps the icon's corners transparent instead of white.
///
/// `display:block` is not cosmetic. An inline <svg> is a replaced inline
/// element, so it sits on the text baseline and the line box reserves descender
/// space underneath it. When the viewport is exactly the icon size there is no
/// room for that space, and the browser pushes the drawing down by the
/// descender — at 16 px that is three pixels, which cuts the bottom off.
function render(text, size) {
  const page = join(scratch, `r${size}.html`);
  const png = join(scratch, `r${size}.png`);
  const sized = text
    .replace('width="1024" height="1024"', `width="${size}" height="${size}"`)
    .replace("<svg ", '<svg style="display:block" ');
  writeFileSync(
    page,
    `<!doctype html><meta charset="utf-8">` +
      `<body style="margin:0;background:transparent">${sized}</body>`,
    "utf8",
  );
  execFileSync(edge, [
    "--headless=new",
    "--disable-gpu",
    "--hide-scrollbars",
    "--default-background-color=00000000",
    "--force-device-scale-factor=1",
    `--screenshot=${png}`,
    `--window-size=${size},${size}`,
    `file:///${page.replace(/\\/g, "/")}`,
  ], { stdio: "ignore" });
  const img = decodePng(readFileSync(png));
  if (img.w !== size || img.h !== size) {
    throw new Error(`${size}px: browser returned ${img.w}x${img.h}`);
  }
  return img;
}

// --- PNG decoding ------------------------------------------------------------

/// Chunked, filtered scanlines back into straight RGBA. Only what a browser
/// screenshot produces is supported: 8 bit, non-interlaced, RGB or RGBA.
function decodePng(buf) {
  if (buf.toString("hex", 0, 8) !== "89504e470d0a1a0a") throw new Error("not a PNG");
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
  const [depth, color, compression, filter, interlace] = [
    ihdr[8], ihdr[9], ihdr[10], ihdr[11], ihdr[12],
  ];
  if (depth !== 8 || interlace !== 0 || compression !== 0 || filter !== 0) {
    throw new Error(`unsupported PNG: depth=${depth} interlace=${interlace}`);
  }
  if (color !== 6 && color !== 2) throw new Error(`unsupported PNG colour type ${color}`);
  const channels = color === 6 ? 4 : 3;
  const raw = inflateSync(Buffer.concat(idat));
  const stride = w * channels;
  const px = Buffer.alloc(h * stride);
  let pos = 0;
  for (let y = 0; y < h; y++) {
    const kind = raw[pos++];
    const line = raw.subarray(pos, pos + stride);
    pos += stride;
    const cur = px.subarray(y * stride, (y + 1) * stride);
    const up = y > 0 ? px.subarray((y - 1) * stride, y * stride) : null;
    unfilter(kind, cur, line, up, channels);
  }
  return { w, h, channels, px };
}

function unfilter(kind, cur, line, up, channels) {
  if (kind === 0) {
    line.copy(cur);
    return;
  }
  for (let i = 0; i < line.length; i++) {
    const left = i >= channels ? cur[i - channels] : 0;
    const above = up ? up[i] : 0;
    let pred;
    if (kind === 1) pred = left;
    else if (kind === 2) pred = above;
    else if (kind === 3) pred = (left + above) >> 1;
    else if (kind === 4) {
      const upperLeft = up && i >= channels ? up[i - channels] : 0;
      const p = left + above - upperLeft;
      const pa = Math.abs(p - left);
      const pb = Math.abs(p - above);
      const pc = Math.abs(p - upperLeft);
      pred = pa <= pb && pa <= pc ? left : pb <= pc ? above : upperLeft;
    } else throw new Error(`unknown filter type ${kind}`);
    // Buffer assignment wraps at 256, which is the arithmetic PNG wants.
    cur[i] = line[i] + pred;
  }
}

// --- the .ico container ------------------------------------------------------

/// Entries below this size are DIBs; from here up they are PNG. A 256 px DIB is
/// 262 kB of raw rows, which is why Explorer-era icons ballooned.
const PNG_MIN = 64;

/// One 32 bpp DIB entry: BITMAPINFOHEADER with the doubled height that tells a
/// reader an AND mask follows, then bottom-up BGRX rows, then that mask.
function dibEntry(img) {
  const { w, h, channels, px } = img;
  const header = Buffer.alloc(40);
  header.writeUInt32LE(40, 0);
  header.writeInt32LE(w, 4);
  header.writeInt32LE(h * 2, 8);
  header.writeUInt16LE(1, 12);
  header.writeUInt16LE(32, 14);
  header.writeUInt32LE(w * h * 4, 20);
  const xor = Buffer.alloc(w * h * 4);
  const maskStride = Math.ceil(w / 32) * 4;
  const and = Buffer.alloc(maskStride * h);
  for (let y = 0; y < h; y++) {
    const srcRow = h - 1 - y; // bottom-up
    for (let x = 0; x < w; x++) {
      const i = (srcRow * w + x) * channels;
      const o = (y * w + x) * 4;
      const a = channels === 4 ? px[i + 3] : 255;
      xor[o] = px[i + 2];
      xor[o + 1] = px[i + 1];
      xor[o + 2] = px[i];
      xor[o + 3] = a;
      if (a < 8) and[y * maskStride + (x >> 3)] |= 0x80 >> (x & 7);
    }
  }
  return Buffer.concat([header, xor, and]);
}

function writeIco(bySize, path) {
  const sizes = [...bySize.keys()].sort((a, b) => a - b);
  const images = sizes.map((size) => {
    const img = bySize.get(size);
    return {
      size,
      png: size >= PNG_MIN,
      data: size >= PNG_MIN ? encodePng(img) : dibEntry(img),
    };
  });
  const dir = Buffer.alloc(6);
  dir.writeUInt16LE(0, 0);
  dir.writeUInt16LE(1, 2);
  dir.writeUInt16LE(images.length, 4);
  let offset = 6 + images.length * 16;
  const entries = images.map(({ size, data }) => {
    const e = Buffer.alloc(16);
    e[0] = size === 256 ? 0 : size; // 256 does not fit in a byte, so it is 0
    e[1] = size === 256 ? 0 : size;
    e.writeUInt16LE(1, 4);
    e.writeUInt16LE(32, 6);
    e.writeUInt32LE(data.length, 8);
    e.writeUInt32LE(offset, 12);
    offset += data.length;
    return e;
  });
  const ico = Buffer.concat([dir, ...entries, ...images.map((i) => i.data)]);
  writeFileSync(path, ico);
  return {
    bytes: ico.length,
    sizes,
    png: images.filter((i) => i.png).map((i) => i.size),
  };
}

// --- main --------------------------------------------------------------------

const bySize = new Map();
for (const variant of VARIANTS) {
  for (const size of variant.sizes) {
    bySize.set(size, render(variant.text, size));
    console.log(`rendered ${size}px from the ${variant.name} drawing`);
  }
}

const { bytes, sizes, png } = writeIco(bySize, "assets/shaiwin.ico");
console.log(
  `wrote assets/shaiwin.ico (${bytes} bytes, ${sizes.join("/")}; ` +
    `PNG at ${png.join("/")})`,
);

rmSync(scratch, { recursive: true, force: true });

// --- PNG encoding (icon frames at 64 px and up) ------------------------------

function encodePng(img) {
  const { w, h, channels, px } = img;
  const stride = w * 4;
  const raw = Buffer.alloc((stride + 1) * h);
  for (let y = 0; y < h; y++) {
    raw[y * (stride + 1)] = 0; // filter: none
    for (let x = 0; x < w; x++) {
      const s = (y * w + x) * channels;
      const d = y * (stride + 1) + 1 + x * 4;
      raw[d] = px[s];
      raw[d + 1] = px[s + 1];
      raw[d + 2] = px[s + 2];
      raw[d + 3] = channels === 4 ? px[s + 3] : 255;
    }
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
  ihdr.writeUInt32BE(w, 0);
  ihdr.writeUInt32BE(h, 4);
  ihdr[8] = 8;
  ihdr[9] = 6;
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk("IHDR", ihdr),
    chunk("IDAT", deflateSync(raw, { level: 9 })),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}
