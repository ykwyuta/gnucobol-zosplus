#!/bin/sh
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$root"
tmp=$(mktemp -d /tmp/imsdb-carddemo-dli.XXXXXX)
if [ "${IMS_KEEP_TMP:-0}" != 1 ]; then
    trap 'rm -rf -- "$tmp"' EXIT
else
    printf 'IMS test files: %s\n' "$tmp"
fi
export LD_LIBRARY_PATH="$root/build/libcob/.libs:$root/build/imsdb-linux/debug${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
export COB_CONFIG_DIR="$root/config"
export COB_COPY_DIR="$root/copy"
export COB_CFLAGS="-I$root"
export COB_LDFLAGS="-L$root/build/libcob/.libs"
export PATH="$root/bin:$PATH"
export COB_IMS_DLI_PREPROCESSOR="$root/build/preprocessors-linux/debug/imsdli-preprocess"
python3 extras/imsdb/tests/test_execdli.py
card="reference/aws-mainframe-modernization-carddemo/app/app-authorization-ims-db2-mq"
build/cobc/.libs/cobc --ims-dli=DBPAUTP0 -m -I "$card/cpy" \
    -o "$tmp/CBPAUP0C.so" "$card/cbl/CBPAUP0C.cbl"
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
printf '05 00005 00010 N\n' | "$tmp/imsbatch" > "$tmp/run.log"
python3 - "$tmp/run.log" <<'PY'
import pathlib
import sys

log = pathlib.Path(sys.argv[1]).read_text(errors='replace')
assert 'STARTING PROGRAM CBPAUP0C' in log, log
assert '# TOTAL SUMMARY READ' in log, log
assert 'CHKP FAILED' not in log, log
print('IMS CARDDEMO EXEC DLI BATCH OK')
PY
cp "$tmp/CBPAUP0C.so" "$tmp/imsbatch" "$tmp/libcobims.so" \
    build/imsdb-carddemo/
: > build/imsdb-carddemo/empty-dd
rm -f build/imsdb-carddemo/card.wal
