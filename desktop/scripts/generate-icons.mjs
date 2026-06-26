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

async function svgToBmp(svgPath, bmpPath, width, height) {
  const pngPath = bmpPath.replace(/\.bmp$/i, '.png');
  run('npx', ['--yes', '@resvg/resvg-js-cli', svgPath, pngPath, '--fit-width', String(width), '--fit-height', String(height)]);
  await sharp(pngPath).toFile(bmpPath);
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

console.log('Installer assets generated.');
