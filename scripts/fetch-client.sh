#!/usr/bin/env bash
# Downloads the matching trusttunnel_client binary from the official GitHub
# release into ./src-tauri/resources/ so `cargo tauri build` can bundle it.
#
# Usage:
#   ./scripts/fetch-client.sh <asset-name>
#   ./scripts/fetch-client.sh --os linux --arch x86_64 [--tag v1.0.49]
set -euo pipefail

OS=""
ARCH=""
TAG=""
ASSET=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --os) OS="$2"; shift 2 ;;
    --arch) ARCH="$2"; shift 2 ;;
    --tag) TAG="$2"; shift 2 ;;
    -*)
      echo "Unknown flag: $1" >&2
      exit 1
      ;;
    *)
      ASSET="$1"
      shift
      ;;
  esac
done

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RES_DIR="${ROOT}/src-tauri/resources"
mkdir -p "$RES_DIR"

if [[ -z "$TAG" ]]; then
  echo "Fetching latest release tag..."
  # `--fail-with-body` makes curl exit non-zero on HTTP 4xx/5xx (e.g. rate limits)
  # while still printing the response body for debugging.
  RESPONSE=$(curl -sSL --fail-with-body \
    -H "Accept: application/vnd.github+json" \
    -H "User-Agent: trusttunnel-gui-ci" \
    https://api.github.com/repos/TrustTunnel/TrustTunnelClient/releases/latest) || {
      echo "GitHub API request failed. Response:" >&2
      echo "$RESPONSE" >&2
      exit 1
    }
  # python3 is pre-installed on every GitHub Actions runner image we care about.
  TAG=$(printf '%s' "$RESPONSE" | python3 -c 'import sys, json; print(json.load(sys.stdin)["tag_name"])')
  if [[ -z "$TAG" ]]; then
    echo "Could not determine TrustTunnel client tag from response:" >&2
    echo "$RESPONSE" >&2
    exit 1
  fi
  echo "Latest tag: $TAG"
fi

if [[ -z "$ASSET" ]]; then
  if [[ -z "$OS" || -z "$ARCH" ]]; then
    echo "Provide either an asset name or both --os and --arch" >&2
    exit 1
  fi
  if [[ "$OS" == "macos" ]]; then
    ASSET="trusttunnel_client-${TAG}-macos-universal.tar.gz"
  elif [[ "$OS" == "windows" ]]; then
    ASSET="trusttunnel_client-${TAG}-windows-${ARCH}.zip"
  else
    ASSET="trusttunnel_client-${TAG}-${OS}-${ARCH}.tar.gz"
  fi
fi

URL="https://github.com/TrustTunnel/TrustTunnelClient/releases/download/${TAG}/${ASSET}"
DEST="${RES_DIR}/${ASSET}"
echo "Downloading: $URL"
curl -sSL -o "$DEST" "$URL"

echo "Extracting ${ASSET}..."
case "$ASSET" in
  *.zip)
    unzip -o "$DEST" -d "$RES_DIR"
    ;;
  *.tar.gz)
    tar -xzf "$DEST" -C "$RES_DIR"
    ;;
esac
rm -f "$DEST"

BIN_NAME="trusttunnel_client"
[[ "$ASSET" == *windows* ]] && BIN_NAME="trusttunnel_client.exe"

found=$(find "$RES_DIR" -name "$BIN_NAME" -type f | head -n1 || true)
if [[ -n "$found" && "$(dirname "$found")" != "$RES_DIR" ]]; then
  mv -f "$found" "$RES_DIR/$BIN_NAME"
fi

if [[ "$ASSET" == *windows* ]]; then
  wintun=$(find "$RES_DIR" -name 'wintun.dll' -type f | head -n1 || true)
  if [[ -n "$wintun" && "$(dirname "$wintun")" != "$RES_DIR" ]]; then
    mv -f "$wintun" "$RES_DIR/wintun.dll"
  fi
fi

chmod +x "$RES_DIR/$BIN_NAME" 2>/dev/null || true

# Remove any leftover extracted subdirectories (e.g. "trusttunnel_client-vX.Y.Z-linux-x86_64/")
# — we already lifted the relevant files to the top of resources/ above.
find "$RES_DIR" -mindepth 1 -maxdepth 1 -type d -name 'trusttunnel_client-*' -exec rm -rf {} +

echo "Done. resources/:"
ls -la "$RES_DIR"
