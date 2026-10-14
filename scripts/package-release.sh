#!/usr/bin/env bash
set -euo pipefail

if [[ $# != 1 || ! "$1" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  printf 'Usage: %s VERSION (for example, 0.1.0)\n' "$0" >&2
  exit 2
fi

project_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$project_root"
if ! git diff --quiet || ! git diff --cached --quiet; then
  printf 'Commit tracked changes before packaging a release.\n' >&2
  exit 1
fi
version="$1"
release_target=x86_64-unknown-linux-musl
package_name="fearless-conspiracy-$version-linux-x86_64"
output_dir="$project_root/target/releases"
archive="$output_dir/$package_name.tar.gz"

if [[ -e "$archive" || -e "$archive.sha256" ]]; then
  printf 'Release output already exists: %s\n' "$archive" >&2
  exit 1
fi

rustup target add "$release_target"
cargo build --locked --release --bins --target "$release_target"

package_root="$(mktemp -d)"
trap 'rm -rf -- "$package_root"' EXIT
mkdir -p "$package_root/$package_name/bin" "$output_dir"
cp -- server client README.md "$package_root/$package_name/"
for binary in fearless-conspiracy-server fearless-conspiracy-client ipc-demo-server ipc-demo-client; do
  cp -- "target/$release_target/release/$binary" "$package_root/$package_name/bin/"
done
{
  printf 'Version: %s\nTarget: %s\n' "$version" "$release_target"
  printf 'Source commit: %s\n' "$(git rev-parse HEAD)"
  rustc --version
} > "$package_root/$package_name/BUILD-INFO.txt"

tar -czf "$archive" -C "$package_root" "$package_name"
(cd "$output_dir" && sha256sum "$package_name.tar.gz" > "$package_name.tar.gz.sha256")
printf 'Created %s and its SHA-256 checksum.\n' "$archive"
