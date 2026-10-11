#!/bin/sh
# Run inside gnucobol-db2-build after jclsh/test-utilities.ps1 created FB data.
set -eu
driver=/usr/local/lib64/python3.9/site-packages
output=build/jclsh
fixture=build/jclsh/genapp-vsam/ksdscust.dat
test -f "$fixture.jclsh-meta"
mkdir -p "$output"
export LD_LIBRARY_PATH="/src/build/libcob/.libs:$driver/clidriver/lib:$driver/ibm_db.libs"
python3 - <<'PY'
import os
import time
import ibm_db

user, password = os.environ["DB2_USER"], os.environ["DB2_PASSWORD"]
dsn = os.environ.get("DB2_DSN")
connection = (f"DSN={dsn};UID={user};PWD={password}" if dsn else
    f"DATABASE={os.environ['DB2_DATABASE']};HOSTNAME={os.environ['DB2_HOST']};"
    f"PORT={os.environ.get('DB2_PORT', '50000')};PROTOCOL=TCPIP;UID={user};PWD={password}")
for attempt in range(30):
    try:
        database = ibm_db.connect(connection, "", "")
        ibm_db.close(database)
        break
    except Exception:
        if attempt == 29:
            raise RuntimeError("Db2 did not become ready") from None
        time.sleep(5)
PY

gcc -std=c11 -Wall -Wextra -Werror -O2 \
  extras/cobcics/local_vsam.c extras/cobcics/test-local-vsam.c \
  -o "$output/test-local-vsam"
cp "$fixture" "$output/genapp-vsam/ksdscust-uow-unit.dat"
cp "$fixture.jclsh-meta" "$output/genapp-vsam/ksdscust-uow-unit.dat.jclsh-meta"
"$output/test-local-vsam" "$output/genapp-vsam/ksdscust-uow-unit.dat"

gcc -std=c11 -Wall -Wextra -Werror -Wno-error=format-truncation \
  -ffunction-sections -fdata-sections -Wl,--gc-sections -I/src \
  extras/cobcics/test-vsam-db2-uow.c extras/cobcics/local_vsam.c \
  -o "$output/test-vsam-db2-uow"
cp "$fixture" "$output/genapp-vsam/ksdscust-uow-mock.dat"
cp "$fixture.jclsh-meta" "$output/genapp-vsam/ksdscust-uow-mock.dat.jclsh-meta"
"$output/test-vsam-db2-uow" "$output/genapp-vsam/ksdscust-uow-mock.dat"

gcc -std=c11 -Wall -Wextra -Werror -Wno-error=format-truncation \
  -ffunction-sections -fdata-sections -Wl,--gc-sections -I/src \
  -I"$driver/clidriver/include" \
  extras/cobcics/test-vsam-db2-real.c extras/cobcics/local_vsam.c \
  extras/db2sql/db2sql.c -L/src/build/libcob/.libs -lcob \
  -L"$driver/clidriver/lib" -ldb2 \
  -Wl,-rpath-link,"$driver/ibm_db.libs" \
  -o "$output/test-vsam-db2-real"
cp "$fixture" "$output/genapp-vsam/ksdscust-uow-real.dat"
cp "$fixture.jclsh-meta" "$output/genapp-vsam/ksdscust-uow-real.dat.jclsh-meta"
"$output/test-vsam-db2-real" "$output/genapp-vsam/ksdscust-uow-real.dat"
