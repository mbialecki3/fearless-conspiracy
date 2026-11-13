#!/usr/bin/env bash
set -euo pipefail

usage() {
  printf 'Usage: %s REPORT.pdf POSTER.pdf [OUTPUT.tgz]\n' "$0" >&2
  exit 2
}
[[ $# -ge 2 && $# -le 3 ]] || usage
project_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
report_path="$1"
poster_path="$2"
output_path="${3:-fearless-conspiracy-submission.tgz}"
for document in "$report_path" "$poster_path"; do
  [[ -f "$document" && "${document,,}" == *.pdf ]] || {
    printf 'A PDF is required: %s\n' "$document" >&2; exit 1;
  }
  [[ "$(head -c 5 "$document")" == '%PDF-' ]] || {
    printf 'Invalid PDF header: %s\n' "$document" >&2; exit 1;
  }
done
[[ "${output_path,,}" == *.tgz && ! -e "$output_path" ]] || {
  printf 'Output must be a new .tgz path: %s\n' "$output_path" >&2; exit 1;
}
package_root="$(mktemp -d /tmp/fearless-conspiracy-package.XXXXXX)"
cleanup() {
  case "$package_root" in
    /tmp/fearless-conspiracy-package.*) rm -r -- "$package_root" ;;
  esac
}
trap cleanup EXIT
staged="$package_root/fearless-conspiracy"
mkdir -p "$staged/scripts"
# Copy the current reviewed sources, including uncommitted fixes. Exclude old
# submissions, build trees, credentials, and unrelated personal draft files.
for path in Cargo.toml Cargo.lock rust-toolchain.toml README.md install server client; do
  cp -- "$project_root/$path" "$staged/"
done
cp -R -- "$project_root/src" "$staged/src"
for script in protected-server.py test-protected-server.py test-server-launcher.py protected-server-guest.py \
              proc-visibility-guest.py run-proc-visibility-vm.py check.sh smoke-game.sh \
              package-release.sh package-submission.sh; do
  cp -- "$project_root/scripts/$script" "$staged/scripts/"
done
cp -- "$report_path" "$staged/Fearless-Conspiracy-Report.pdf"
cp -- "$poster_path" "$staged/Fearless-Conspiracy-Poster.pdf"
if [[ -d "$project_root/docs/evidence" ]]; then
  cp -R -- "$project_root/docs/evidence" "$staged/evidence"
fi
pdf_count="$(find "$staged" -type f -iname '*.pdf' | wc -l)"
[[ "$pdf_count" -eq 2 ]] || { printf 'Expected two PDFs, found %s\n' "$pdf_count" >&2; exit 1; }
if find "$staged" -type f \( -iname '*.doc' -o -iname '*.docx' -o -iname '*.odt' \) | grep -q .; then
  printf 'Unexpected word-processing document in submission.\n' >&2; exit 1
fi
# Hash the staged content so the exact submitted sources can be identified.
(cd "$staged" && find . -type f -print0 | sort -z | xargs -0 sha256sum > "$package_root/SHA256SUMS")
cp -- "$package_root/SHA256SUMS" "$staged/SHA256SUMS"
tar -czf "$output_path" -C "$package_root" fearless-conspiracy
printf 'Created %s with two PDFs and current game sources.\n' "$output_path"
