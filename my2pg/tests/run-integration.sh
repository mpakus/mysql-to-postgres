#!/bin/sh
set -eu
exec python3 "$(dirname "$0")/support/harness.py" "$@"
