#!/usr/bin/env python3
"""Convert and compile every Bank of Z CICS COBOL source (excluding IMS calls).

Requires Python, offline Cargo dependencies, and the gnucobol-db2-build image.
Generated files and compiler diagnostics are kept under build/bank-syntax.
"""

import pathlib
import shutil
import subprocess
import sys


ROOT = pathlib.Path(__file__).resolve().parents[2]
SOURCE = ROOT / "reference/Bank-of-Z-main/src/base/cics"
OUTPUT = ROOT / "build/bank-syntax"


def run(*args):
    result = subprocess.run(args, cwd=ROOT, text=True, capture_output=True)
    if result.returncode:
        raise RuntimeError(f"{' '.join(map(str, args))}:\n{result.stdout}{result.stderr}")


def main():
    maps_dir = OUTPUT / "maps"
    converted_dir = OUTPUT / "converted"
    sql_dir = OUTPUT / "sql"
    for directory in (maps_dir, converted_dir, sql_dir):
        directory.mkdir(parents=True, exist_ok=True)
    map_copybooks = []
    for source in sorted((SOURCE / "bms").glob("*.bms")):
        copybook = maps_dir / (source.stem + ".cpy")
        run("cargo", "run", "--manifest-path", "extras/cobcics/Cargo.toml",
            "--offline", "--quiet", "--bin", "cobcics-mapgen", "--",
            str(source), str(copybook))
        map_copybooks.append(copybook.read_text(encoding="utf-8"))
    combined = maps_dir / "all-maps.cpy"
    combined.write_text("".join(map_copybooks), encoding="utf-8")
    if sys.platform == "win32":
        cics = ROOT / "extras/preprocessors/target/debug/cics-preprocess.exe"
        sql = ROOT / "extras/preprocessors/target/debug/db2sql-preprocess.exe"
    else:
        cics = ROOT / "build/preprocessors-linux/debug/cics-preprocess"
        sql = ROOT / "build/preprocessors-linux/debug/db2sql-preprocess"
    sources = sorted((SOURCE / "cobol").glob("*.cbl"))
    for source in sources:
        converted = converted_dir / (source.stem + ".cob")
        if "EXEC CICS" in source.read_text(encoding="utf-8").upper():
            run(str(cics), "--any-mapset", str(source), str(combined), str(converted))
        else:
            shutil.copyfile(source, converted)
        run(str(sql), "-I",
            str(SOURCE / "copy"), str(converted), str(sql_dir / converted.name))
    script = """set -eu
for file in build/bank-syntax/sql/*.cob; do
  name=$(basename "$file" .cob)
  build/db2sql-build/cobc/.libs/cobc -c \
    -I reference/Bank-of-Z-main/src/base/cics/copy \
    -I extras/cobcics/copy -I build/bank-syntax/maps \
    -o "build/bank-syntax/$name.o" "$file" \
    >"build/bank-syntax/$name.log" 2>&1 || {
      cat "build/bank-syntax/$name.log"
      exit 1
    }
  echo "$name OK"
done
gcc -std=c11 -Wall -Wextra -Werror -I. \
  extras/cobcics/bridge.c extras/cobcics/test_bridge_channel.c \
  -Lbuild/db2sql-build/libcob/.libs -lcob -Wl,-rpath,/src/build/db2sql-build/libcob/.libs \
  -o build/bank-syntax/test_bridge_channel
printf 'CCI3\\175OMENBANKAPP \\000\\000\\000\\000' | \
  build/bank-syntax/test_bridge_channel
"""
    run("docker", "run", "--rm", "-e", "LICENSE=accept",
        "-e", "LD_LIBRARY_PATH=/src/build/db2sql-build/libcob/.libs:/src/build/libcob/.libs",
        "-e", "COB_CONFIG_DIR=/src/config", "-e", "COB_COPY_DIR=/src/copy",
        "-e", "COB_CFLAGS=-I/src",
        "--entrypoint", "sh", "-v", f"{ROOT}:/src", "-w", "/src",
        "gnucobol-db2-build", "-c", script)
    print(f"{len(sources)} Bank of Z COBOL sources compiled successfully")


if __name__ == "__main__":
    try:
        main()
    except (OSError, RuntimeError, ValueError) as error:
        print(error, file=sys.stderr)
        raise SystemExit(1)
