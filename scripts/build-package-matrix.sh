#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"

for ubuntu_version in 22.04 24.04; do
    for architecture in amd64 arm64; do
        ./scripts/build-deb.sh "$ubuntu_version" "$architecture"
    done
done

version=$(tr -d '\r\n' < VERSION)
echo "Built Autobricks DNS package matrix $version"
find build -maxdepth 1 -type f -name "autobricks-dns-${version}-ubuntu-*.deb" -print | sort
