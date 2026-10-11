#!/usr/bin/env bash
set -euo pipefail

group_name="fearless-conspiracy"
runtime_dir="/run/fearless-conspiracy"
socket_path="${1:-}"
failed=0

pass() {
  printf 'PASS: %s\n' "$1"
}

fail() {
  printf 'FAIL: %s\n' "$1" >&2
  failed=1
}

if getent group "$group_name" >/dev/null; then
  pass "group $group_name exists"
else
  fail "group $group_name is missing"
fi

for account in fearless_host fearless_p{1..9}; do
  if ! id "$account" >/dev/null 2>&1; then
    fail "account $account is missing"
    continue
  fi
  if id -nG "$account" | tr ' ' '\n' | grep -Fxq "$group_name"; then
    pass "$account belongs to $group_name"
  else
    fail "$account does not belong to $group_name"
  fi
done

if [[ -d "$runtime_dir" ]]; then
  runtime_owner="$(stat -c '%U' "$runtime_dir")"
  runtime_group="$(stat -c '%G' "$runtime_dir")"
  runtime_mode="$(stat -c '%a' "$runtime_dir")"
  [[ "$runtime_owner" == "fearless_host" ]] \
    && pass "$runtime_dir owner is fearless_host" \
    || fail "$runtime_dir owner is $runtime_owner, expected fearless_host"
  [[ "$runtime_group" == "$group_name" ]] \
    && pass "$runtime_dir group is $group_name" \
    || fail "$runtime_dir group is $runtime_group, expected $group_name"
  [[ "$runtime_mode" == "2750" ]] \
    && pass "$runtime_dir mode is 2750" \
    || fail "$runtime_dir mode is $runtime_mode, expected 2750"
else
  fail "$runtime_dir is missing"
fi

for binary in ipc-demo-server ipc-demo-client fearless-conspiracy-server fearless-conspiracy-client; do
  path="/usr/local/bin/$binary"
  if [[ ! -f "$path" ]]; then
    fail "$path is missing"
    continue
  fi
  binary_owner="$(stat -c '%U' "$path")"
  binary_group="$(stat -c '%G' "$path")"
  binary_mode="$(stat -c '%a' "$path")"
  [[ "$binary_owner:$binary_group:$binary_mode" == "root:$group_name:550" ]] \
    && pass "$path is root:$group_name mode 0550" \
    || fail "$path is $binary_owner:$binary_group mode $binary_mode; expected root:$group_name mode 0550"
done

if [[ -n "$socket_path" ]]; then
  if [[ ! -S "$socket_path" ]]; then
    fail "$socket_path is not a Unix socket"
  else
    socket_owner="$(stat -c '%U' "$socket_path")"
    socket_group="$(stat -c '%G' "$socket_path")"
    socket_mode="$(stat -c '%a' "$socket_path")"
    [[ "$socket_owner" == "fearless_host" ]] \
      && pass "$socket_path owner is fearless_host" \
      || fail "$socket_path owner is $socket_owner, expected fearless_host"
    [[ "$socket_group" == "$group_name" ]] \
      && pass "$socket_path inherited group $group_name" \
      || fail "$socket_path group is $socket_group, expected $group_name"
    [[ "$socket_mode" == "660" ]] \
      && pass "$socket_path mode is 0660" \
      || fail "$socket_path mode is $socket_mode, expected 0660"
  fi
fi

if (( failed )); then
  printf 'Preflight failed. Correct the reported account or permission problems and retry.\n' >&2
  exit 1
fi

printf 'Multi-user preflight passed.\n'
