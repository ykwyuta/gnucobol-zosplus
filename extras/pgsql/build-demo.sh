#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$root"
output=${PGSQL_BUILD_DIR:-$root/build/pgsql}
mkdir -p "$output"

if [ -n "${COBC_BIN:-}" ]; then
    cobc=$COBC_BIN
elif [ -x "$root/build/cobc/.libs/cobc" ]; then
    cobc=$root/build/cobc/.libs/cobc
else
    cobc=cobc
fi
libcob_dir=${LIBCOB_DIR:-$root/build/libcob/.libs}
export COB_CONFIG_DIR=${COB_CONFIG_DIR:-$root/config}
export COB_COPY_DIR=${COB_COPY_DIR:-$root/copy}
export COB_CFLAGS="-I$root ${COB_CFLAGS:-}"
export COB_LDFLAGS="-L$libcob_dir ${COB_LDFLAGS:-}"
export LD_LIBRARY_PATH="$libcob_dir${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
export COB_DB2_SQL_PREPROCESSOR="$root/build/preprocessors-linux/debug/db2sql-preprocess"

gcc -std=c11 -Wall -Wextra -Werror -fPIC -shared \
    -I"$root" $(pkg-config --cflags libpq) extras/pgsql/pgsql.c \
    -L"$libcob_dir" -lcob $(pkg-config --libs libpq) \
    -o "$output/libpgsqlsql.so"

"$cobc" --postgres-sql -x -Icopy \
    -o "$output/pgsql-demo" extras/pgsql/demo.cbl
"$cobc" --postgres-sql -fdata-codeset=IBM-1399 \
    -fsource-codeset=UTF-8 -x -Icopy \
    -o "$output/pgsql-ebcdic-demo" extras/pgsql/ebcdic-demo.cbl
printf 'Built PostgreSQL EXEC SQL demo in %s\n' "$output"
