#!/bin/sh
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$root"
out=build/imsdb-carddemo-jcl
card=reference/aws-mainframe-modernization-carddemo/app/app-authorization-ims-db2-mq
mkdir -p "$out/proclib"
: > "$out/empty-dd"
: > "$out/proclib/DFSVSMDB"
: > "$out/roots.fb"
: > "$out/children.fb"
rm -f "$out/card.wal"
rm -f "$out/load.wal"
export LD_LIBRARY_PATH="$root/build/libcob/.libs:$root/build/imsdb-linux/debug${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
export COB_CONFIG_DIR="$root/config"
export COB_COPY_DIR="$root/copy"
export COB_CFLAGS="-I$root"
export COB_LDFLAGS="-L$root/build/libcob/.libs"
build/imsdb-linux/debug/imsdbctl import-dfsurgu0 "$out/card.wal" \
    "$card/ims/DBPAUTP0.dbd" \
    "$card/data/EBCDIC/AWS.M2.CARDDEMO.IMSDATA.DBPAUTP0.dat"
build/imsdb-linux/debug/imsdbctl split-dfsurgu0 \
    "$card/ims/DBPAUTP0.dbd" \
    "$card/data/EBCDIC/AWS.M2.CARDDEMO.IMSDATA.DBPAUTP0.dat" \
    "$out/expected-roots.fb" "$out/expected-children.fb"
build/cobc/.libs/cobc -m -fdata-codeset=IBM-037 \
    -fbinary-byteorder=big-endian -I "$card/cpy" \
    -o "$out/PAUDBUNL.so" "$card/cbl/PAUDBUNL.CBL"
build/cobc/.libs/cobc -m -fdata-codeset=IBM-037 \
    -fbinary-byteorder=big-endian -I "$card/cpy" \
    -o "$out/PAUDBLOD.so" "$card/cbl/PAUDBLOD.CBL"
gcc -std=c11 -Wall -Wextra -Werror -I. extras/imsdb/imsbatch.c \
    -Lbuild/libcob/.libs -lcob -o "$out/imsbatch"
gcc -std=c11 -Wall -Wextra -Werror -fPIC -shared -I. -Iextras/imsdb \
    extras/imsdb/cbltdli.c -Lbuild/imsdb-linux/debug -limsdb \
    -Lbuild/libcob/.libs -lcob -pthread -o "$out/libcobims.so"
