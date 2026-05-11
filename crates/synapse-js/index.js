// @supersynergy/synapse — napi-rs loader
// Auto-selects the correct platform .node binary.

const { platform, arch } = process;

function getPlatformTriple() {
  if (platform === 'darwin' && arch === 'arm64') return 'darwin-arm64';
  if (platform === 'darwin' && arch === 'x64') return 'darwin-x64';
  if (platform === 'linux' && arch === 'x64') return 'linux-x64-gnu';
  throw new Error(`@supersynergy/synapse: unsupported platform ${platform}-${arch}`);
}

let native;
try {
  native = require(`./synapse-js.${getPlatformTriple()}.node`);
} catch (e) {
  throw new Error(`@supersynergy/synapse native addon not found. Run: napi build --release\n${e.message}`);
}

module.exports = native;
