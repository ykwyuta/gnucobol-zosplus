"""Package the release Rust executables into one downloadable CI ZIP."""

import argparse
from pathlib import Path
from zipfile import ZIP_DEFLATED, ZipFile


ROOT = Path(__file__).resolve().parents[2]
TOOLS = {
    "cobcicsd": "cobcics",
    "cobcics-mapgen": "cobcics",
    "jclsh": "jclsh",
    "fbconvert": "jclsh",
    "pl1c": "pl1c",
    "imsdbctl": "imsdb",
}


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--os", choices=("Linux", "Windows"), required=True)
    args = parser.parse_args()

    suffix = ".exe" if args.os == "Windows" else ""
    output = ROOT / "dist" / f"zosplus-tools-{args.os}.zip"
    output.parent.mkdir(exist_ok=True)

    files = {}
    for name, crate in TOOLS.items():
        binary = ROOT / "extras" / crate / "target" / "release" / f"{name}{suffix}"
        if not binary.is_file():
            raise FileNotFoundError(binary)
        files[f"bin/{name}{suffix}"] = binary

    runtime = ROOT / "extras" / "pl1c" / "runtime"
    for source in sorted(runtime.iterdir()):
        if source.is_file() and source.suffix in (".c", ".h"):
            files[f"pl1c/runtime/{source.name}"] = source

    files["jclsh/vsam_db2.py"] = ROOT / "extras" / "jclsh" / "vsam_db2.py"
    files["bin/imsdli-preprocess.py"] = ROOT / "bin" / "imsdli-preprocess.py"
    for name in ("cbltdli.c", "imsdb.h", "imsbatch.c"):
        files[f"imsdb/{name}"] = ROOT / "extras" / "imsdb" / name
    library = "imsdb.dll" if args.os == "Windows" else "libimsdb.so"
    files[f"lib/{library}"] = ROOT / "extras" / "imsdb" / "target" / "release" / library

    with ZipFile(output, "w", compression=ZIP_DEFLATED, compresslevel=9) as archive:
        for destination, source in files.items():
            archive.write(source, destination)

    with ZipFile(output) as archive:
        if archive.testzip() is not None or not set(files).issubset(archive.namelist()):
            raise RuntimeError(f"Invalid binary ZIP: {output}")
    print(f"Created {output} with {len(files)} files")


if __name__ == "__main__":
    main()
