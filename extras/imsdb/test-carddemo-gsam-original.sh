#!/bin/sh
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$root"
tmp=$(mktemp -d /tmp/imsdb-carddemo-gsam-original.XXXXXX)
trap 'rm -rf -- "$tmp"' EXIT
card=reference/aws-mainframe-modernization-carddemo/app/app-authorization-ims-db2-mq
export LD_LIBRARY_PATH="$root/build/libcob/.libs:$root/build/imsdb-linux/debug${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
export COB_CONFIG_DIR="$root/config"
export COB_COPY_DIR="$root/copy"
export COB_CFLAGS="-I$root"
export COB_LDFLAGS="-L$root/build/libcob/.libs"

build/imsdb-linux/debug/imsdbctl import-dfsurgu0 \
    "$tmp/card.wal" "$card/ims/DBPAUTP0.dbd" \
    "$card/data/EBCDIC/AWS.M2.CARDDEMO.IMSDATA.DBPAUTP0.dat"
build/cobc/.libs/cobc -m -fdata-codeset=IBM-037 \
    -fbinary-byteorder=big-endian -I "$card/cpy" \
    -o "$tmp/DBUNLDGS.so" "$card/cbl/DBUNLDGS.CBL"
gcc -std=c11 -Wall -Wextra -Werror -I. extras/imsdb/imsbatch.c \
    -Lbuild/libcob/.libs -lcob -o "$tmp/imsbatch"
gcc -std=c11 -Wall -Wextra -Werror -fPIC -shared -I. -Iextras/imsdb \
    extras/imsdb/cbltdli.c -Lbuild/imsdb-linux/debug -limsdb \
    -Lbuild/libcob/.libs -lcob -pthread -o "$tmp/libcobims.so"
export COB_PRE_LOAD="$tmp/libcobims.so"
export COB_LIBRARY_PATH="$tmp"
export COB_IMS_WAL="$tmp/card.wal"
export COB_IMS_DBD_DIR="$root/$card/ims"
export COB_IMS_PSB="$root/$card/ims/DLIGSAMP.PSB"
export COB_IMS_LINKAGE=DB
export COB_IMS_ENTRY=DBUNLDGS
export COB_IMS_ENCODING=IBM-037
export DD_PASFILOP="$tmp/parents.fb"
export DD_PADFILOP="$tmp/children.fb"
if ! "$tmp/imsbatch" > "$tmp/run.log" 2>&1; then
    cat "$tmp/run.log"
    exit 1
fi
python3 - "$tmp/parents.fb" "$tmp/children.fb" <<'PY'
import pathlib
import sys

parents = pathlib.Path(sys.argv[1]).read_bytes()
children = pathlib.Path(sys.argv[2]).read_bytes()
assert len(parents) == 21 * 100, (len(parents), parents[:32].hex())
assert len(children) == 202 * 200, (len(children), children[:32].hex())
print("CardDemo original DBUNLDGS GSAM 21 parents 202 children OK")
PY
