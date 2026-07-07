import { execFileSync } from 'node:child_process';
import { existsSync, mkdtempSync, rmSync, unlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import sharp from 'sharp';
import pngToIco from 'png-to-ico';

const ICO_SIZES = [16, 24, 32, 48, 64, 128, 256];

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const iconsDir = join(root, 'src-tauri', 'icons');
const appIconSvg = join(root, 'app-icon.svg');
const appIconPng = join(root, 'app-icon.png');

function run(cmd, args, cwd = root) {
  execFileSync(cmd, args, { cwd, stdio: 'inherit', shell: process.platform === 'win32' });
}

async function writeMultiSizeIco(pngPath, icoPath) {
  const dir = mkdtempSync(join(tmpdir(), 'flowyrouter-ico-'));
  try {
    const paths = [];
    for (const size of ICO_SIZES) {
      const path = join(dir, `${size}.png`);
      await sharp(pngPath).resize(size, size).png().toFile(path);
      paths.push(path);
    }
    writeFileSync(icoPath, await pngToIco(paths));
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

function writeBmp24(rgbBuffer, width, height, bmpPath) {
  const rowSize = Math.ceil((width * 3) / 4) * 4;
  const pixelDataSize = rowSize * height;
  const fileSize = 54 + pixelDataSize;
  const buffer = Buffer.alloc(fileSize);

  buffer.write('BM', 0);
  buffer.writeUInt32LE(fileSize, 2);
  buffer.writeUInt32LE(54, 10);
  buffer.writeUInt32LE(40, 14);
  buffer.writeInt32LE(width, 18);
  buffer.writeInt32LE(height, 22);
  buffer.writeUInt16LE(1, 26);
  buffer.writeUInt16LE(24, 28);

  let offset = 54;
  for (let y = height - 1; y >= 0; y--) {
    for (let x = 0; x < width; x++) {
      const src = (y * width + x) * 3;
      buffer[offset++] = rgbBuffer[src + 2];
      buffer[offset++] = rgbBuffer[src + 1];
      buffer[offset++] = rgbBuffer[src];
    }
    const padding = rowSize - width * 3;
    for (let p = 0; p < padding; p++) buffer[offset++] = 0;
  }

  writeFileSync(bmpPath, buffer);
}

async function svgToBmp(svgPath, bmpPath, width, height) {
  const pngPath = bmpPath.replace(/\.bmp$/i, '.png');
  run('npx', ['--yes', '@resvg/resvg-js-cli', svgPath, pngPath, '--fit-width', String(width), '--fit-height', String(height)]);
  const { data, info } = await sharp(pngPath)
    .resize(width, height, { fit: 'fill' })
    .removeAlpha()
    .raw()
    .toBuffer({ resolveWithObject: true });
  writeBmp24(data, info.width, info.height, bmpPath);
  unlinkSync(pngPath);
}

if (!existsSync(appIconSvg)) {
  console.error('missing app-icon.svg');
  process.exit(1);
}

run('npx', ['--yes', '@resvg/resvg-js-cli', appIconSvg, appIconPng]);
run('npx', ['tauri', 'icon', appIconPng], root);
await writeMultiSizeIco(appIconPng, join(iconsDir, 'icon.ico'));

await svgToBmp(join(iconsDir, 'installer-sidebar.svg'), join(iconsDir, 'installer-sidebar.bmp'), 164, 314);
await svgToBmp(join(iconsDir, 'installer-header.svg'), join(iconsDir, 'installer-header.bmp'), 150, 57);
await svgToBmp(join(iconsDir, 'installer-wix-dialog.svg'), join(iconsDir, 'installer-wix-dialog.bmp'), 493, 312);
await svgToBmp(join(iconsDir, 'installer-wix-banner.svg'), join(iconsDir, 'installer-wix-banner.bmp'), 493, 58);

console.log('Installer assets generated.');
