#!/usr/bin/env bash
set -euo pipefail

usage() {
  printf 'Usage: %s REPORT.pdf [OUTPUT.tgz]\n' "$0" >&2
  exit 2
}

[[ $# -ge 1 && $# -le 2 ]] || usage
report_path="$1"
output_path="${2:-terminal-salem-submission.tgz}"

[[ -f "$report_path" ]] || {
  printf 'Report does not exist: %s\n' "$report_path" >&2
  exit 1
}
[[ "${report_path,,}" == *.pdf ]] || {
  printf 'Report must have a .pdf extension: %s\n' "$report_path" >&2
  exit 1
}
[[ "${output_path,,}" == *.tgz ]] || {
  printf 'Output must have a .tgz extension: %s\n' "$output_path" >&2
  exit 1
}
[[ ! -e "$output_path" ]] || {
  printf 'Refusing to overwrite existing output: %s\n' "$output_path" >&2
  exit 1
}

if [[ "$(head -c 5 "$report_path")" != "%PDF-" ]]; then
  printf 'Report does not begin with a PDF header: %s\n' "$report_path" >&2
  exit 1
fi

if ! git diff --quiet || ! git diff --cached --quiet; then
  printf 'Commit all tracked worktree changes before packaging.\n' >&2
  git status --short >&2
  exit 1
fi

package_root="$(mktemp -d /tmp/terminal-salem-package.XXXXXX)"
cleanup() {
  case "$package_root" in
    /tmp/terminal-salem-package.*) rm -r -- "$package_root" ;;
  esac
}
trap cleanup EXIT

git archive --format=tar --prefix=terminal-salem/ HEAD \
  | tar -xf - -C "$package_root"
cp -- "$report_path" "$package_root/terminal-salem/Terminal-Salem-Report.pdf"

if find "$package_root/terminal-salem" -type f \
  \( -iname '*.doc' -o -iname '*.docx' -o -iname '*.odt' -o -iname '*.pages' \) \
  | grep -q .; then
  printf 'Word-processing document found; only the PDF report may be submitted.\n' >&2
  exit 1
fi

pdf_count="$(find "$package_root/terminal-salem" -type f -iname '*.pdf' | wc -l)"
[[ "$pdf_count" -eq 1 ]] || {
  printf 'Expected exactly one PDF, found %s.\n' "$pdf_count" >&2
  exit 1
}

tar -czf "$output_path" -C "$package_root" terminal-salem
tar -tzf "$output_path"
printf 'Created %s from commit %s\n' "$output_path" "$(git rev-parse --short HEAD)"
