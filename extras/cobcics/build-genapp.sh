#!/bin/sh
set -eu
export COB_CFLAGS="${COB_CFLAGS:-} -DCOBCICS_LOCAL_VSAM"
cobc=${COBC_BIN:-build/cobc/cobc}
driver=/usr/local/lib64/python3.9/site-packages
gcc -std=c11 -Wall -Wextra -Werror -fPIC -shared \
  -I/src -I"$driver/clidriver/include" extras/db2sql/db2sql.c \
  -L/src/build/libcob/.libs -lcob -L"$driver/clidriver/lib" -ldb2 \
  -o build/genapp/libdb2sql.so
gcc -std=c11 -Wall -Wextra -Werror -fPIC -shared \
  -I/src -I"$driver/clidriver/include" \
  extras/db2sql/db2sql.c extras/cobcics/genapp_file.c extras/cobcics/local_vsam.c \
  -L/src/build/libcob/.libs -lcob -L"$driver/clidriver/lib" -ldb2 \
  -o build/genapp/libgenapp_file.so
gcc -std=c11 -Wall -Wextra -Werror -fPIC -shared \
  extras/cobcics/genapp_queue.c -o build/genapp/libgenapp_queue.so
python3 bin/db2sql-preprocess.py \
  extras/db2sql/varchar-demo.cbl build/genapp/varchar-demo.cob
"$cobc" -x -o build/genapp/varchar-demo \
  build/genapp/varchar-demo.cob
for source in build/genapp/sql/*.cob; do
  name=${source##*/}
  name=${name%.cob}
  "$cobc" -c \
    -I reference/cics-genapp/base/src -I extras/cobcics/copy -I build/genapp \
    -o "build/genapp/$name.o" "$source"
done
"$cobc" -x -o build/genapp/GENAPP \
  extras/cobcics/genapp-entry-worker.cob \
  build/genapp/*.o \
  extras/cobcics/bridge.c extras/cobcics/genapp_queue.c \
  extras/cobcics/genapp_file.c extras/cobcics/local_vsam.c extras/cobcics/genapp_schedule.c
"$cobc" -x -o build/genapp/GENAPP-API \
  extras/cobcics/genapp-api-worker.cob \
  build/genapp/*.o \
  extras/cobcics/bridge.c extras/cobcics/genapp_queue.c \
  extras/cobcics/genapp_file.c extras/cobcics/local_vsam.c extras/cobcics/genapp_schedule.c
for name in lgtestc1 lgtestp1 lgtestp2 lgtestp3 lgtestp4; do
  objects=''
  for object in build/genapp/*.o; do
    if [ "$object" != "build/genapp/$name.o" ]; then
      objects="$objects $object"
    fi
  done
  "$cobc" -x \
    -I reference/cics-genapp/base/src -I extras/cobcics/copy -I build/genapp \
    -o "build/genapp/$name" \
    "build/genapp/sql/$name.cob" $objects \
    extras/cobcics/bridge.c extras/cobcics/genapp_queue.c \
    extras/cobcics/genapp_file.c extras/cobcics/local_vsam.c extras/cobcics/genapp_schedule.c
done
python3 bin/cics-preprocess.py extras/cobcics/genapp-map5-demo.cbl \
  build/genapp/SSMAP.cpy build/genapp/map5-demo.cob
"$cobc" -x -I build/genapp -o build/genapp/map5-demo \
  build/genapp/map5-demo.cob extras/cobcics/bridge.c \
  extras/cobcics/genapp_queue.c extras/cobcics/genapp_file.c extras/cobcics/local_vsam.c \
  extras/cobcics/genapp_schedule.c
export LD_LIBRARY_PATH="/src/build/libcob/.libs:/src/build/genapp:$driver/clidriver/lib:$driver/ibm_db.libs"
python3 extras/cobcics/test-genapp-setup.py build/genapp/GENAPP
python3 extras/cobcics/test-genapp-stsq.py build/genapp/GENAPP
python3 extras/cobcics/test-genapp-schedule.py build/genapp/GENAPP
