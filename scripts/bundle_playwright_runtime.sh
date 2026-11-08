#!/bin/sh
set -euo pipefail

SOURCE="${NOLA_PLAYWRIGHT_SOURCE:-${SRCROOT}/web/playwright_runtime}"
DESTINATION="${NOLA_PLAYWRIGHT_OUTPUT:-${TARGET_BUILD_DIR}/${UNLOCALIZED_RESOURCES_FOLDER_PATH}/NolaPlaywrightRuntime}"
CACHE="${NOLA_PLAYWRIGHT_CACHE:-${DERIVED_FILE_DIR}/nola-playwright-runtime}"

mkdir -p "$CACHE" "$(dirname "$DESTINATION")"
cp "$SOURCE/package.json" "$SOURCE/package-lock.json" "$CACHE/"
NODE_BIN="${NOLA_NODE_BIN:-}"
if [[ -z "$NODE_BIN" ]]; then
  NODE_VERSION="24.20.0"
  case "$(uname -m)" in
    arm64) NODE_ARCH="arm64" ;;
    x86_64) NODE_ARCH="x64" ;;
    *) echo "error: unsupported Mac architecture for Playwright runtime" >&2; exit 1 ;;
  esac
  NODE_DIST="node-v${NODE_VERSION}-darwin-${NODE_ARCH}"
  NODE_ARCHIVE="$CACHE/${NODE_DIST}.tar.gz"
  # A partially cleaned/extracted cache may retain node but lose npm. Treat
  # the distribution as reusable only when both required entry points exist.
  if [[ ! -x "$CACHE/$NODE_DIST/bin/node" || ! -f "$CACHE/$NODE_DIST/lib/node_modules/npm/bin/npm-cli.js" ]]; then
    /usr/bin/curl --fail --location --retry 3 \
      "https://nodejs.org/dist/v${NODE_VERSION}/${NODE_DIST}.tar.gz" \
      --output "$NODE_ARCHIVE"
    /usr/bin/tar -xzf "$NODE_ARCHIVE" -C "$CACHE"
  fi
  NODE_BIN="$CACHE/$NODE_DIST/bin/node"
fi
# Never run an unrelated npm from Xcode's PATH. Its shebang can select an old
# system Node even when the app bundles a newer runtime. Resolve npm beside the
# selected Node and invoke its JavaScript entry point with that exact binary.
NODE_BIN="$("$NODE_BIN" -p 'process.execPath')"
NPM_BIN="${NOLA_NPM_BIN:-$(dirname "$NODE_BIN")/../lib/node_modules/npm/bin/npm-cli.js}"
if [[ ! -f "$NPM_BIN" ]]; then
  echo "error: npm entry point missing for selected Node: $NODE_BIN; set NOLA_NPM_BIN to npm-cli.js from the same Node installation" >&2
  exit 1
fi
NODE_MAJOR="$("$NODE_BIN" -p 'process.versions.node.split(".")[0]')"
NPM_VERSION="$("$NODE_BIN" "$NPM_BIN" --version)"
NPM_MAJOR="${NPM_VERSION%%.*}"
if [[ "$NODE_MAJOR" -lt 20 || "$NPM_MAJOR" -lt 9 ]]; then
  echo "error: Playwright bundling requires Node >=20 and npm >=9 (found Node $NODE_MAJOR, npm $NPM_VERSION)" >&2
  exit 1
fi
if [[ ! -d "$CACHE/node_modules/playwright-core" ]]; then
  (cd "$CACHE" && PATH="$(dirname "$NODE_BIN"):$PATH" "$NODE_BIN" "$NPM_BIN" ci --omit=dev --ignore-scripts=false)
fi
cp "$SOURCE/bridge.cjs" "$CACHE/bridge.cjs"
cp "$SOURCE/owner_queue.cjs" "$CACHE/owner_queue.cjs"
rm -rf "$DESTINATION"
mkdir -p "$DESTINATION"
cp -R "$CACHE/bridge.cjs" "$CACHE/owner_queue.cjs" "$CACHE/package.json" "$CACHE/package-lock.json" "$CACHE/node_modules" "$DESTINATION/"
mkdir -p "$DESTINATION/bin"
cp "$NODE_BIN" "$DESTINATION/bin/node"
chmod 755 "$DESTINATION/bin/node"
if [[ "${CODE_SIGNING_ALLOWED:-NO}" == "YES" ]]; then
  /usr/bin/codesign --force --sign - --timestamp=none "$DESTINATION/bin/node"
fi
touch "$DESTINATION/.ready"
