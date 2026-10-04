#!/bin/sh
set -eu

site_packages=/usr/local/lib64/python3.9/site-packages
cli_driver=$site_packages/clidriver

mkdir -p build/pl1c
cc -std=c11 -Wall -Wextra -Werror \
  -Iextras/pl1c/runtime -I"$cli_driver/include" \
  build/pl1c/BNKSTMT.c \
  extras/pl1c/runtime/pli_runtime.c \
  extras/pl1c/runtime/pli_db2.c \
  -L"$cli_driver/lib" -Wl,-rpath-link,"$site_packages/ibm_db.libs" \
  -ldb2 -o build/pl1c/BNKSTMT
