#!/usr/bin/env bash
# Fetches the pinned upstream API description for the conformance check and prints where
# it is. The file goes under target/, is checked against its SHA-256 and is never added to
# the repository. With --optional a failed download is reported and is not an error, so a
# check without a network still runs everything else.
set -euo pipefail
cd "$(dirname "$0")/.."
pin=crates/xenadmin-core/xapi-description.pin
value() { sed -n "s/^$1=//p" "$pin"; }
sha256="$(value sha256)"
url="https://raw.githubusercontent.com/$(value repository)/$(value commit)/$(value path)"
file="target/xapi-description/$sha256.json"
verified() { [[ -f "$1" ]] && [[ "$(sha256sum "$1" | cut -d' ' -f1)" == "$sha256" ]]; }
if ! verified "$file"; then
  mkdir -p "$(dirname "$file")"
  if ! curl --fail --silent --show-error --location --retry 3 --max-time 120 -o "$file.part" "$url"; then
    rm -f "$file.part"
    if [[ "${1:-}" == "--optional" ]]; then
      echo "xapi description: could not be fetched; the conformance check will be skipped" >&2
      exit 0
    fi
    echo "xapi description: could not fetch $url" >&2
    exit 1
  fi
  if ! verified "$file.part"; then
    rm -f "$file.part"
    echo "xapi description: $url does not have the pinned SHA-256" >&2
    exit 1
  fi
  mv "$file.part" "$file"
fi
echo "$PWD/$file"
