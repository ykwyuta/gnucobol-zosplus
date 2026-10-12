#!/bin/sh
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$root"
tmp=$(mktemp -d /tmp/imsdb-bank-original.XXXXXX)
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
gcc -std=c11 -Wall -Wextra -Werror -fPIC -shared -I. -Iextras/imsdb \
    extras/imsdb/cbltdli.c -Lbuild/imsdb-linux/debug -limsdb \
    -Lbuild/libcob/.libs -lcob -pthread -o "$tmp/libcobims.so"
gcc -std=c11 -Wall -Wextra -Werror -I. extras/imsdb/imsbatch.c \
    -Lbuild/libcob/.libs -lcob -o "$tmp/imsbatch"
build/cobc/.libs/cobc -m -fdata-codeset=IBM-037 \
    -fbinary-byteorder=big-endian -o "$tmp/DLITCBL.so" \
    reference/Bank-of-Z-main/src/base/ims/cobol/IBSCUDAT.cbl
python3 - "$tmp/input.msg" <<'PY'
import pathlib
import struct
import sys

def field(value, length):
    return value.ljust(length).encode('cp037')

customer = (struct.pack('<I', 1) + field('TESTLAST', 50)
            + field('TESTFIRST', 50) + field('1 MAIN STREET', 80)
            + field('AUSTIN', 25) + field('TX', 2)
            + field('78701', 15) + field('555-0100', 12))
message = b'\x00\x00' + field('IBSCUDAT', 8) + customer
message = struct.pack('>H', len(message) + 2) + message
pathlib.Path(sys.argv[1]).write_bytes(message)
PY
build/imsdb-linux/debug/imsdbctl import-csv-bundle "$tmp/bank.wal" \
    reference/Bank-of-Z-main/src/base/ims/DBD \
    reference/Bank-of-Z-main/src/base/ims/LoadData
export COB_LIBRARY_PATH="$tmp"
export COB_PRE_LOAD="$tmp/libcobims.so"
export COB_IMS_WAL="$tmp/bank.wal"
export COB_IMS_DBD_DIR="$root/reference/Bank-of-Z-main/src/base/ims/DBD"
export COB_IMS_PSB="$root/reference/Bank-of-Z-main/src/base/ims/PSB/IBSCUDAT.asm"
export COB_IMS_ENTRY=DLITCBL
export COB_IMS_DBD_NAME=CUSTOMER
export COB_IMS_ENCODING=IBM-037
export COB_IMS_AUTOCOMMIT=1
export DD_IMSINPUT="$tmp/input.msg"
export DD_IMSOUTPUT="$tmp/output.msg"
"$tmp/imsbatch" > "$tmp/run.log"
build/cobc/.libs/cobc -x -fdata-codeset=IBM-037 \
    -fbinary-byteorder=big-endian -o "$tmp/ims-verify" \
    extras/imsdb/tests/bank-verify.cob
"$tmp/ims-verify"
build/cobc/.libs/cobc -x -o "$tmp/ims-dexec" \
    extras/imsdb/tests/execdli-service-smoke.cob
"$tmp/ims-dexec"
checkpoint=$(build/imsdb-linux/debug/imsdbctl latest-checkpoint "$tmp/bank.wal" \
    reference/Bank-of-Z-main/src/base/ims/DBD)
test "$checkpoint" = 'CHECK001'
python3 - "$tmp/output.msg" "$tmp/run.log" <<'PY'
import pathlib
import struct
import sys

data = pathlib.Path(sys.argv[1]).read_bytes()
length = struct.unpack('>H', data[:2])[0]
assert length == len(data), (length, len(data))
assert len(data) >= 274, len(data)
assert b'TESTFIRST'.decode('ascii').encode('cp037') in data
log = pathlib.Path(sys.argv[2]).read_text(errors='replace')
assert 'UPDATE SUCCESSFUL' in log, log
assert 'NO MORE MESSAGES' in log, log
print('IMS BANK ORIGINAL OK')
PY
build/cobc/.libs/cobc -x -fbinary-byteorder=big-endian \
    -o "$tmp/ims-io" extras/imsdb/tests/io-pcb-smoke.cob
export DD_IMSOUTPUT="$tmp/rollback.msg"
if IMS_IO_FAIL=1 "$tmp/ims-io"; then
    echo 'IMS I/O PCB failure path returned zero' >&2
    exit 1
fi
test ! -e "$tmp/rollback.msg"
IMS_IO_FAIL=0 "$tmp/ims-io"
python3 - "$tmp/rollback.msg" <<'PY'
import pathlib
import sys
data = pathlib.Path(sys.argv[1]).read_bytes()
assert data == b'\x00\x08\x00\x00TEST', data
print('IMS IO PCB COMMIT/ROLLBACK OK')
PY

# The same unchanged Bank of Z entry now receives a live, durable TM message.
export COB_IMS_TM_DIR="$tmp/tm-queue"
export COB_IMS_TM_DEST=IBSCUDAT
input_hex=$(python3 - "$tmp/input.msg" <<'PY'
import pathlib, sys
print(pathlib.Path(sys.argv[1]).read_bytes().hex())
PY
)
build/imsdb-linux/debug/imsdbctl tm-enqueue "$COB_IMS_TM_DIR" IBSCUDAT TERM0001 "$input_hex" >/dev/null
"$tmp/imsbatch" > "$tmp/tm-run.log"
test -z "$(build/imsdb-linux/debug/imsdbctl tm-list "$COB_IMS_TM_DIR" IBSCUDAT)"
build/imsdb-linux/debug/imsdbctl tm-list "$COB_IMS_TM_DIR" TERM0001 > "$tmp/tm-output.txt"
python3 - "$tmp/tm-output.txt" "$tmp/tm-run.log" <<'PY'
import pathlib, sys
rows = pathlib.Path(sys.argv[1]).read_text().splitlines()
assert len(rows) == 1, rows
id_, source, payload = rows[0].split()
assert source == 'IBSCUDAT', rows
data = bytes.fromhex(payload)
assert int.from_bytes(data[:2], 'big') == len(data)
assert 'TESTFIRST'.encode('cp037') in data
log = pathlib.Path(sys.argv[2]).read_text(errors='replace')
assert 'UPDATE SUCCESSFUL' in log and 'NO MORE MESSAGES' in log, log
print('IMS TM BANK QUEUE OK')
PY

# A failing COBOL task must release its input lease and discard its reply.
export COB_IMS_TM_DEST=IMSIOTEST
export COB_IMS_TM_LTERM=TERM0002
if IMS_IO_FAIL=1 "$tmp/ims-io"; then
    echo 'IMS TM failure path returned zero' >&2
    exit 1
fi
test -z "$(build/imsdb-linux/debug/imsdbctl tm-list "$COB_IMS_TM_DIR" TERM0002)"
IMS_IO_FAIL=0 "$tmp/ims-io"
build/imsdb-linux/debug/imsdbctl tm-list "$COB_IMS_TM_DIR" TERM0002 | \
    grep -q '0008000054455354'
echo 'IMS TM TASK ROLLBACK OK'
