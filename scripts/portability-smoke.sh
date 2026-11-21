#!/usr/bin/env bash
# Verify the publishable workspace archives against real registry dependencies.
# Cargo prepares sibling archives in dependency order and verifies them through
# its temporary registry, without uploads or a copied checkout/build cache.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"
if (($# != 0)); then
  echo "usage: scripts/portability-smoke.sh (verifies the current checkout)" >&2
  exit 2
fi

excluded="$(cargo metadata --locked --no-deps --format-version 1 | python3 -c '
import json, sys
metadata = json.load(sys.stdin)
members = set(metadata["workspace_members"])
for package in metadata["packages"]:
    if package["id"] in members and package.get("publish") == []:
        print(package["name"])
')"
exclude_args=()
while IFS= read -r package; do
  if [[ -n "$package" ]]; then
    exclude_args+=(--exclude "$package")
  fi
done <<<"$excluded"
# Exclude explicitly: --workspace includes publish=false members, even with -p.
# Keep verification enabled; external SDK/semantic crates must really exist.
cargo package --workspace --locked "${exclude_args[@]}"
