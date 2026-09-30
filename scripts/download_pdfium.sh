#!/usr/bin/env bash
set -euo pipefail

# scripts/download_pdfium.sh
# Verified runtime downloader for Google Pdfium dynamic libraries.
# Source: bblanchon/pdfium-binaries (Chromium 8076)

VERSION="8076"
RELEASE_TAG="chromium%2F8076"
BASE_URL="https://github.com/bblanchon/pdfium-binaries/releases/download/${RELEASE_TAG}"

TARGET_DIR="${1:-${HOME}/.local/share/kkpdf-zed/lib}"

OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
ARCH="$(uname -m)"

case "${OS}" in
    linux)
        case "${ARCH}" in
            x86_64)
                ARCHIVE="pdfium-linux-x64.tgz"
                EXPECTED_SHA256="d9d67bc40af03aef4fe28a60b19b1086f28ace019c8c9caf19cb7fe3d14ceca3"
                LIB_NAME="libpdfium.so"
                ;;
            aarch64|arm64)
                ARCHIVE="pdfium-linux-arm64.tgz"
                EXPECTED_SHA256="d7247b33ae5545615a5e877235dd97afc879e3a8805689684f528cae3339d352"
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
                EXPECTED_SHA256="40865f34642c34d82cc336132df9e0347133f4692cd46776647af160f9a5cca9"
                LIB_NAME="libpdfium.dylib"
                ;;
            arm64|aarch64)
                ARCHIVE="pdfium-mac-arm64.tgz"
                EXPECTED_SHA256="0d6781fe08906baff3d82c90953e519fbc4eb253fe76431e5ed53b157763b97c"
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
                EXPECTED_SHA256="808d36da9bc5a3104315fb307c80998121f565ee53953633bf33e80d7429e5ac"
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
