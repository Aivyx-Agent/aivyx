#!/bin/bash
# First-launch (OOBE) setup for the Aivyx PA WSL distribution. Invoked once, as
# root, by /etc/wsl-distribution.conf the first time the distro starts. Its job
# is to create an unprivileged default user (WSL discourages running as root)
# and point the operator at the getting-started commands. Aivyx PA itself is
# already installed on PATH from the appliance image.
set -eu

DEFAULT_USER="aivyx"

echo "Setting up the Aivyx PA WSL distribution…"

# Create the default user (uid 1000, matching wsl-distribution.conf defaultUid)
# with passwordless sudo — the conventional WSL default — if absent.
if ! id -u "$DEFAULT_USER" >/dev/null 2>&1; then
    useradd --create-home --shell /bin/bash --user-group --groups sudo "$DEFAULT_USER"
    passwd --delete "$DEFAULT_USER" >/dev/null 2>&1 || true
    install -d -m 0755 /etc/sudoers.d
    echo "$DEFAULT_USER ALL=(ALL) NOPASSWD:ALL" > /etc/sudoers.d/90-aivyx
    chmod 0440 /etc/sudoers.d/90-aivyx
fi

cat <<'BANNER'

  Aivyx PA is installed in this WSL distribution.

  Get started:
      aivyx-pa init   # one-time guided setup (model, access level, agent)
      aivyx-pa        # start chatting — auto-starts the local daemon

  Once the daemon is running, the Web Studio is at
      http://127.0.0.1:7843      (open it from your Windows browser)

  Your files live inside this distro and are reachable from Windows at
      \\wsl$\Aivyx-PA\home\aivyx

BANNER
