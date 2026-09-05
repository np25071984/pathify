#!/usr/bin/env bash
# Fail if a crate capable of network I/O enters the dependency graph.
#
# "Nothing is ever uploaded anywhere" is Pathify's central promise, and the only
# way to keep it true over time is to make it a build failure rather than a
# habit. This runs in CI and is worth running locally before adding a dependency.
set -euo pipefail

LOCKFILE="${1:-Cargo.lock}"

# Crates whose whole purpose is talking to a network, plus the async runtimes
# and TLS stacks that carry them in. Matched against `name = "..."` lines in
# Cargo.lock, so this catches transitive dependencies too.
BANNED=(
  reqwest hyper hyper-util h2 curl curl-sys ureq attohttpc isahc surf
  tokio async-std smol
  native-tls openssl openssl-sys rustls rustls-webpki webpki tokio-rustls
  socket2 mio-extras trust-dns-resolver hickory-resolver
  ws url-fetch tungstenite tokio-tungstenite
)

if [[ ! -f "$LOCKFILE" ]]; then
  echo "error: $LOCKFILE not found; run cargo build first" >&2
  exit 2
fi

found=()
for crate in "${BANNED[@]}"; do
  if grep -q "^name = \"${crate}\"$" "$LOCKFILE"; then
    found+=("$crate")
  fi
done

if (( ${#found[@]} > 0 )); then
  echo "error: network-capable crates found in the dependency graph:" >&2
  printf '  - %s\n' "${found[@]}" >&2
  cat >&2 <<'EOF'

Pathify is local-first: it must not be able to make a network request at all.
If a dependency you are adding pulls one of these in, find another crate or
disable the feature that pulls it. If a crate on this list is a genuine false
positive (it is vendored for a non-network purpose), remove it from the list in
this script in the same commit, with a note explaining why.
EOF
  exit 1
fi

echo "ok: no network-capable crates in $LOCKFILE"
