#!/bin/sh
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$root"
tmp=$(mktemp -d /tmp/imsdb-bank-inquiry.XXXXXX)
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

build/imsdb-linux/debug/imsdbctl import-csv-bundle "$tmp/bank.wal" \
    reference/Bank-of-Z-main/src/base/ims/DBD \
    reference/Bank-of-Z-main/src/base/ims/LoadData >/dev/null
gcc -std=c11 -Wall -Wextra -Werror -I. extras/imsdb/imsbatch.c \
    -Lbuild/libcob/.libs -lcob -o "$tmp/imsbatch"
gcc -std=c11 -Wall -Wextra -Werror -fPIC -shared -I. -Iextras/imsdb \
    extras/imsdb/cbltdli.c -Lbuild/imsdb-linux/debug -limsdb \
    -Lbuild/libcob/.libs -lcob -pthread -o "$tmp/libcobims.so"
export COB_PRE_LOAD="$tmp/libcobims.so"
export COB_IMS_WAL="$tmp/bank.wal"
export COB_IMS_DBD_DIR="$root/reference/Bank-of-Z-main/src/base/ims/DBD"
export COB_IMS_ENTRY=DLITCBL
export COB_IMS_DBD_NAME=CUSTOMER
export COB_IMS_ENCODING=IBM-037
export COB_IMS_AUTOCOMMIT=1
export COB_IMS_TM_DIR="$tmp/tm-queue"

for program in IBGCUDAT IBACSUM; do
    mkdir "$tmp/$program"
    build/cobc/.libs/cobc -m -fdata-codeset=IBM-037 \
        -fbinary-byteorder=big-endian -o "$tmp/$program/DLITCBL.so" \
        "reference/Bank-of-Z-main/src/base/ims/cobol/$program.cbl"
    export COB_LIBRARY_PATH="$tmp/$program"
    export COB_IMS_PSB="$root/reference/Bank-of-Z-main/src/base/ims/PSB/$program.asm"
    export COB_IMS_TM_DEST="$program"
    if [ "$program" = IBGCUDAT ]; then
        cases='000000001:TERM0001 999999999:TERM0002'
    else
        cases='000000001:TERM0003'
    fi
    for item in $cases; do
        customer=${item%%:*}
        lterm=${item#*:}
        message_hex=$(python3 - "$program" "$customer" <<'PY'
import struct
import sys

def field(text, length):
    return text.ljust(length).encode('cp037')

message = b'\0\0' + field(sys.argv[1], 8) + field(sys.argv[2], 9)
print((struct.pack('>H', len(message) + 2) + message).hex())
PY
        )
        build/imsdb-linux/debug/imsdbctl tm-enqueue \
            "$COB_IMS_TM_DIR" "$program" "$lterm" "$message_hex" >/dev/null
        "$tmp/imsbatch" > "$tmp/$lterm.log"
        build/imsdb-linux/debug/imsdbctl tm-list \
            "$COB_IMS_TM_DIR" "$lterm" > "$tmp/$lterm.output"
    done
done
python3 - "$tmp" <<'PY'
import pathlib
import sys

base = pathlib.Path(sys.argv[1])

def payload(lterm):
    rows = (base / f'{lterm}.output').read_text().splitlines()
    assert len(rows) == 1, (lterm, rows)
    return bytes.fromhex(rows[0].split()[2])

customer = payload('TERM0001')
assert 'Martha'.encode('cp037') in customer, customer.hex()
assert 'Antonelli'.encode('cp037') in customer, customer.hex()
missing = payload('TERM0002')
assert 'CUSTOMER DOES NOT EXIST'.encode('cp037') in missing, missing.hex()
summary = payload('TERM0003')
assert summary[36:37] == '2'.encode('cp037'), summary[32:40].hex()
print('Bank original IBGCUDAT customer and IBACSUM two accounts OK')
PY
