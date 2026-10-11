#!/bin/sh
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$root"
tmp=$(mktemp -d /tmp/imsdb-cics-uow.XXXXXX)
trap 'rm -rf -- "$tmp"' EXIT
export LD_LIBRARY_PATH="$root/build/libcob/.libs:$root/build/imsdb-linux/debug${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
export COB_CONFIG_DIR="$root/config"
export COB_COPY_DIR="$root/copy"
export COB_CFLAGS="-I$root"
export COB_LDFLAGS="-L$root/build/libcob/.libs -L$root/build/imsdb-linux/debug -limsdb -pthread"
build/cobc/.libs/cobc -x -o "$tmp/ims-ssa-native" \
    extras/imsdb/tests/execdli-ssa-smoke.cob extras/imsdb/cbltdli.c
"$tmp/ims-ssa-native"
build/cobc/.libs/cobc -x -fdata-codeset=IBM-037 \
    -o "$tmp/ims-ssa-ebcdic" \
    extras/imsdb/tests/execdli-ssa-smoke.cob extras/imsdb/cbltdli.c
"$tmp/ims-ssa-ebcdic"
build/cobc/.libs/cobc -x -fdata-codeset=IBM-037 \
    -fbinary-byteorder=big-endian -o "$tmp/ims-cics-uow" \
    extras/imsdb/tests/cics-uow-smoke.cob \
    extras/cobcics/test-ims-uow-real.c extras/imsdb/cbltdli.c
build/imsdb-linux/debug/imsdbctl import-csv-bundle "$tmp/bank.wal" \
    reference/Bank-of-Z-main/src/base/ims/DBD \
    reference/Bank-of-Z-main/src/base/ims/LoadData
export COB_IMS_WAL="$tmp/bank.wal"
export COB_IMS_DBD_DIR="$root/reference/Bank-of-Z-main/src/base/ims/DBD"
export COB_IMS_PSB="$root/reference/Bank-of-Z-main/src/base/ims/PSB/IBSCUDAT.asm"
unset COB_IMS_AUTOCOMMIT
COB_IMS_MODE=C "$tmp/ims-cics-uow"
COB_IMS_MODE=R "$tmp/ims-cics-uow"
COB_IMS_MODE=V "$tmp/ims-cics-uow"
