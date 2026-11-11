#!/bin/sh
set -eu

usage() {
	printf 'Usage: %s KEY_FILE\nCreates a persistent media signing key if absent.\n' "$0"
}

if [ "${1:-}" = '--help' ]; then
	usage
	exit 0
fi
if [ "$#" -ne 1 ]; then
	usage >&2
	exit 2
fi

key=$1
if [ -e "$key" ]; then
	if [ ! -f "$key" ] || [ "$(wc -c < "$key")" -lt 32 ]; then
		printf 'Invalid media key: %s (expected at least 32 bytes)\n' "$key" >&2
		exit 1
	fi
	exit 0
fi

umask 077
tmp=$(mktemp "${key}.XXXXXX")
trap 'rm -f "$tmp"' EXIT HUP INT TERM
openssl rand -hex 32 > "$tmp"
# A hard link creates the key only if another setup has not created it first.
if ! ln "$tmp" "$key" 2>/dev/null; then
	if [ ! -f "$key" ] || [ "$(wc -c < "$key")" -lt 32 ]; then
		printf 'Unable to create a valid media key: %s\n' "$key" >&2
		exit 1
	fi
fi
