#!/bin/sh
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$root"
tmp=$(mktemp -d /tmp/imsdb-carddemo-unload.XXXXXX)
if [ "${IMS_KEEP_TMP:-0}" != 1 ]; then
    trap 'rm -rf -- "$tmp"' EXIT
else
    printf 'IMS test files: %s\n' "$tmp"
fi
card=reference/aws-mainframe-modernization-carddemo/app/app-authorization-ims-db2-mq
export LD_LIBRARY_PATH="$root/build/libcob/.libs:$root/build/imsdb-linux/debug${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
export COB_CONFIG_DIR="$root/config"
export COB_COPY_DIR="$root/copy"
export COB_CFLAGS="-I$root"
export COB_LDFLAGS="-L$root/build/libcob/.libs"
export PATH="$root/bin:$PATH"
build/imsdb-linux/debug/imsdbctl import-dfsurgu0 "$tmp/card.wal" \
    "$card/ims/DBPAUTP0.dbd" \
    "$card/data/EBCDIC/AWS.M2.CARDDEMO.IMSDATA.DBPAUTP0.dat"
build/cobc/.libs/cobc --ims-dli=DBPAUTP0 -m \
    -fdata-codeset=IBM-037 -fbinary-byteorder=big-endian -I "$card/cpy" \
    -o "$tmp/CBPAUP0C.so" "$card/cbl/CBPAUP0C.cbl"
build/cobc/.libs/cobc -x -fdata-codeset=IBM-037 \
    -fbinary-byteorder=big-endian \
    -o "$tmp/ims-verify" extras/imsdb/tests/carddemo-verify.cob
gcc -std=c11 -Wall -Wextra -Werror -I. extras/imsdb/imsbatch.c \
    -Lbuild/libcob/.libs -lcob -o "$tmp/imsbatch"
gcc -std=c11 -Wall -Wextra -Werror -fPIC -shared -I. -Iextras/imsdb \
    extras/imsdb/cbltdli.c -Lbuild/imsdb-linux/debug -limsdb \
    -Lbuild/libcob/.libs -lcob -pthread -o "$tmp/libcobims.so"
export COB_PRE_LOAD="$tmp/libcobims.so"
export COB_IMS_WAL="$tmp/card.wal"
export COB_IMS_DBD_DIR="$root/$card/ims"
export COB_IMS_PSB="$root/$card/ims/PSBPAUTB.PSB"
export COB_IMS_AUTOCOMMIT=1
export COB_LIBRARY_PATH="$tmp"
export COB_IMS_ENTRY=CBPAUP0C
export COB_IMS_DBD_NAME=DBPAUTP0
export COB_IMS_ENCODING=IBM-037
if printf '05 00005 00010 N\n' | "$tmp/imsbatch" > "$tmp/run.log"; then
    echo 'CBPAUP0C unexpectedly accepted DLET after GNP' >&2
    exit 1
fi
grep -q 'AUTH SUMMARY DELETE FAILED :DJ' "$tmp/run.log"
"$tmp/ims-verify"
printf 'IMS CARDDEMO ORIGINAL DATA ROLLBACK OK\n'
