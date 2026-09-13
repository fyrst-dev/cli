#!/usr/bin/env bash
# Install fyrst-cli from GitHub Releases (Linux amd64 / arm64).
#
# One-liner (after this file is on main):
#   curl -fsSL https://raw.githubusercontent.com/fyrst-dev/cli/main/scripts/install.sh | bash
#
# Env:
#   FYRST_CLI_VERSION  Release tag (e.g. v0.1.0). Default: latest.
#   PREFIX             Install prefix. Default: /usr/local
#   BINDIR             Binary directory. Default: $PREFIX/bin
#   REPO               GitHub repo owner/name. Default: fyrst-dev/cli
#   GITHUB_TOKEN / GH_TOKEN  Optional; used only as an Authorization header
#                            (never printed) if GitHub rate-limits anonymous API.
#   FYRST_CLI_GITHUB_API     API root. Default: https://api.github.com
#   FYRST_CLI_GITHUB_DOWNLOAD Download root. Default: https://github.com
set -euo pipefail

REPO="${REPO:-fyrst-dev/cli}"
PREFIX="${PREFIX:-/usr/local}"
BINDIR="${BINDIR:-${PREFIX}/bin}"
BINARY_NAME="fyrst-cli"
GITHUB_API="${FYRST_CLI_GITHUB_API:-https://api.github.com}"
GITHUB_DOWNLOAD="${FYRST_CLI_GITHUB_DOWNLOAD:-https://github.com}"
INSTALL_TMP=""

cleanup() {
  if [[ -n "${INSTALL_TMP:-}" ]]; then
    rm -rf "$INSTALL_TMP"
  fi
}
trap cleanup EXIT

usage() {
  cat <<EOF
Install ${BINARY_NAME} from GitHub Releases.

Usage:
  install.sh [VERSION]
  install.sh --help

VERSION may also be set via FYRST_CLI_VERSION (e.g. v0.1.0). Default: latest.

Environment:
  PREFIX      Install prefix (default: /usr/local)
  BINDIR      Binary dir (default: \$PREFIX/bin)
  REPO        GitHub repo (default: fyrst-dev/cli)

Examples:
  curl -fsSL https://raw.githubusercontent.com/${REPO}/main/scripts/install.sh | bash
  PREFIX="\$HOME/.local" bash scripts/install.sh
  FYRST_CLI_VERSION=v0.1.0 bash scripts/install.sh
EOF
}

log() { printf '==> %s\n' "$*" >&2; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }

have_cmd() { command -v "$1" >/dev/null 2>&1; }

# GitHub API / asset fetches. Token is passed as a header only; never echoed.
github_auth_args() {
  local token="${GITHUB_TOKEN:-${GH_TOKEN:-}}"
  if [[ -n "$token" ]]; then
    printf '%s\n' "Authorization: Bearer ${token}"
  fi
}

http_get_stdout() {
  local url="$1"
  local auth
  auth="$(github_auth_args || true)"
  if have_cmd curl; then
    if [[ -n "$auth" ]]; then
      curl -fsSL --retry 3 --connect-timeout 15 \
        -H "Accept: application/vnd.github+json" \
        -H "User-Agent: fyrst-cli-install" \
        -H "$auth" \
        "$url"
    else
      curl -fsSL --retry 3 --connect-timeout 15 \
        -H "Accept: application/vnd.github+json" \
        -H "User-Agent: fyrst-cli-install" \
        "$url"
    fi
  elif have_cmd wget; then
    if [[ -n "$auth" ]]; then
      wget -q -O - --header="Accept: application/vnd.github+json" \
        --header="User-Agent: fyrst-cli-install" \
        --header="$auth" \
        "$url"
    else
      wget -q -O - --header="Accept: application/vnd.github+json" \
        --header="User-Agent: fyrst-cli-install" \
        "$url"
    fi
  else
    die "need curl or wget"
  fi
}

http_download_file() {
  local url="$1" out="$2"
  local auth
  auth="$(github_auth_args || true)"
  if have_cmd curl; then
    if [[ -n "$auth" ]]; then
      curl -fsSL --retry 3 --connect-timeout 15 \
        -H "User-Agent: fyrst-cli-install" \
        -H "$auth" \
        -o "$out" "$url"
    else
      curl -fsSL --retry 3 --connect-timeout 15 \
        -H "User-Agent: fyrst-cli-install" \
        -o "$out" "$url"
    fi
  elif have_cmd wget; then
    if [[ -n "$auth" ]]; then
      wget -q --header="User-Agent: fyrst-cli-install" --header="$auth" \
        -O "$out" "$url"
    else
      wget -q --header="User-Agent: fyrst-cli-install" -O "$out" "$url"
    fi
  else
    die "need curl or wget"
  fi
}

http_download_optional() {
  local url="$1" out="$2"
  local auth
  auth="$(github_auth_args || true)"
  if have_cmd curl; then
    local code
    if [[ -n "$auth" ]]; then
      code="$(curl -sS -L --retry 3 --connect-timeout 15 \
        -H "User-Agent: fyrst-cli-install" \
        -H "$auth" \
        -o "$out" -w '%{http_code}' "$url" || true)"
    else
      code="$(curl -sS -L --retry 3 --connect-timeout 15 \
        -H "User-Agent: fyrst-cli-install" \
        -o "$out" -w '%{http_code}' "$url" || true)"
    fi
    [[ "$code" == "200" ]]
  elif have_cmd wget; then
    if [[ -n "$auth" ]]; then
      wget -q --header="User-Agent: fyrst-cli-install" --header="$auth" \
        -O "$out" "$url"
    else
      wget -q --header="User-Agent: fyrst-cli-install" -O "$out" "$url"
    fi
  else
    die "need curl or wget"
  fi
}

file_sha256() {
  if have_cmd sha256sum; then
    sha256sum "$1" | awk '{print $1}'
  elif have_cmd shasum; then
    shasum -a 256 "$1" | awk '{print $1}'
  else
    die "need sha256sum or shasum to verify checksums"
  fi
}

detect_target() {
  local os arch
  os="$(printf '%s' "${FYRST_CLI_UNAME_S:-$(uname -s)}" | tr '[:upper:]' '[:lower:]')"
  arch="$(printf '%s' "${FYRST_CLI_UNAME_M:-$(uname -m)}" | tr '[:upper:]' '[:lower:]')"
  case "$os" in
    linux) ;;
    *)
      die "unsupported OS '${os}'. Release binaries are Linux amd64/arm64 only. Build from source: cargo build --release"
      ;;
  esac
  case "$arch" in
    x86_64|amd64) TARGET="x86_64-unknown-linux-gnu" ;;
    aarch64|arm64) TARGET="aarch64-unknown-linux-gnu" ;;
    *)
      die "unsupported architecture '${arch}'. Supported: x86_64 (amd64), aarch64 (arm64)"
      ;;
  esac
}

parse_tag_name() {
  local json="$1"
  local tag=""
  if have_cmd python3; then
    tag="$(printf '%s' "$json" | python3 -c 'import json,sys; print(json.load(sys.stdin)["tag_name"])' 2>/dev/null || true)"
  fi
  if [[ -z "$tag" ]]; then
    tag="$(printf '%s' "$json" | tr -d '\r' | sed -n 's/^[[:space:]]*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -n 1)"
  fi
  [[ -n "$tag" ]] || die "could not parse tag_name from GitHub API response"
  printf '%s' "$tag"
}

normalize_tag() {
  local v="$1"
  if [[ "$v" == "latest" ]]; then
    printf ''
    return
  fi
  v="${v#v}"
  printf 'v%s' "$v"
}

validate_tag() {
  local tag="$1"
  [[ "$tag" =~ ^v[A-Za-z0-9._-]+$ ]] || die "unexpected release tag '${tag}'"
}

validate_repo() {
  case "$REPO" in
    *..*|/*|*/*/*|.*)
      die "invalid REPO '${REPO}' (expected owner/name)"
      ;;
  esac
  [[ "$REPO" =~ ^[A-Za-z0-9][A-Za-z0-9._-]+/[A-Za-z0-9][A-Za-z0-9._-]+$ ]] \
    || die "invalid REPO '${REPO}' (expected owner/name)"
}

# sudo must not consume stdin when this script is piped (curl | bash).
run_as_writer() {
  if [[ "${1:-}" == "--sudo" ]]; then
    shift
    if have_cmd sudo && sudo -n true 2>/dev/null; then
      sudo "$@"
    elif [[ -r /dev/tty ]] && have_cmd sudo; then
      # Prompt on the real terminal; do not read the script pipe.
      sudo "$@" </dev/tty
    else
      die "cannot write to ${BINDIR}. Re-run with PREFIX=\"\$HOME/.local\" (add \$HOME/.local/bin to PATH) or as a user who can write there"
    fi
  else
    "$@"
  fi
}

install_binary() {
  local src="$1" dest="$2"
  local dir
  dir="$(dirname "$dest")"
  local need_sudo=0
  if [[ ! -d "$dir" ]]; then
    if mkdir -p "$dir" 2>/dev/null && [[ -w "$dir" ]]; then
      :
    else
      need_sudo=1
      run_as_writer --sudo mkdir -p "$dir"
    fi
  elif [[ ! -w "$dir" ]] || { [[ -e "$dest" ]] && [[ ! -w "$dest" ]]; }; then
    need_sudo=1
  fi
  if [[ "$need_sudo" -eq 1 ]]; then
    if have_cmd install; then
      run_as_writer --sudo install -m 0755 "$src" "$dest"
    else
      run_as_writer --sudo cp "$src" "$dest"
      run_as_writer --sudo chmod 0755 "$dest"
    fi
  else
    if have_cmd install; then
      install -m 0755 "$src" "$dest"
    else
      cp "$src" "$dest"
      chmod 0755 "$dest"
    fi
  fi
}

main() {
  local arg="${1:-}"
  case "$arg" in
    -h|--help|help)
      usage
      exit 0
      ;;
    -*)
      die "unknown option '${arg}' (see --help)"
      ;;
  esac

  validate_repo
  detect_target

  local version="${arg:-${FYRST_CLI_VERSION:-}}"
  local tag="" api json
  if [[ -n "$version" ]]; then
    tag="$(normalize_tag "$version")"
    validate_tag "$tag"
    api="${GITHUB_API}/repos/${REPO}/releases/tags/${tag}"
    log "resolving ${tag} from GitHub Releases"
  else
    api="${GITHUB_API}/repos/${REPO}/releases/latest"
    log "resolving latest release from GitHub Releases"
  fi

  json="$(http_get_stdout "$api")" || die "failed to fetch ${api}"
  tag="$(parse_tag_name "$json")"
  validate_tag "$tag"

  local tarball="${BINARY_NAME}-${TARGET}.tar.gz"
  local url="${GITHUB_DOWNLOAD}/${REPO}/releases/download/${tag}/${tarball}"
  local sums_url="${GITHUB_DOWNLOAD}/${REPO}/releases/download/${tag}/SHA256SUMS"

  INSTALL_TMP="$(mktemp -d)"
  local tmp="$INSTALL_TMP"

  log "downloading ${tarball} (${tag}, ${TARGET})"
  http_download_file "$url" "${tmp}/${tarball}" || die "failed to download ${url} (no matching asset for this OS/arch?)"

  if http_download_optional "$sums_url" "${tmp}/SHA256SUMS"; then
    log "verifying SHA256"
    local expected actual
    expected="$(awk -v f="$tarball" 'NF >= 2 && $NF == f { print $1; exit }' "${tmp}/SHA256SUMS")"
    [[ -n "$expected" ]] || die "SHA256SUMS has no entry for ${tarball}"
    actual="$(file_sha256 "${tmp}/${tarball}")"
    if [[ "$expected" != "$actual" ]]; then
      die "checksum mismatch for ${tarball} (expected ${expected}, got ${actual})"
    fi
  else
    log "SHA256SUMS not present; skipping checksum verification"
  fi

  local contents
  contents="$(tar -tzf "${tmp}/${tarball}" | sed 's|^\./||')"
  [[ "$contents" == "$BINARY_NAME" ]] || die "unexpected tarball contents (wanted a single ${BINARY_NAME}): ${contents}"
  tar -xzf "${tmp}/${tarball}" -C "$tmp" "$BINARY_NAME" || tar -xzf "${tmp}/${tarball}" -C "$tmp" "./${BINARY_NAME}"
  [[ -f "${tmp}/${BINARY_NAME}" ]] || die "tarball did not contain ${BINARY_NAME}"
  chmod 0755 "${tmp}/${BINARY_NAME}"

  local dest="${BINDIR}/${BINARY_NAME}"
  log "installing to ${dest}"
  install_binary "${tmp}/${BINARY_NAME}" "$dest"

  if have_cmd "$dest"; then
    log "installed $($dest --version 2>/dev/null || echo "$BINARY_NAME ${tag}")"
  else
    log "installed ${dest} (${tag})"
    case ":${PATH}:" in
      *":${BINDIR}:"*) ;;
      *) log "note: ${BINDIR} is not on PATH" ;;
    esac
  fi
}

main "$@"
