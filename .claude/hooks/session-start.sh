#!/bin/bash
# SPDX-License-Identifier: AGPL-3.0-only
# Copyright (C) 2026 Manuel Baesler and contributors

# What `cargo xtask check` needs beyond the pinned toolchain: QEMU and the
# UEFI firmware for `test --qemu` and the end-to-end run, and the two
# OpenSSH packages for the Secure Shell interop run.
set -euo pipefail

[ "${CLAUDE_CODE_REMOTE:-}" = "true" ] || exit 0

missing=""
command -v qemu-system-x86_64 >/dev/null || missing="$missing qemu-system-x86 ovmf"
command -v ssh-keygen >/dev/null || missing="$missing openssh-client"
[ -x /usr/sbin/sshd ] || missing="$missing openssh-server"

if [ -n "$missing" ]; then
  export DEBIAN_FRONTEND=noninteractive
  # The cached index points at withdrawn .deb versions: update first.
  sudo_cmd=""
  [ "$(id -u)" -eq 0 ] || sudo_cmd="sudo"
  $sudo_cmd apt-get update -q
  # shellcheck disable=SC2086
  $sudo_cmd apt-get install -y -q --no-install-recommends $missing
fi

# The xtask finds the firmware next to the QEMU binary; name it when it cannot.
if [ -n "${CLAUDE_ENV_FILE:-}" ] && [ -z "${AUDHSOS_OVMF:-}" ]; then
  for image in /usr/share/OVMF/OVMF_CODE_4M.fd /usr/share/OVMF/OVMF_CODE.fd /usr/share/ovmf/OVMF.fd; do
    [ -f "$image" ] && echo "export AUDHSOS_OVMF=$image" >> "$CLAUDE_ENV_FILE" && break
  done
fi

# The Secure Shell interop run authenticates the account of the run; the
# container starts with neither variable set.
if [ -n "${CLAUDE_ENV_FILE:-}" ] && [ -z "${USER:-}" ]; then
  echo "export USER=$(id -un)" >> "$CLAUDE_ENV_FILE"
fi

# `sshd` refuses to start without its privilege separation directory, which
# a container that never ran the service does not have.
if [ ! -d /run/sshd ]; then
  sudo_cmd=""
  [ "$(id -u)" -eq 0 ] || sudo_cmd="sudo"
  $sudo_cmd mkdir -p /run/sshd
fi

# Warm the build cache so the first check does not compile from scratch.
sh "${CLAUDE_PROJECT_DIR:-.}/tools/xtask.sh" --help >/dev/null 2>&1 || true
