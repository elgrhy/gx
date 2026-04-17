#!/usr/bin/env node
/**
 * GX Language — npm postinstall script
 * Downloads the correct native binary for the current platform.
 */

const https = require('https');
const fs = require('fs');
const path = require('path');
const { execSync } = require('child_process');
const os = require('os');
const zlib = require('zlib');

const VERSION = require('../package.json').version;
const REPO = 'elgrhy/gx';
const BIN_DIR = path.join(__dirname, '..', 'bin');

function getPlatformTarget() {
  const platform = os.platform();
  const arch = os.arch();

  if (platform === 'darwin') {
    return arch === 'arm64' ? 'gx-macos-arm64.tar.gz' : 'gx-macos-x64.tar.gz';
  }
  if (platform === 'linux') {
    return arch === 'arm64' ? 'gx-linux-arm64.tar.gz' : 'gx-linux-x64.tar.gz';
  }
  if (platform === 'win32') {
    return 'gx-windows-x64.zip';
  }
  throw new Error(`Unsupported platform: ${platform}/${arch}`);
}

function download(url, dest) {
  return new Promise((resolve, reject) => {
    const file = fs.createWriteStream(dest);
    const get = (u) => {
      https.get(u, (res) => {
        if (res.statusCode === 301 || res.statusCode === 302) {
          get(res.headers.location);
          return;
        }
        if (res.statusCode !== 200) {
          reject(new Error(`HTTP ${res.statusCode} from ${u}`));
          return;
        }
        res.pipe(file);
        file.on('finish', () => { file.close(); resolve(); });
      }).on('error', reject);
    };
    get(url);
  });
}

async function main() {
  try {
    const target = getPlatformTarget();
    const url = `https://github.com/${REPO}/releases/download/v${VERSION}/${target}`;
    const isWindows = process.platform === 'win32';
    const binaryName = isWindows ? 'gx.exe' : 'gx';
    const binaryPath = path.join(BIN_DIR, binaryName);

    // Skip if already installed
    if (fs.existsSync(binaryPath)) {
      console.log('gx binary already installed');
      return;
    }

    console.log(`Downloading gx v${VERSION} for ${target}...`);

    fs.mkdirSync(BIN_DIR, { recursive: true });

    const tmpFile = path.join(os.tmpdir(), target);
    await download(url, tmpFile);

    // Extract
    if (target.endsWith('.tar.gz')) {
      execSync(`tar -xzf "${tmpFile}" -C "${BIN_DIR}"`);
    } else if (target.endsWith('.zip')) {
      execSync(`unzip -o "${tmpFile}" -d "${BIN_DIR}"`);
    }

    fs.unlinkSync(tmpFile);

    if (!isWindows) {
      fs.chmodSync(binaryPath, 0o755);
    }

    console.log(`gx installed to ${binaryPath}`);

    // Verify
    const result = execSync(`"${binaryPath}" version`).toString().trim();
    console.log(`Verified: ${result}`);

  } catch (err) {
    // Don't fail the npm install if download fails — user can build from source
    console.warn(`\nWarning: Could not download gx binary: ${err.message}`);
    console.warn('You can build from source: https://github.com/elgrhy/gx');
    console.warn('Or install via: curl -sSf https://raw.githubusercontent.com/elgrhy/gx/main/install.sh | sh\n');
  }
}

main();
