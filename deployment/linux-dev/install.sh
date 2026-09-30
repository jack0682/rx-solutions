#!/bin/sh
# No sudo, host package changes, device access, or automatic robot activation.
set -eu
rx_bundle=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
exec python3 "$rx_bundle/rx-dev" install "$@"
