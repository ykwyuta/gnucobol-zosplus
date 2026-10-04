#!/usr/bin/env bash
set -euo pipefail
workspace="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
binary="$(mktemp)"
marker="$(mktemp)"
trap 'rm -f -- "$binary" "$marker"' EXIT
gcc -Wall -Wextra -Werror "$workspace/extras/jclsh/tests/step.c" -o "$binary"
set +e
JCL_PARM=ABEND JCL_ABEND_FILE="$marker" "$binary"
rc=$?
set -e
test "$rc" -eq 12
test "$(cat "$marker")" = U1234
printf 'WSL step adapter passed: ABEND=U1234\n'
