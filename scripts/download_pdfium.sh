#!/usr/bin/env bash
set -euo pipefail

# scripts/download_pdfium.sh
# Verified runtime downloader for Google Pdfium dynamic libraries.
# Source: bblanchon/pdfium-binaries (Chromium 8125)

VERSION="8125"
RELEASE_TAG="chromium%2F8125"
BASE_URL="https://github.com/bblanchon/pdfium-binaries/releases/download/${RELEASE_TAG}"

TARGET_DIR="${1:-${HOME}/.local/share/kkpdf-zed/lib}"

OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
ARCH="$(uname -m)"

case "${OS}" in
    linux)
        case "${ARCH}" in
            x86_64)
                ARCHIVE="pdfium-linux-x64.tgz"
                EXPECTED_SHA256="e1c3ce6e270cb5e0c20816b379ad9f2c1309972697c505f508de92bf5e7d6659"
                LIB_NAME="libpdfium.so"
                ;;
            aarch64|arm64)
                ARCHIVE="pdfium-linux-arm64.tgz"
                EXPECTED_SHA256="8b8818f2ec57d1b213d952ca6d7f0ffeca9d0110487d0e20ad83ba918bc92efa"
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
                EXPECTED_SHA256="8a5193535b80c7215d8932f6647ae5097465b6dba2811602c21e120c2af112b0"
                LIB_NAME="libpdfium.dylib"
                ;;
            arm64|aarch64)
                ARCHIVE="pdfium-mac-arm64.tgz"
                EXPECTED_SHA256="4939bd935ac83ddc09a3a4f038bc43a19cea1660316a3667f26cb07a070e139d"
                LIB_NAME="libpdfium.dylib"
                ;;
            *)
                echo "Unsupported macOS architecture: ${ARCH}" >&2
                exit 1
                ;;
        esac
        ;;
    msys*|mingw*|cygwin*|windows*)
        case "${ARCH}" in
            x86_64|amd64)
                ARCHIVE="pdfium-win-x64.tgz"
                EXPECTED_SHA256="9a964261760cff52570aabf038ab346d3a5034dd8b30d0d5fe8861235d356b8b"
                LIB_NAME="pdfium.dll"
                ;;
            *)
                echo "Unsupported Windows architecture: ${ARCH}" >&2
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

if [[ -f "${TMP_DIR}/bin/${LIB_NAME}" ]]; then
    cp -f "${TMP_DIR}/bin/${LIB_NAME}" "${TARGET_DIR}/${LIB_NAME}"
elif [[ -f "${TMP_DIR}/lib/${LIB_NAME}" ]]; then
    cp -f "${TMP_DIR}/lib/${LIB_NAME}" "${TARGET_DIR}/${LIB_NAME}"
elif [[ -f "${TMP_DIR}/${LIB_NAME}" ]]; then
    cp -f "${TMP_DIR}/${LIB_NAME}" "${TARGET_DIR}/${LIB_NAME}"
else
    echo "ERROR: Could not find ${LIB_NAME} inside archive!" >&2
    exit 1
fi

if [[ -f "${TMP_DIR}/lib/${LIB_NAME}.lib" ]]; then
    cp -f "${TMP_DIR}/lib/${LIB_NAME}.lib" "${TARGET_DIR}/${LIB_NAME}.lib"
fi

chmod 755 "${TARGET_DIR}/${LIB_NAME}"
echo "==> Installed ${TARGET_DIR}/${LIB_NAME} successfully."
