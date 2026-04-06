#!/usr/bin/env node
// Copies the platform-specific binary to bin/mtw-mcp after install.
// Pattern used by esbuild, turbo, biome, etc.

const { existsSync, mkdirSync, copyFileSync, chmodSync } = require("fs");
const { join } = require("path");

const PLATFORMS = {
  "linux-x64": "@matware/mtw-request-mcp-linux-x64",
  "linux-arm64": "@matware/mtw-request-mcp-linux-arm64",
  "darwin-x64": "@matware/mtw-request-mcp-darwin-x64",
  "darwin-arm64": "@matware/mtw-request-mcp-darwin-arm64",
  "win32-x64": "@matware/mtw-request-mcp-win32-x64",
};

const platform = process.platform;
const arch = process.arch;
const key = `${platform}-${arch}`;
const pkg = PLATFORMS[key];

if (!pkg) {
  console.warn(`mtw-mcp: no pre-built binary for ${key}, you can build from source with: cargo build --release -p mtw-mcp`);
  process.exit(0);
}

try {
  const binaryDir = join(__dirname, "bin");
  if (!existsSync(binaryDir)) mkdirSync(binaryDir, { recursive: true });

  const binaryName = platform === "win32" ? "mtw-mcp.exe" : "mtw-mcp";
  const sourcePath = join(require.resolve(pkg + "/package.json"), "..", binaryName);
  const destPath = join(binaryDir, binaryName);

  if (existsSync(sourcePath)) {
    copyFileSync(sourcePath, destPath);
    if (platform !== "win32") chmodSync(destPath, 0o755);
    console.log(`mtw-mcp: installed ${key} binary`);
  } else {
    console.warn(`mtw-mcp: binary not found in ${pkg}, build from source: cargo build --release -p mtw-mcp`);
  }
} catch (e) {
  console.warn(`mtw-mcp: postinstall failed (${e.message}), build from source: cargo build --release -p mtw-mcp`);
}
