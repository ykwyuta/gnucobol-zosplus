#!/bin/sh
# Build the PostgreSQL EXEC SQL ABI and CICS FILE bridge in one shared library.
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$root"
output=${PG_VSAM_BUILD_DIR:-$root/build/cobcics-postgres}
libcob_dir=${LIBCOB_DIR:-$root/build/libcob/.libs}
mkdir -p "$output"
gcc -std=c11 -Wall -Wextra -Werror -fPIC -shared \
    -DCOBCICS_VSAM_POSTGRES -I"$root" $(pkg-config --cflags libpq) \
    extras/pgsql/pgsql.c extras/cobcics/genapp_file.c \
    extras/cobcics/pg_vsam.c extras/cobcics/local_vsam.c \
    -L"$libcob_dir" -lcob $(pkg-config --libs libpq) \
    -o "$output/libpgsqlsql.so"
printf 'Built %s/libpgsqlsql.so\n' "$output"
