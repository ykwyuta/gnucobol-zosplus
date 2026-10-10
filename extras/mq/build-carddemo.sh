#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$root"
output=${MQ_BUILD_DIR:-build/mq}
mkdir -p "$output"

if [ -n "${COBC_BIN:-}" ]; then
    cobc=$COBC_BIN
elif [ -x build/cobc/cobc ]; then
    cobc=$root/build/cobc/cobc
else
    cobc=cobc
fi
libcob_dir=${LIBCOB_DIR:-$root/build/libcob/.libs}
export COB_CONFIG_DIR=${COB_CONFIG_DIR:-$root/config}
export COB_COPY_DIR=${COB_COPY_DIR:-$root/copy}
export COB_CFLAGS="-I$root ${COB_CFLAGS:-}"
export COB_LDFLAGS="-L$libcob_dir ${COB_LDFLAGS:-}"
export LD_LIBRARY_PATH="$libcob_dir${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"

gcc -std=c11 -Wall -Wextra -Werror -fPIC -shared \
    extras/mq/mqbridge.c $(pkg-config --cflags --libs libcurl) \
    -o "$output/libcobmq.so"

"$cobc" -x -I extras/mq/copy \
    -o "$output/mqdemo" extras/mq/demo.cbl

for name in CODATE01 COACCT01; do
    python3 bin/cics-preprocess.py \
        "reference/aws-mainframe-modernization-carddemo/app/app-vsam-mq/cbl/$name.cbl" \
        extras/mq/cics-placeholder.cpy "$output/$name.cob"
done

"$cobc" -x -I extras/mq/copy -I extras/cobcics/copy \
    -o "$output/CODATE01" "$output/CODATE01.cob" \
    extras/cobcics/bridge.c
"$cobc" -x -I extras/mq/copy -I extras/cobcics/copy \
    -I reference/aws-mainframe-modernization-carddemo/app/cpy \
    -o "$output/COACCT01" "$output/COACCT01.cob" \
    extras/cobcics/bridge.c extras/mq/carddemo_file.c

printf 'Built MQ demo and CardDemo MQ programs in %s\n' "$output"
