#!/usr/bin/env python3
"""Convert and compile every Bank of Z CICS COBOL source (excluding IMS calls).

Requires Python, offline Cargo dependencies, and the gnucobol-db2-build image.
Generated files and compiler diagnostics are kept under build/bank-syntax.
"""

import importlib.util
import pathlib
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
    spec = importlib.util.spec_from_file_location("cics_preprocess", ROOT / "bin/cics-preprocess.py")
    cics = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(cics)
    maps = {}
    for source in sorted((SOURCE / "bms").glob("*.bms")):
        copybook = maps_dir / (source.stem + ".cpy")
        run("cargo", "run", "--manifest-path", "extras/cobcics/Cargo.toml",
            "--offline", "--quiet", "--bin", "cobcics-mapgen", "--",
            str(source), str(copybook))
        for name, fields in cics.maps_from_copybook(copybook.read_text()).items():
            if name in maps:
                raise RuntimeError(f"duplicate BMS map {name}")
            maps[name] = fields
    sources = sorted((SOURCE / "cobol").glob("*.cbl"))
    for source in sources:
        text = source.read_text(encoding="utf-8")
        if "EXEC CICS" in text.upper():
            text = cics.transform(text, maps)
        converted = converted_dir / (source.stem + ".cob")
        converted.write_text(text, encoding="utf-8")
        run(sys.executable, "bin/db2sql-preprocess.py", "-I",
            str(SOURCE / "copy"), str(converted), str(sql_dir / converted.name))
    script = """set -eu
for file in build/bank-syntax/sql/*.cob; do
  name=$(basename "$file" .cob)
  build/cobc/cobc -c \
    -I reference/Bank-of-Z-main/src/base/cics/copy \
    -I extras/cobcics/copy -I build/bank-syntax/maps \
    -o "build/bank-syntax/$name.o" "$file" \
    >"build/bank-syntax/$name.log" 2>&1 || {
      cat "build/bank-syntax/$name.log"
      exit 1
    }
  echo "$name OK"
done
gcc -std=c11 -Wall -Wextra -Werror \
  extras/cobcics/bridge.c extras/cobcics/test_bridge_channel.c \
  -o build/bank-syntax/test_bridge_channel
printf 'CCI3\\175OMENBANKAPP \\000\\000\\000\\000' | \
  build/bank-syntax/test_bridge_channel
"""
    run("docker", "run", "--rm", "-e", "LICENSE=accept",
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
