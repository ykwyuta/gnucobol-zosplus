#!/bin/sh
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$root"
tmp=$(mktemp -d /tmp/imsdb-bank-login.XXXXXX)
trap 'rm -rf -- "$tmp"' EXIT
export LD_LIBRARY_PATH="$root/build/libcob/.libs:$root/build/imsdb-linux/debug${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
export COB_CONFIG_DIR="$root/config"
export COB_COPY_DIR="$root/copy"
export COB_CFLAGS="-I$root"
export COB_LDFLAGS="-L$root/build/libcob/.libs"

# The original PROGRAM-ID lacks its terminating period. Normalize the build
# copy and retain the reference byte-for-byte.
sed 's/^       PROGRAM-ID\. IBLOGIN1$/       PROGRAM-ID. IBLOGIN1./' \
    reference/Bank-of-Z-main/src/base/ims/cobol/IBLOGIN1.cbl \
    > "$tmp/IBLOGIN1.cbl"
build/cobc/.libs/cobc -m -fdata-codeset=IBM-037 \
    -fbinary-byteorder=big-endian -o "$tmp/DLITCBL.so" "$tmp/IBLOGIN1.cbl"
gcc -std=c11 -Wall -Wextra -Werror -I. extras/imsdb/imsbatch.c \
    -Lbuild/libcob/.libs -lcob -o "$tmp/imsbatch"
gcc -std=c11 -Wall -Wextra -Werror -fPIC -shared -I. -Iextras/imsdb \
    extras/imsdb/cbltdli.c -Lbuild/imsdb-linux/debug -limsdb \
    -Lbuild/libcob/.libs -lcob -pthread -o "$tmp/libcobims.so"
build/imsdb-linux/debug/imsdbctl import-csv-bundle "$tmp/bank.wal" \
    reference/Bank-of-Z-main/src/base/ims/DBD \
    reference/Bank-of-Z-main/src/base/ims/LoadData >/dev/null
export COB_LIBRARY_PATH="$tmp"
export COB_PRE_LOAD="$tmp/libcobims.so"
export COB_IMS_WAL="$tmp/bank.wal"
export COB_IMS_DBD_DIR="$root/reference/Bank-of-Z-main/src/base/ims/DBD"
export COB_IMS_PSB="$root/reference/Bank-of-Z-main/src/base/ims/PSB/IBLOGIN.asm"
export COB_IMS_ENTRY=DLITCBL
export COB_IMS_DBD_NAME=CUSTOMER
export COB_IMS_ENCODING=IBM-037
export COB_IMS_AUTOCOMMIT=1
export COB_IMS_TM_DIR="$tmp/tm-queue"
export COB_IMS_TM_DEST=IBLOGIN1

for case in bad-password success already-logged-in; do
    case "$case" in
        bad-password) password=incorrect; lterm=TERM0001 ;;
        success) password=password; lterm=TERM0002 ;;
        already-logged-in) password=password; lterm=TERM0003 ;;
    esac
    input_hex=$(python3 - "$password" <<'PY'
import struct
import sys

def field(text, length):
    return text.ljust(length).encode('cp037')

message = (b'\0\0' + field('IBLOGIN1', 8) + field('000000001', 9)
           + field(sys.argv[1], 16))
message = struct.pack('>H', len(message) + 2) + message
print(message.hex())
PY
    )
    build/imsdb-linux/debug/imsdbctl tm-enqueue \
        "$COB_IMS_TM_DIR" IBLOGIN1 "$lterm" "$input_hex" >/dev/null
    "$tmp/imsbatch" > "$tmp/$case.log"
    build/imsdb-linux/debug/imsdbctl tm-list \
        "$COB_IMS_TM_DIR" "$lterm" > "$tmp/$case.output"
done
python3 - "$tmp" <<'PY'
import pathlib
import sys

base = pathlib.Path(sys.argv[1])
expected = {
    'bad-password': 'PASSWORD INVALID',
    'success': 'LOGIN SUCCESSFUL',
    'already-logged-in': 'CUSTOMER ALREADY LOGGED IN',
}
for case, phrase in expected.items():
    rows = (base / f'{case}.output').read_text().splitlines()
    assert len(rows) == 1, (case, rows)
    payload = bytes.fromhex(rows[0].split()[2])
    assert phrase.encode('cp037') in payload, (case, payload.hex())
print('Bank original IBLOGIN1 IMS TM three paths OK')
PY
build/imsdb-linux/debug/imsdbctl get-root "$tmp/bank.wal" \
    reference/Bank-of-Z-main/src/base/ims/DBD \
    CUSTOMER 00000001 > "$tmp/customer.txt"
python3 - "$tmp/customer.txt" <<'PY'
import pathlib
import sys

record = bytes.fromhex(pathlib.Path(sys.argv[1]).read_text().split()[-1])
assert record[238:239] == '1'.encode('cp037'), record[238:239].hex()
print('Bank original IBLOGIN1 CUSTOMER status committed OK')
PY

mkdir "$tmp/logout"
build/cobc/.libs/cobc -m -fdata-codeset=IBM-037 \
    -fbinary-byteorder=big-endian -o "$tmp/logout/DLITCBL.so" \
    reference/Bank-of-Z-main/src/base/ims/cobol/IBLOGOUT.cbl
export COB_LIBRARY_PATH="$tmp/logout"
export COB_IMS_PSB="$root/reference/Bank-of-Z-main/src/base/ims/PSB/IBLOGOUT.asm"
export COB_IMS_TM_DEST=IBLOGOUT
logout_hex=$(python3 - <<'PY'
import struct

def field(text, length):
    return text.ljust(length).encode('cp037')

message = b'\0\0' + field('IBLOGOUT', 8) + field('000000001', 9)
print((struct.pack('>H', len(message) + 2) + message).hex())
PY
)
build/imsdb-linux/debug/imsdbctl tm-enqueue \
    "$COB_IMS_TM_DIR" IBLOGOUT TERM0004 "$logout_hex" >/dev/null
"$tmp/imsbatch" > "$tmp/logout.log"
build/imsdb-linux/debug/imsdbctl tm-list \
    "$COB_IMS_TM_DIR" TERM0004 > "$tmp/logout.output"
build/imsdb-linux/debug/imsdbctl get-root "$tmp/bank.wal" \
    reference/Bank-of-Z-main/src/base/ims/DBD \
    CUSTOMER 00000001 > "$tmp/customer-logout.txt"
python3 - "$tmp/logout.output" "$tmp/customer-logout.txt" <<'PY'
import pathlib
import sys

rows = pathlib.Path(sys.argv[1]).read_text().splitlines()
assert len(rows) == 1, rows
payload = bytes.fromhex(rows[0].split()[2])
assert 'LOGOFF SUCCESSFUL'.encode('cp037') in payload, payload.hex()
record = bytes.fromhex(pathlib.Path(sys.argv[2]).read_text().split()[-1])
assert record[238:239] == '0'.encode('cp037'), record[238:239].hex()
print('Bank original IBLOGOUT IMS TM and CUSTOMER status committed OK')
PY
