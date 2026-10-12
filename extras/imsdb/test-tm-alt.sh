#!/bin/sh
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$root"
tmp=$(mktemp -d /tmp/imsdb-tm-alt.XXXXXX)
trap 'rm -rf -- "$tmp"' EXIT
export LD_LIBRARY_PATH="$root/build/libcob/.libs:$root/build/imsdb-linux/debug${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
export COB_CONFIG_DIR="$root/config"
export COB_CFLAGS="-I$root"
export COB_LDFLAGS="-L$root/build/libcob/.libs"
gcc -std=c11 -Wall -Wextra -Werror -fPIC -shared -I. -Iextras/imsdb \
    extras/imsdb/cbltdli.c -Lbuild/imsdb-linux/debug -limsdb \
    -Lbuild/libcob/.libs -lcob -pthread -o "$tmp/libcobims.so"
gcc -std=c11 -Wall -Wextra -Werror -I. extras/imsdb/imsbatch.c \
    -Lbuild/libcob/.libs -lcob -o "$tmp/imsbatch"
build/cobc/.libs/cobc -m -o "$tmp/IMSTMSMOKE.so" extras/imsdb/tests/tm-alt.cob
build/cobc/.libs/cobc -m -o "$tmp/IMSTMPCBS.so" extras/imsdb/tests/tm-multipcb.cob
build/cobc/.libs/cobc -m -o "$tmp/IMSTMMULTI.so" extras/imsdb/tests/tm-multiseg.cob
build/cobc/.libs/cobc -m -o "$tmp/IMSTMINPUT.so" extras/imsdb/tests/tm-input.cob
build/cobc/.libs/cobc -m -o "$tmp/IMSTMBOUND.so" extras/imsdb/tests/tm-boundary.cob
export COB_LIBRARY_PATH="$tmp"
export COB_PRE_LOAD="$tmp/libcobims.so"
export COB_IMS_PSB="$root/extras/imsdb/tests/tm-alt.psb"
export COB_IMS_ENTRY=IMSTMSMOKE
export COB_IMS_TM_DIR="$tmp/queue"
export COB_IMS_TM_DEST=TMALT
"$tmp/imsbatch"
build/imsdb-linux/debug/imsdbctl tm-list "$COB_IMS_TM_DIR" TERMALT > "$tmp/output"
test "$(wc -l < "$tmp/output")" -eq 1
grep -q 'TMALT  *00080000414c544f' "$tmp/output"
echo 'IMS TM ALTERNATE PCB CHNG/ISRT OK'
mkdir "$tmp/ebcdic"
build/cobc/.libs/cobc -m -fdata-codeset=IBM-037 -o "$tmp/ebcdic/IMSTMSMOKE.so" extras/imsdb/tests/tm-alt.cob
export COB_LIBRARY_PATH="$tmp/ebcdic"
export COB_IMS_ENCODING=IBM-037
export COB_IMS_TM_DIR="$tmp/ebcdic-queue"
export COB_IMS_PSB="$root/extras/imsdb/tests/tm-alt.psb"
"$tmp/imsbatch"
build/imsdb-linux/debug/imsdbctl tm-list "$COB_IMS_TM_DIR" TERMALT | \
    grep -q '00080000c1d3e3d6'
echo 'IMS TM EBCDIC ALTERNATE PCB OK'
export COB_LIBRARY_PATH="$tmp"
unset COB_IMS_ENCODING
export COB_IMS_PSB="$root/extras/imsdb/tests/tm-multipcb.psb"
export COB_IMS_ENTRY=IMSTMPCBS
"$tmp/imsbatch"
echo 'IMS TM MULTI PCB PSB ORDER OK'
export COB_IMS_PSB="$root/extras/imsdb/tests/tm-alt.psb"
export COB_IMS_ENTRY=IMSTMMULTI
export COB_IMS_TM_DIR="$tmp/multi-queue"
"$tmp/imsbatch"
build/imsdb-linux/debug/imsdbctl tm-list "$COB_IMS_TM_DIR" TERMONE | \
    grep -q '000800004f4e4531000800004f4e4532'
build/imsdb-linux/debug/imsdbctl tm-list "$COB_IMS_TM_DIR" TERMTWO | \
    grep -q '0008000054574f31'
echo 'IMS TM MULTI SEGMENT/PURG OK'
export COB_IMS_PSB="$root/extras/imsdb/tests/tm-input.psb"
export COB_IMS_ENTRY=IMSTMINPUT
export COB_IMS_TM_DEST=TMINPUT
build/imsdb-linux/debug/imsdbctl tm-enqueue "$COB_IMS_TM_DIR" TMINPUT TERM0001 \
    '00080000414243440008000045464748' >/dev/null
if IMS_TM_FAIL=1 "$tmp/imsbatch"; then
    echo 'IMS TM input rollback path returned zero' >&2
    exit 1
fi
test "$(build/imsdb-linux/debug/imsdbctl tm-list "$COB_IMS_TM_DIR" TMINPUT | wc -l)" -eq 1
IMS_TM_FAIL=0 "$tmp/imsbatch"
test -z "$(build/imsdb-linux/debug/imsdbctl tm-list "$COB_IMS_TM_DIR" TMINPUT)"
echo 'IMS TM MULTI SEGMENT GU/GN/ROLLBACK OK'
export COB_IMS_ENTRY=IMSTMBOUND
export COB_IMS_TM_DEST=TMINPUT
export COB_IMS_TM_DIR="$tmp/boundary-queue"
build/imsdb-linux/debug/imsdbctl tm-enqueue "$COB_IMS_TM_DIR" TMINPUT TERM0001 \
    '000800004f4e4531' >/dev/null
build/imsdb-linux/debug/imsdbctl tm-enqueue "$COB_IMS_TM_DIR" TMINPUT TERM0001 \
    '0008000054574f32' >/dev/null
if "$tmp/imsbatch"; then
    echo 'IMS TM second-message failure returned zero' >&2
    exit 1
fi
remaining=$(build/imsdb-linux/debug/imsdbctl tm-list "$COB_IMS_TM_DIR" TMINPUT)
test "$(printf '%s\n' "$remaining" | wc -l)" -eq 1
printf '%s\n' "$remaining" | grep -q '0008000054574f32'
echo 'IMS TM NEXT-GU SYNCPOINT OK'
