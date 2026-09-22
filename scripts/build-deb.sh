#!/bin/sh
set -eu

architecture=${1:-amd64}
case "$architecture" in
    amd64|arm64) ;;
    *) echo 'Usage: ./scripts/build-deb.sh [amd64|arm64]' >&2; exit 2 ;;
esac
if [ "$#" -gt 1 ]; then
    echo 'Usage: ./scripts/build-deb.sh [amd64|arm64]' >&2
    exit 2
fi

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
command -v docker >/dev/null 2>&1 || { echo 'Docker is required.' >&2; exit 1; }
mkdir -p "$root/build"
context=$(mktemp -d "$root/build/deb-context.XXXXXX")
trap 'rm -rf "$context"' EXIT HUP INT TERM

# Send only build inputs to Docker, excluding .git, target, and local artifacts.
cp "$root/Cargo.toml" "$root/Cargo.lock" "$root/VERSION" "$root/build.rs" \
    "$root/README.md" "$root/INSTALL.md" "$root/LICENSE" "$context/"
cp -R "$root/src" "$root/config" "$root/packaging" "$context/"
cp -R "$root/packaging/debian" "$context/debian"
version=$(tr -d '\r\n' < "$root/VERSION")
case "$version" in
    ''|*[!0-9.]*) echo 'Invalid VERSION.' >&2; exit 1 ;;
esac
cat > "$context/debian/changelog" <<EOF
autobricks-dns ($version-1) jammy; urgency=medium

  * Package Autobricks DNS for Ubuntu 22.04 amd64 and arm64.

 -- Autobricks, Co. <paulcho@users.noreply.github.com>  Wed, 23 Sep 2026 00:00:00 +0000
EOF

docker build --platform "linux/$architecture" \
    --file "$context/packaging/Dockerfile" \
    --output "type=local,dest=$root/build" "$context"
(
    cd "$root/build"
    for arch in amd64 arm64; do
        package="autobricks-dns_${version}-1_${arch}.deb"
        if [ -f "$package" ]; then
            shasum -a 256 "$package"
        fi
    done > "$context/SHA256SUMS"
)
mv "$context/SHA256SUMS" "$root/build/SHA256SUMS"
echo "Package: $root/build/autobricks-dns_${version}-1_${architecture}.deb"
