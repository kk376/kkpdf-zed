#!/usr/bin/env bash
set -euo pipefail

# scripts/download_pdfium.sh
# Verified runtime downloader for Google Pdfium dynamic libraries.
# Source: bblanchon/pdfium-binaries (Chromium 8138)

VERSION="8138"
RELEASE_TAG="chromium%2F8138"
BASE_URL="https://github.com/bblanchon/pdfium-binaries/releases/download/${RELEASE_TAG}"

TARGET_DIR="${1:-${HOME}/.local/share/kkpdf-zed/lib}"

OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
ARCH="$(uname -m)"

case "${OS}" in
    linux)
        case "${ARCH}" in
            x86_64)
                ARCHIVE="pdfium-linux-x64.tgz"
                EXPECTED_SHA256="d6b1e28950a0392590d79bbdbb7eb19686872b688f8a36f170acb19e41473113"
                LIB_NAME="libpdfium.so"
                ;;
            aarch64|arm64)
                ARCHIVE="pdfium-linux-arm64.tgz"
                EXPECTED_SHA256="c70b1ce4d13257f741af19e0aeee575890875995eb81370a65be942e601a1862"
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
                EXPECTED_SHA256="72662ac8eff7bf5381ef82ceebd0a0ad5bddb051b303981115114e6972fd201c"
                LIB_NAME="libpdfium.dylib"
                ;;
            arm64|aarch64)
                ARCHIVE="pdfium-mac-arm64.tgz"
                EXPECTED_SHA256="21a1567b397b571f4f77f21ca91eac7b892774219c3c715a5127a20950ccb18b"
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
                EXPECTED_SHA256="8d48973e3eb472dda0f5f2435e595ac043f9f2219dd71883392f549445afa8a5"
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
if command -v sha256sum >/dev/null 2>&1; then
    ACTUAL_SHA256="$(sha256sum "${ARCHIVE_PATH}" | awk '{print $1}')"
elif command -v shasum >/dev/null 2>&1; then
    ACTUAL_SHA256="$(shasum -a 256 "${ARCHIVE_PATH}" | awk '{print $1}')"
else
    echo "ERROR: Neither sha256sum nor shasum is available for checksum verification" >&2
    exit 1
fi
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
