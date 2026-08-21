#!/bin/sh
# Install mimo-code-herdr: downloads the release binary for your platform,
# installs it to ~/.local/bin, and runs `mimo-herdr install` to wire up the
# MiMo Code plugin.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/ShinoharaHaruna/mimo-code-herdr/main/install.sh | sh
#
# Env overrides:
#   MIMO_HERDR_VERSION   release tag to fetch (default: latest)
#   MIMO_HERDR_INSTALL_DIR   install dir (default: ~/.local/bin)
#
# Dev/testing:
#   install.sh --dry-run            print what would happen, change nothing
#   install.sh --from-tarball FILE  install from a local release tarball

set -eu

REPO="ShinoharaHaruna/mimo-code-herdr"
VERSION="${MIMO_HERDR_VERSION:-latest}"
INSTALL_DIR="${MIMO_HERDR_INSTALL_DIR:-$HOME/.local/bin}"
DRY_RUN=0
FROM_TARBALL=""

while [ "$#" -gt 0 ]; do
  case "$1" in
    --dry-run) DRY_RUN=1; shift ;;
    --from-tarball) FROM_TARBALL="${2:-}"; shift 2 ;;
    --from-tarball=*) FROM_TARBALL="${1#*=}"; shift ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

detect_target() {
  case "$(uname -s)-$(uname -m)" in
    Linux-x86_64|Linux-amd64) echo "x86_64-unknown-linux-gnu" ;;
    Darwin-arm64) echo "aarch64-apple-darwin" ;;
    Darwin-x86_64) echo "x86_64-apple-darwin" ;;
    *)
      echo "unsupported platform: $(uname -s)-$(uname -m)" >&2
      exit 1
      ;;
  esac
}

resolve_version() {
  if [ "$VERSION" = "latest" ]; then
    # Resolve the latest release tag via the GitHub API.
    tag="$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" 2>/dev/null \
      | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -1)"
    if [ -z "$tag" ]; then
      echo "could not resolve the latest release" >&2
      exit 1
    fi
    echo "$tag"
  else
    echo "$VERSION"
  fi
}

TARGET="$(detect_target)"
TARBALL="mimo-herdr-$TARGET.tar.gz"

if [ "$DRY_RUN" = 1 ]; then
  echo "mimo-code-herdr installer (dry run)"
  echo "  version:  $VERSION"
  echo "  target:   $TARGET"
  echo "  install:  $INSTALL_DIR/mimo-herdr"
  if [ -n "$FROM_TARBALL" ]; then
    echo "  source:   $FROM_TARBALL (local)"
  else
    echo "  source:   https://github.com/$REPO/releases/download/<tag>/$TARBALL"
  fi
  echo "dry run: nothing was changed"
  exit 0
fi

if [ -z "$FROM_TARBALL" ]; then
  VERSION="$(resolve_version)"
  BASE_URL="https://github.com/$REPO/releases/download/$VERSION"
  URL="$BASE_URL/$TARBALL"
  SHA_URL="$BASE_URL/$TARBALL.sha256"
fi

echo "mimo-code-herdr installer"
echo "  version:  $VERSION"
echo "  target:   $TARGET"
echo "  install:  $INSTALL_DIR/mimo-herdr"

mkdir -p "$INSTALL_DIR"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT HUP INT TERM

if [ -n "$FROM_TARBALL" ]; then
  cp "$FROM_TARBALL" "$TMP/$TARBALL"
else
  echo "downloading $URL"
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL -o "$TMP/$TARBALL" "$URL"
    curl -fsSL -o "$TMP/$TARBALL.sha256" "$SHA_URL"
  elif command -v wget >/dev/null 2>&1; then
    wget -qO "$TMP/$TARBALL" "$URL"
    wget -qO "$TMP/$TARBALL.sha256" "$SHA_URL"
  else
    echo "need curl or wget" >&2
    exit 1
  fi
  # Verify checksum when a .sha256 file was fetched alongside.
  if [ -f "$TMP/$TARBALL.sha256" ] && command -v shasum >/dev/null 2>&1; then
    (cd "$TMP" && shasum -a 256 -c "$TARBALL.sha256") >/dev/null
  fi
fi

tar -xzf "$TMP/$TARBALL" -C "$TMP"
chmod +x "$TMP/mimo-herdr"
mv "$TMP/mimo-herdr" "$INSTALL_DIR/mimo-herdr"
echo "installed $INSTALL_DIR/mimo-herdr"

# Wire up the MiMo Code plugin (bakes the binary path into the plugin).
"$INSTALL_DIR/mimo-herdr" install

echo "done. Run \`mimo-herdr status\` to verify, then \`mimo-herdr spawn --name runner\`."
echo "Note: ensure $INSTALL_DIR is on your PATH."
