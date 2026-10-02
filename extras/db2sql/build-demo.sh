#!/bin/sh
# Run inside Dockerfile.build with the repository mounted at /src.
set -eu

cd "$(dirname "$0")/../.."
root=$(pwd)
build="$root/build/db2sql-build"
driver=${DB2_CLI_ROOT:-/usr/local/lib64/python3.9/site-packages/clidriver}

sh autogen.sh
mkdir -p "$build"
cd "$build"
"$root/configure" --without-db --prefix="$build/install"
make tarstamp.h
make -C lib -j4
make -C libcob -j4
make -C cobc cobc

cd "$root"
gcc -std=c11 -Wall -Wextra -Werror -fPIC -shared \
    -I"$root" -I"$driver/include" extras/db2sql/db2sql.c \
    -L"$build/libcob/.libs" -lcob -L"$driver/lib" -ldb2 \
    -o "$build/libdb2sql.so"
COB_DB2_SQL_PREPROCESSOR="$root/bin/db2sql-preprocess.py" \
    sh "$build/pre-inst-env" "$build/cobc/cobc" --db2-sql \
    -x -Icopy -o "$build/db2-demo" extras/db2sql/demo.cbl
COB_DB2_SQL_PREPROCESSOR="$root/bin/db2sql-preprocess.py" \
    sh "$build/pre-inst-env" "$build/cobc/cobc" --db2-sql \
    -x -Icopy -o "$build/phase2-demo" extras/db2sql/phase2-demo.cbl
COB_DB2_SQL_PREPROCESSOR="$root/bin/db2sql-preprocess.py" \
    sh "$build/pre-inst-env" "$build/cobc/cobc" --db2-sql \
    -fdata-codeset=IBM-1399 -fsource-codeset=UTF-8 \
    -x -Icopy -o "$build/ebcdic-demo" extras/db2sql/ebcdic-demo.cbl
