// Renders assets/icon.svg (and icon-small.svg for tiny sizes) into the PNGs
// and the multi-size .ico that the build embeds.
//
//   npm install --no-save @resvg/resvg-js@2.6.2
//   node tools/render-icon.mjs
//
// The outputs are committed, so you only need this after editing the SVGs.

import { Resvg } from "@resvg/resvg-js";
import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const assets = join(dirname(fileURLToPath(import.meta.url)), "..", "assets");
const big = readFileSync(join(assets, "icon.svg"), "utf8");
const small = readFileSync(join(assets, "icon-small.svg"), "utf8");

function render(size) {
  const svg = size <= 24 ? small : big;
  return new Resvg(svg, { fitTo: { mode: "width", value: size } }).render().asPng();
}

// PNGs used by the window, the tray and the docs.
for (const size of [32, 64, 256]) {
  writeFileSync(join(assets, `icon-${size}.png`), render(size));
}

// .ico with PNG-compressed entries (supported since Windows Vista).
const sizes = [16, 20, 24, 32, 40, 48, 64, 128, 256];
const images = sizes.map(render);
const header = Buffer.alloc(6);
header.writeUInt16LE(0, 0); // reserved
header.writeUInt16LE(1, 2); // type: icon
header.writeUInt16LE(sizes.length, 4);
const entries = [];
let offset = 6 + 16 * sizes.length;
sizes.forEach((size, i) => {
  const e = Buffer.alloc(16);
  e.writeUInt8(size >= 256 ? 0 : size, 0); // width (0 means 256)
  e.writeUInt8(size >= 256 ? 0 : size, 1); // height
  e.writeUInt8(0, 2); // palette
  e.writeUInt8(0, 3); // reserved
  e.writeUInt16LE(1, 4); // colour planes
  e.writeUInt16LE(32, 6); // bits per pixel
  e.writeUInt32LE(images[i].length, 8);
  e.writeUInt32LE(offset, 12);
  offset += images[i].length;
  entries.push(e);
});
writeFileSync(join(assets, "icon.ico"), Buffer.concat([header, ...entries, ...images]));
console.log(`wrote icon.ico (${sizes.join(", ")}) and icon-{32,64,256}.png`);
