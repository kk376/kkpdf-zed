#!/usr/bin/env bash
set -euo pipefail

# scripts/download_pdfium.sh
# Verified runtime downloader for Google Pdfium dynamic libraries.
# Source: bblanchon/pdfium-binaries (Chromium 8035)

VERSION="8035"
RELEASE_TAG="chromium%2F8035"
BASE_URL="https://github.com/bblanchon/pdfium-binaries/releases/download/${RELEASE_TAG}"

TARGET_DIR="${1:-${HOME}/.local/share/kkpdf-zed/lib}"

OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
ARCH="$(uname -m)"

case "${OS}" in
    linux)
        case "${ARCH}" in
            x86_64)
                ARCHIVE="pdfium-linux-x64.tgz"
                EXPECTED_SHA256="2e6db042dd2cff2d5247023dbec6c7ebb800042ce83c835d6468d45229669bd4"
                LIB_NAME="libpdfium.so"
                ;;
            aarch64|arm64)
                ARCHIVE="pdfium-linux-arm64.tgz"
                EXPECTED_SHA256="10bb7728a5268593a31c2024e0e57ea26b73b7a54195054f43d2ad0869d55a48"
                LIB_NAME="libpdfium.so"
                ;;
            *)
                echo "Unsupported Linux architecture: ${ARCH}" >&2
                exit 1
                ;;
        esac
        ;;
    darwin)
        case "${ARCH}" in
            x86_64)
                ARCHIVE="pdfium-mac-x64.tgz"
                EXPECTED_SHA256="9170dd3bb0f14a712369dd8a1978e77e0b5a05c4371aca2ee49727daabf3201a"
                LIB_NAME="libpdfium.dylib"
                ;;
            arm64|aarch64)
                ARCHIVE="pdfium-mac-arm64.tgz"
                EXPECTED_SHA256="308fd9c6eff1be5b7bde62e7a9a42f525075901314a2a50058ae0b6ea0ff30a2"
                LIB_NAME="libpdfium.dylib"
                ;;
            *)
                echo "Unsupported macOS architecture: ${ARCH}" >&2
                exit 1
                ;;
        esac
        ;;
    *)
        echo "Unsupported OS: ${OS}" >&2
        exit 1
        ;;
esac

echo "==> Target directory: ${TARGET_DIR}"
echo "==> Target asset:     ${ARCHIVE} (${LIB_NAME})"
echo "==> Expected SHA256:  ${EXPECTED_SHA256}"

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "${TMP_DIR}"' EXIT

ARCHIVE_PATH="${TMP_DIR}/${ARCHIVE}"

echo "==> Downloading from ${BASE_URL}/${ARCHIVE}..."
curl -sSL --fail "${BASE_URL}/${ARCHIVE}" -o "${ARCHIVE_PATH}"

echo "==> Verifying SHA256 checksum..."
ACTUAL_SHA256="$(sha256sum "${ARCHIVE_PATH}" | awk '{print $1}')"
if [[ "${ACTUAL_SHA256}" != "${EXPECTED_SHA256}" ]]; then
    echo "ERROR: SHA256 mismatch for ${ARCHIVE}!" >&2
    echo "  Expected: ${EXPECTED_SHA256}" >&2
    echo "  Actual:   ${ACTUAL_SHA256}" >&2
    exit 1
fi
echo "==> Checksum verified successfully."

mkdir -p "${TARGET_DIR}"

echo "==> Extracting ${LIB_NAME} to ${TARGET_DIR}..."
tar -xzf "${ARCHIVE_PATH}" -C "${TMP_DIR}"

if [[ -f "${TMP_DIR}/lib/${LIB_NAME}" ]]; then
    cp -f "${TMP_DIR}/lib/${LIB_NAME}" "${TARGET_DIR}/${LIB_NAME}"
elif [[ -f "${TMP_DIR}/${LIB_NAME}" ]]; then
    cp -f "${TMP_DIR}/${LIB_NAME}" "${TARGET_DIR}/${LIB_NAME}"
else
    echo "ERROR: Could not find ${LIB_NAME} inside archive!" >&2
    exit 1
fi

chmod 755 "${TARGET_DIR}/${LIB_NAME}"
echo "==> Installed ${TARGET_DIR}/${LIB_NAME} successfully."
