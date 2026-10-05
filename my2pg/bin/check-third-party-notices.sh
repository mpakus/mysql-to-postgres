#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
mode=${1:---check}
about=${CARGO_ABOUT:-cargo-about}
expected_version='cargo-about 0.8.4'

if [ "$mode" != --check ] && [ "$mode" != --write ]; then
    echo "usage: $0 [--check|--write]" >&2
    exit 2
fi
if [ "$("$about" --version)" != "$expected_version" ]; then
    echo "expected $expected_version (set CARGO_ABOUT to its path if needed)" >&2
    exit 1
fi

tmp=$(mktemp "${TMPDIR:-/tmp}/my2pg-third-party-notices.XXXXXX")
trap 'rm -f "$tmp"' EXIT HUP INT TERM
"$about" generate \
    --config "$root/about.toml" \
    --manifest-path "$root/Cargo.toml" \
    --locked --offline --fail --all-features \
    "$root/third-party-notices.hbs" >"$tmp"

if [ "$mode" = --write ]; then
    cp "$tmp" "$root/THIRD-PARTY-NOTICES.md"
else
    if ! cmp -s "$tmp" "$root/THIRD-PARTY-NOTICES.md"; then
        echo "THIRD-PARTY-NOTICES.md is stale; run $0 --write" >&2
        exit 1
    fi
fi

if ! cargo package --list --allow-dirty --no-verify --locked --offline \
    | grep -Fqx 'THIRD-PARTY-NOTICES.md'; then
    echo 'THIRD-PARTY-NOTICES.md is missing from the Cargo source package' >&2
    exit 1
fi
