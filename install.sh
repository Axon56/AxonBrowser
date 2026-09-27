#!/usr/bin/env bash
set -euo pipefail

REPO="${AXONBROWSER_REPO:-ayomidelog/AxonBrowser}"
VERSION="${AXONBROWSER_VERSION:-${VERSION:-latest}}"
INSTALL_DIR="${AXONBROWSER_INSTALL_DIR:-/usr/local/bin}"
BASE_URL="${AXONBROWSER_BASE_URL:-}"
INSTALL_DEPS="${AXONBROWSER_INSTALL_DEPS:-0}"
VERIFY_CHECKSUM="${AXONBROWSER_VERIFY_CHECKSUM:-1}"
TMP_DIR=""

cleanup() {
  if [[ -n "${TMP_DIR:-}" ]]; then
    rm -rf "$TMP_DIR"
  fi
}

need_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "install.sh: required command not found: $1" >&2
    exit 1
  fi
}

is_writable_path() {
  local path="$1"
  if [[ -d "$path" ]]; then
    [[ -w "$path" ]]
    return
  fi

  local parent
  parent="$(dirname "$path")"
  [[ -w "$parent" ]]
}

run_install() {
  if is_writable_path "$INSTALL_DIR"; then
    "$@"
    return
  fi

  if command -v sudo >/dev/null 2>&1; then
    sudo "$@"
    return
  fi

  echo "install.sh: cannot write to $INSTALL_DIR and sudo is unavailable" >&2
  echo "install.sh: set AXONBROWSER_INSTALL_DIR to a writable directory, for example ~/.local/bin" >&2
  exit 1
}

asset_urls() {
  local asset_name="$1"
  if [[ -n "$BASE_URL" ]]; then
    printf '%s\n' "${BASE_URL%/}/${asset_name}"
    return
  fi

  if [[ "$VERSION" == "latest" ]]; then
    printf '%s\n' "https://github.com/${REPO}/releases/latest/download/${asset_name}"
    return
  fi

  printf '%s\n' "https://github.com/${REPO}/releases/download/${VERSION}/${asset_name}"
}

main() {
  need_cmd curl
  need_cmd tar
  need_cmd install
  need_cmd mktemp

  local os arch bundle_name bundle_asset checksum_asset bundle_url checksum_url
  os="$(uname -s)"
  arch="$(uname -m)"

  if [[ "$os" != "Linux" ]]; then
    echo "install.sh: unsupported OS: $os" >&2
    echo "install.sh: this installer currently publishes Linux release assets only" >&2
    exit 1
  fi

  case "$arch" in
    x86_64)
      bundle_name="axonbrowser-linux-x86_64"
      ;;
    aarch64 | arm64)
      bundle_name="axonbrowser-linux-aarch64"
      ;;
    *)
      echo "install.sh: unsupported architecture: $arch" >&2
      echo "install.sh: available release bundles: axonbrowser-linux-x86_64, axonbrowser-linux-aarch64" >&2
      exit 1
      ;;
  esac

  bundle_asset="${bundle_name}.tar.gz"
  checksum_asset="${bundle_asset}.sha256"
  bundle_url="$(asset_urls "$bundle_asset")"
  checksum_url="$(asset_urls "$checksum_asset")"

  local tmp_dir bundle_path checksum_path extract_dir
  tmp_dir="$(mktemp -d)"
  TMP_DIR="$tmp_dir"
  trap cleanup EXIT
  bundle_path="${tmp_dir}/${bundle_asset}"
  checksum_path="${tmp_dir}/${checksum_asset}"
  extract_dir="${tmp_dir}/extract"

  echo "Downloading ${bundle_asset} from ${bundle_url}"
  curl -fsSL "$bundle_url" -o "$bundle_path"

  if [[ "$VERIFY_CHECKSUM" == "1" ]]; then
    need_cmd sha256sum
    echo "Downloading ${checksum_asset}"
    curl -fsSL "$checksum_url" -o "$checksum_path"
    (
      cd "$tmp_dir"
      sha256sum -c "$checksum_asset"
    )
  fi

  mkdir -p "$extract_dir"
  tar -xzf "$bundle_path" -C "$extract_dir"

  if [[ ! -f "$extract_dir/bin/axonbrowser" ]]; then
    echo "install.sh: release bundle is missing the axonbrowser binary" >&2
    exit 1
  fi

  run_install mkdir -p "$INSTALL_DIR"
  run_install install -m 755 "$extract_dir/bin/axonbrowser" "$INSTALL_DIR/axonbrowser"

  echo "Installed axonbrowser to $INSTALL_DIR/axonbrowser"

  if [[ "$INSTALL_DEPS" == "1" ]]; then
    echo "Running axonbrowser install-deps"
    "$INSTALL_DIR/axonbrowser" install-deps
  else
    echo "Next: $INSTALL_DIR/axonbrowser install-deps"
  fi
}

main "$@"
