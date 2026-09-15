#!/bin/bash
# SPDX-License-Identifier: AGPL-3.0-only
# Copyright (C) 2026 Manuel Baesler and contributors

# What `cargo xtask check` needs beyond the pinned toolchain: QEMU and the
# UEFI firmware for `test --qemu` and the end-to-end run.
set -euo pipefail

[ "${CLAUDE_CODE_REMOTE:-}" = "true" ] || exit 0

if ! command -v qemu-system-x86_64 >/dev/null; then
  export DEBIAN_FRONTEND=noninteractive
  # The cached index points at withdrawn .deb versions: update first.
  sudo_cmd=""
  [ "$(id -u)" -eq 0 ] || sudo_cmd="sudo"
  $sudo_cmd apt-get update -q
  $sudo_cmd apt-get install -y -q --no-install-recommends qemu-system-x86 ovmf
fi

# The xtask finds the firmware next to the QEMU binary; name it when it cannot.
if [ -n "${CLAUDE_ENV_FILE:-}" ] && [ -z "${AUDHSOS_OVMF:-}" ]; then
  for image in /usr/share/OVMF/OVMF_CODE_4M.fd /usr/share/OVMF/OVMF_CODE.fd /usr/share/ovmf/OVMF.fd; do
    [ -f "$image" ] && echo "export AUDHSOS_OVMF=$image" >> "$CLAUDE_ENV_FILE" && break
  done
fi

# Warm the build cache so the first check does not compile from scratch.
sh "${CLAUDE_PROJECT_DIR:-.}/tools/xtask.sh" --help >/dev/null 2>&1 || true
