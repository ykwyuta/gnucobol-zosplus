#!/bin/sh
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$root"
tmp=$(mktemp -d /tmp/imsdb-carddemo-roundtrip.XXXXXX)
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
build/imsdb-linux/debug/imsdbctl import-dfsurgu0 "$tmp/card.wal" \
    "$card/ims/DBPAUTP0.dbd" \
    "$card/data/EBCDIC/AWS.M2.CARDDEMO.IMSDATA.DBPAUTP0.dat"
build/imsdb-linux/debug/imsdbctl split-dfsurgu0 \
    "$card/ims/DBPAUTP0.dbd" \
    "$card/data/EBCDIC/AWS.M2.CARDDEMO.IMSDATA.DBPAUTP0.dat" \
    "$tmp/expected-roots.fb" "$tmp/expected-children.fb"
build/cobc/.libs/cobc -m -fdata-codeset=IBM-037 \
    -fbinary-byteorder=big-endian -I "$card/cpy" \
    -o "$tmp/PAUDBUNL.so" "$card/cbl/PAUDBUNL.CBL"
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
export COB_IMS_ENTRY=PAUDBUNL
export COB_IMS_DBD_NAME=DBPAUTP0
export COB_IMS_ENCODING=IBM-037
export COB_IMS_LINKAGE=DB
export DD_OUTFIL1="$tmp/roots.fb"
export DD_OUTFIL2="$tmp/children.fb"
"$tmp/imsbatch" > "$tmp/unload.log"
python3 - "$tmp" <<'PY'
from pathlib import Path
import sys

folder = Path(sys.argv[1])
expected_roots = (folder / 'expected-roots.fb').read_bytes()
actual_roots = (folder / 'roots.fb').read_bytes()
expected_children = (folder / 'expected-children.fb').read_bytes()
actual_children = (folder / 'children.fb').read_bytes()
assert len(expected_roots) == 22 * 100
assert expected_roots[-100:-94] == b'\x40' * 6  # invalid packed key in source
assert actual_roots == expected_roots[:-100]  # PAUDBUNL tests IS NUMERIC
assert len(actual_children) == 202 * 206
assert actual_children == expected_children
print('IMS CARDDEMO ORIGINAL UNLOAD 21 ROOTS 202 CHILDREN OK')
PY
