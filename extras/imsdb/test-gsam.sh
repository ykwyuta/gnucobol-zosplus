#!/bin/sh
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$root"
tmp=$(mktemp -d /tmp/imsdb-gsam.XXXXXX)
trap 'rm -rf -- "$tmp"' EXIT
export LD_LIBRARY_PATH="$root/build/libcob/.libs:$root/build/imsdb-linux/debug${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
export COB_CONFIG_DIR="$root/config"
export COB_COPY_DIR="$root/copy"
export COB_CFLAGS="-I$root"
export COB_LDFLAGS="-L$root/build/libcob/.libs"
gcc -std=c11 -Wall -Wextra -Werror -fPIC -shared \
    -I. -Iextras/imsdb extras/imsdb/cbltdli.c \
    -Lbuild/imsdb-linux/debug -limsdb -Lbuild/libcob/.libs -lcob -pthread \
    -o "$tmp/libcobims.so"
build/cobc/.libs/cobc -x -o "$tmp/ims-gsam" extras/imsdb/tests/gsam-smoke.cob
printf '%-100s%-100s' FIRST SECOND > "$tmp/input.fb"
export DD_PASFILIP="$tmp/input.fb"
export DD_PASFILOP="$tmp/output.fb"
export COB_PRE_LOAD="$tmp/libcobims.so"
export COB_IMS_WAL="$tmp/gsam.wal"
export COB_IMS_DBD_DIR="$root/reference/aws-mainframe-modernization-carddemo/app/app-authorization-ims-db2-mq/ims"
export COB_IMS_PSB="$COB_IMS_DBD_DIR/DLIGSAMP.PSB"
result=$("$tmp/ims-gsam")
printf '%s\n' "$result"
test "$result" = 'IMS GSAM OK'
test "$(wc -c < "$tmp/output.fb")" -eq 100
test "$(dd if="$tmp/output.fb" bs=1 count=5 2>/dev/null)" = THIRD
