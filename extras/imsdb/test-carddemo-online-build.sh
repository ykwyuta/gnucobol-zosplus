#!/bin/sh
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$root"
tmp=$(mktemp -d /tmp/imsdb-carddemo-online.XXXXXX)
trap 'rm -rf -- "$tmp"' EXIT
card=reference/aws-mainframe-modernization-carddemo/app/app-authorization-ims-db2-mq
export LD_LIBRARY_PATH="$root/build/libcob/.libs${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
export COB_CONFIG_DIR="$root/config"
export COB_COPY_DIR="$root/copy"
export COB_CFLAGS="-I$root"
export COB_LDFLAGS="-L$root/build/libcob/.libs"
export PATH="$root/bin:$PATH"
export COB_CICS_PREPROCESSOR="$root/build/preprocessors-linux/debug/cics-preprocess"
export COB_IMS_DLI_PREPROCESSOR="$root/build/preprocessors-linux/debug/imsdli-preprocess"
export COB_DB2_SQL_PREPROCESSOR="$root/build/preprocessors-linux/debug/db2sql-preprocess"
python3 extras/cobcics/test_cics_preprocess.py
python3 extras/imsdb/tests/test_execdli.py
cp "$card/dcl/AUTHFRDS.dcl" "$tmp/AUTHFRDS.cpy"
for spec in COPAUS0C:COPAU00 COPAUS1C:COPAU01 COPAUA0C:COPAU00 COPAUS2C:COPAU00; do
    program=${spec%%:*}
    map=${spec#*:}
    source="$card/cbl/$program.cbl"
    if [ "$program" = COPAUS2C ]; then
        # The original ASKTIME repeats NOHANDLE. Keep the reference unchanged.
        sed 's/EXEC CICS ASKTIME NOHANDLE/EXEC CICS ASKTIME         /' "$source" > "$tmp/$program.cbl"
        source="$tmp/$program.cbl"
    fi
    for encoding in native IBM-037; do
        set --
        if [ "$encoding" = IBM-037 ]; then
            set -- -fdata-codeset=IBM-037 -fbinary-byteorder=big-endian
        fi
        if [ "$program" = COPAUS2C ]; then
            set -- "$@" --db2-sql -I "$tmp"
        fi
        build/cobc/.libs/cobc "$@" "--cics-map=$card/cpy-bms/$map.cpy" \
            --ims-dli=DBPAUTP0 -m \
            -I "$card/cpy" -I "$card/cpy-bms" \
            -I reference/aws-mainframe-modernization-carddemo/app/cpy \
            -I extras/cobcics/copy -I extras/mq/copy \
            -o "$tmp/$program-$encoding.so" "$source"
        test -s "$tmp/$program-$encoding.so"
        printf 'IMS CICS BUILD %s %s OK\n' "$program" "$encoding"
    done
done
