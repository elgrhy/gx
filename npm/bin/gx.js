#!/usr/bin/env node
/**
 * GX Language — npm wrapper
 * Delegates to the native gx binary.
 */

const { spawnSync } = require('child_process');
const path = require('path');
const fs = require('fs');
const os = require('os');

const isWindows = process.platform === 'win32';
const binaryName = isWindows ? 'gx.exe' : 'gx';
const binaryPath = path.join(__dirname, binaryName);

if (!fs.existsSync(binaryPath)) {
  console.error('Error: gx binary not found.');
  console.error('Try reinstalling: npm install -g gxlang');
  console.error('Or: curl -sSf https://raw.githubusercontent.com/elgrhy/gx/main/install.sh | sh');
  process.exit(1);
}

const result = spawnSync(binaryPath, process.argv.slice(2), {
  stdio: 'inherit',
  env: process.env,
});

if (result.error) {
  console.error(`Failed to run gx: ${result.error.message}`);
  process.exit(1);
}

process.exit(result.status ?? 0);
