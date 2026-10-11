#!/bin/sh
# Run from WSL after building the Linux Rust library in build/imsdb-linux.
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$root"
tmp=$(mktemp -d /tmp/imsdb-cbltdli.XXXXXX)
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
build/cobc/.libs/cobc -x -fdata-codeset=IBM-037 -fbinary-byteorder=big-endian \
    -o "$tmp/ims-smoke" extras/imsdb/tests/cbltdli-smoke.cob
build/imsdb-linux/debug/imsdbctl import-csv-bundle "$tmp/bank.wal" \
    reference/Bank-of-Z-main/src/base/ims/DBD \
    reference/Bank-of-Z-main/src/base/ims/LoadData

export COB_PRE_LOAD="$tmp/libcobims.so"
export COB_IMS_WAL="$tmp/bank.wal"
export COB_IMS_DBD_DIR="$root/reference/Bank-of-Z-main/src/base/ims/DBD"
export COB_IMS_PSB="$root/reference/Bank-of-Z-main/src/base/ims/PSB/IBSCUDAT.asm"
export COB_IMS_VERIFY_ONLY=0
if result=$("$tmp/ims-smoke"); then
    :
else
    printf '%s\n' "$result"
    exit 1
fi
printf '%s\n' "$result"
test "$result" = 'IMS CUSTOMER UPDATE OK'
export COB_IMS_VERIFY_ONLY=2
if result=$("$tmp/ims-smoke"); then
    :
else
    printf '%s\n' "$result"
    exit 1
fi
printf '%s\n' "$result"
test "$result" = 'IMS CUSTOMER UNCOMMITTED OK'
export COB_IMS_VERIFY_ONLY=1
if result=$("$tmp/ims-smoke"); then
    :
else
    printf '%s\n' "$result"
    exit 1
fi
printf '%s\n' "$result"
test "$result" = 'IMS CUSTOMER REOPEN OK'
export COB_IMS_AUTOCOMMIT=1
export COB_IMS_VERIFY_ONLY=3
result=$("$tmp/ims-smoke")
printf '%s\n' "$result"
test "$result" = 'IMS CUSTOMER AUTO UPDATE OK'
export COB_IMS_VERIFY_ONLY=4
result=$("$tmp/ims-smoke")
printf '%s\n' "$result"
test "$result" = 'IMS CUSTOMER AUTO REOPEN OK'
export COB_IMS_VERIFY_ONLY=9
if result=$("$tmp/ims-smoke"); then
    printf '%s\n' 'failed batch unexpectedly returned RC 0'
    exit 1
else
    rc=$?
fi
printf '%s\n' "$result"
test "$rc" -eq 8
test "$result" = 'IMS CUSTOMER FAILED BATCH UPDATE OK'
export COB_IMS_VERIFY_ONLY=4
result=$("$tmp/ims-smoke")
printf '%s\n' "$result"
test "$result" = 'IMS CUSTOMER AUTO REOPEN OK'
export COB_IMS_VERIFY_ONLY=6
result=$("$tmp/ims-smoke")
printf '%s\n' "$result"
test "$result" = 'IMS CUSTOMER CHKP OK'
checkpoint=$(build/imsdb-linux/debug/imsdbctl latest-checkpoint "$tmp/bank.wal" \
    reference/Bank-of-Z-main/src/base/ims/DBD)
test "$checkpoint" = 'CHECK001'
export COB_IMS_VERIFY_ONLY=7
result=$("$tmp/ims-smoke")
printf '%s\n' "$result"
test "$result" = 'IMS CUSTOMER CHECKPOINT REOPEN OK'
export COB_IMS_VERIFY_ONLY=8
result=$("$tmp/ims-smoke")
printf '%s\n' "$result"
test "$result" = 'IMS CUSTOMER ROLB OK'
export COB_IMS_VERIFY_ONLY=7
result=$("$tmp/ims-smoke")
printf '%s\n' "$result"
test "$result" = 'IMS CUSTOMER CHECKPOINT REOPEN OK'
