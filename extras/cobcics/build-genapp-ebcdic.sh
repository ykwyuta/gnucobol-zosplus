#!/bin/sh
# Compile the translated GenApp programs in IBM-037 data mode without
# replacing the normal build. The caller supplies COB_CONFIG_DIR, CFLAGS,
# LDFLAGS and COPY_DIR exactly as for build-genapp-setup.ps1.
set -eu
export COB_CFLAGS="${COB_CFLAGS:-} -DCOBCICS_LOCAL_VSAM"
cobc=${COBC_BIN:-build/cobc/cobc}

destination=build/genapp-ebcdic
mkdir -p "$destination/sql"
# The normal GenApp audit maps two source-level EBCDIC spaces to ASCII
# spaces for native mode. Restore the original literals for IBM-037 data.
for source in build/genapp/sql/*.cob; do
  cp "$source" "$destination/sql/${source##*/}"
done
screen_source="$destination/sql/lgtestc1.cob"
test "$(grep -ci "by x'20'" "$screen_source")" -eq 2
sed -i "s/by x'20'/by x'40'/g" "$screen_source"
for source in build/genapp/sql/*.cob; do
  name=${source##*/}
  name=${name%.cob}
  "$cobc" -c -fdata-codeset=IBM-037 \
    -I reference/cics-genapp/base/src -I extras/cobcics/copy \
    -I build/genapp -o "$destination/$name.o" "$destination/sql/$name.cob"
done

"$cobc" -x -fdata-codeset=IBM-037 \
  -o "$destination/GENAPP" extras/cobcics/genapp-entry-worker.cob \
  "$destination"/*.o extras/cobcics/bridge.c \
  extras/cobcics/genapp_queue.c extras/cobcics/genapp_file.c extras/cobcics/local_vsam.c \
  extras/cobcics/genapp_schedule.c

for name in lgtestc1 lgtestp1 lgtestp2 lgtestp3 lgtestp4; do
  objects=''
  for object in "$destination"/*.o; do
    if [ "$object" != "$destination/$name.o" ]; then
      objects="$objects $object"
    fi
  done
  "$cobc" -x -fdata-codeset=IBM-037 \
    -I reference/cics-genapp/base/src -I extras/cobcics/copy \
    -I build/genapp -o "$destination/$name" \
    "$destination/sql/$name.cob" $objects \
    extras/cobcics/bridge.c extras/cobcics/genapp_queue.c \
    extras/cobcics/genapp_file.c extras/cobcics/local_vsam.c extras/cobcics/genapp_schedule.c
done
