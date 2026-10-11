#!/bin/sh
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$root"
tmp=$(mktemp -d /tmp/imsdb-carddemo-load.XXXXXX)
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
build/imsdb-linux/debug/imsdbctl split-dfsurgu0 \
    "$card/ims/DBPAUTP0.dbd" \
    "$card/data/EBCDIC/AWS.M2.CARDDEMO.IMSDATA.DBPAUTP0.dat" \
    "$tmp/roots.fb" "$tmp/children.fb"
build/cobc/.libs/cobc -m -fdata-codeset=IBM-037 \
    -fbinary-byteorder=big-endian -I "$card/cpy" \
    -o "$tmp/PAUDBLOD.so" "$card/cbl/PAUDBLOD.CBL"
build/cobc/.libs/cobc -x -fdata-codeset=IBM-037 \
    -fbinary-byteorder=big-endian \
    -o "$tmp/ims-verify" extras/imsdb/tests/carddemo-verify.cob
gcc -std=c11 -Wall -Wextra -Werror -I. extras/imsdb/imsbatch.c \
    -Lbuild/libcob/.libs -lcob -o "$tmp/imsbatch"
gcc -std=c11 -Wall -Wextra -Werror -fPIC -shared -I. -Iextras/imsdb \
    extras/imsdb/cbltdli.c -Lbuild/imsdb-linux/debug -limsdb \
    -Lbuild/libcob/.libs -lcob -pthread -o "$tmp/libcobims.so"
export COB_PRE_LOAD="$tmp/libcobims.so"
export COB_LIBRARY_PATH="$tmp"
export COB_IMS_WAL="$tmp/card.wal"
export COB_IMS_DBD_DIR="$root/$card/ims"
export COB_IMS_PSB="$root/$card/ims/PSBPAUTB.PSB"
export COB_IMS_AUTOCOMMIT=1
export COB_IMS_ENTRY=PAUDBLOD
export COB_IMS_DBD_NAME=DBPAUTP0
export COB_IMS_ENCODING=IBM-037
export DD_INFILE1="$tmp/roots.fb"
export DD_INFILE2="$tmp/children.fb"
"$tmp/imsbatch" > "$tmp/load.log"
"$tmp/ims-verify"
