# imsdb: Rust IMS DB compatibility core (in progress)

This crate is the first implementation slice of [the IMS DB proposal](../../docs/proposal/imsdb-rust.md).
It includes an experimental GnuCOBOL `CBLTDLI` entry point for DB PCBs.

## Implemented

- Parse fixed-format DBD/PSB macro sources, including continuations, parent/child segments,
  sequence fields, LCHILD declarations, PCB processing options, and SENSEG declarations.
  Unsupported macro names fail with a source line number.
- Preserve segment bodies and sequence keys as raw bytes. Packed decimal and EBCDIC values
  are not converted on storage or retrieval.
- Insert, replace, delete, and read fixed-length segments through the Rust API. Parent-child
  relationships and unique versus nonunique sequence fields are represented in a B-tree index.
- Stage changes in a transaction. Committed changes use a checksummed append log and
  sync_data; rollback discards staged changes. Startup replays complete frames and trims
  an interrupted final frame. Commit applies only the staged changes and keeps an undo
  set until the log has synced, rather than copying the whole database index.
- Lock roots within one process, with timeout and a total key order to prevent deadlock.
  Hold reads retain the lock through commit or rollback. The WAL has an exclusive OS file lock
  while open, so a second process cannot replay or append to it concurrently.
- Read through a PSB session using GU/GN/GNP and GHU/GHN/GHNP, with unqualified SSAs or
  an equality predicate over a DBD field. EXEC DLI Get commands have a separate entry
  point with [Get Hold behavior](https://www.ibm.com/docs/en/ims/15.4.0?topic=programs-comparing-exec-dli-commands-dli-calls).
  Read positioning uses the B-tree index. Return statuses include GA, GK, GE, and GB.
- Apply ISRT/REPL/DLET through the same session and UOW. PROCOPT is checked; duplicate,
  missing Hold, and permission conditions produce II, DJ, and AM. DLET of a held parent
  recursively removes its dependents in the same UOW.
- A length-checked Rust call adapter fills the nine-field [IMS DB PCB mask](https://www.ibm.com/docs/en/ims/15.4.0?topic=db-specifying-pcb-mask),
  copies raw segment bytes to the caller, and accepts ASCII or GnuCOBOL 1047 control text.
  For EBCDIC SSAs, it decodes only the segment/field/operator bytes and preserves binary
  qualification values. The exported C ABI checks buffer lengths and translates native
  little-endian `COMP-5` values at the call boundary. A GnuCOBOL loadable shim provides
  `CBLTDLI` and the temporary explicit commit call `IMSDBCOM`.
- The GnuCOBOL shim accepts basic batch `CHKP` (commit) and `ROLB` (rollback) calls
  with an I/O PCB. It validates the eight-character checkpoint ID and stores it
  atomically with the DB changes in a checksummed WAL frame. `imsdbctl
  latest-checkpoint WAL DBD_OR_DIR` reads the last committed ID in a fresh process.
  Symbolic checkpoints, automatic program restart, and `XRST` remain unsupported.
- For an I/O PCB with a 32-byte mask, `GU` reads length-prefixed messages from
  `DD_IMSINPUT` and returns `QC` at end of input. `ISRT` stages a length-prefixed
  message for `DD_IMSOUTPUT` until commit. `imsbatch.c` can launch a GnuCOBOL
  module's original `DLITCBL` entry with an I/O PCB and one DB PCB. The
  `test-bank-original.sh` integration test runs the unmodified Bank of Z
  `IBSCUDAT.cbl` and checks its customer update and output message.
- `cobc --ims-dli=<DBD>` runs `bin/imsdli-preprocess.py` for the `EXEC DLI`
  forms in CardDemo. It produces DB calls with DIBSTAT feedback, and maps Get
  commands to Get Hold behavior. `IMSDEXEC` handles `SCHD`, `TERM`, and basic
  `CHKP`; `TERM` commits and releases the scheduled PSB. The original
  `CBPAUP0C.cbl` compiles and runs against an empty `DBPAUTP0` database in
  `test-carddemo-execdli.sh`. `test-carddemo-jcl.ps1` also runs its original
  `CBPAUP0J.jcl` through `jclsh` and Docker with the original inline SYSIN.
- Parse CardDemo's fixed-record GSAM DBDs and PCBs. `CBLTDLI` supports `OPEN`,
  `CLSE`, `GN`, `GU`
  with an eight-byte RSA, and `ISRT` through `DD_<input DD>` and `DD_<output DD>`.
  The output is staged until commit and discarded on rollback. `test-gsam.sh`
  exercises the original `DLIGSAMP` PSB and `PASFLDBD` DBD from COBOL.
- `jclsh` recognizes `PGM=DFSRRC00` with `BMP`, `DLI`, or `DBB` PARM, resolves a
  registered program and PSB, and passes IMS paths into the Docker step. With
  the rebuilt repository `libcob`, a zero-return `STOP RUN` commits the pending
  IMS UOW; nonzero return rolls it back. Programs can also call `IMSDBCOM` or
  basic `CHKP` explicitly.

## COBOL integration smoke test on WSL

Build the Linux Rust library and CLI with Docker from PowerShell at the repository root:

    docker run --rm --network none -v D:\workspace\gnucobol:/src -w /src `
        -e CARGO_TARGET_DIR=/src/build/imsdb-linux rust:1-bookworm `
        sh -lc 'PATH=/usr/local/cargo/bin:$PATH cargo build --manifest-path extras/imsdb/Cargo.toml --offline --lib --bin imsdbctl'
    wsl --exec sh /mnt/d/workspace/gnucobol/extras/imsdb/test-cbltdli.sh
    wsl --exec sh /mnt/d/workspace/gnucobol/extras/imsdb/test-gsam.sh
    wsl --exec sh /mnt/d/workspace/gnucobol/extras/imsdb/test-bank-original.sh
    wsl --exec sh /mnt/d/workspace/gnucobol/extras/imsdb/test-carddemo-execdli.sh
    ./extras/imsdb/test-carddemo-jcl.ps1

Adjust both mount paths if the checkout is elsewhere. The script uses the repository's built
GnuCOBOL compiler and `libcob`, then compiles the C shim and a COBOL program in a temporary
WSL directory. It loads Bank of Z's nine DBDs and 1,169 records, runs `GU`, `GHU`, `REPL`,
`IMSDBCOM`, and `GU` against `CUSTOMER`. A second process replaces the name without
committing; a third process verifies that the committed value remains. The test PSB is
`IBSCUDAT`, which grants update permission for `CUSTOMER`.

For other programs, compile `cbltdli.c` as a shared library linked with `libimsdb` and
`libcob`, set `COB_PRE_LOAD` to its path, and set `COB_IMS_WAL`, `COB_IMS_DBD_DIR`, and
`COB_IMS_PSB`. The caller must initialize the first eight bytes of its DB PCB mask with the
DBD name. `CBLTDLI` accepts a four-byte function, the PCB, an I/O area, and up to 16 SSAs.
`IMSDBCOM` commits the current UOW. By default process exit rolls back pending
changes. With `COB_IMS_AUTOCOMMIT=1` and this repository's rebuilt `libcob`, a
zero-return `STOP RUN` commits and a nonzero return rolls back. The shim accepts
native ASCII and GnuCOBOL IBM-037 control text. EBCDIC segment bytes remain raw.
The `imsbatch` runner explicitly completes the UOW from `cob_call`'s result so a
nonzero `GOBACK` rolls back as well. Set `COB_IMS_LINKAGE=DB` for originals
whose procedure division takes only a DB PCB; the default is `IO,DB`.

For a JCL launch and persistence smoke test, run from PowerShell:

    ./extras/imsdb/test-dfsrrc00.ps1

This builds an Ubuntu 24.04 integration image, rebuilds `libcob`, and runs a synthetic `DFSRRC00` JCL step
against the Bank of Z sample data. The profile in
`extras/jclsh/tests/ims-dfsrrc00.conf` maps `program.IMSSMOKE`,
`ims_psb.IBSCUDAT`, `ims_dbd_dir`, `ims_wal`, and `ims_cbltdli`. The script checks
`plan`, program RC 0, automatic commit without `IMSDBCOM`, and a fresh-process readback.

## Build and inspect reference definitions

Use PowerShell from the repository root:

    cargo test --manifest-path extras/imsdb/Cargo.toml --offline
    cargo test --manifest-path extras/imsdb/Cargo.toml --offline --test reference -- --ignored
    cargo build --manifest-path extras/imsdb/Cargo.toml --offline
    $bank = @(Get-ChildItem reference/Bank-of-Z-main/src/base/ims/DBD -File | ForEach-Object FullName) +
            @(Get-ChildItem reference/Bank-of-Z-main/src/base/ims/PSB -File | ForEach-Object FullName)
    $card = @(Get-ChildItem reference/aws-mainframe-modernization-carddemo/app/app-authorization-ims-db2-mq/ims -File | ForEach-Object FullName)
    & ./extras/imsdb/target/debug/imsdbctl.exe inspect ($bank + $card)

The inspection currently recognizes all 13 reference DBDs and all 12 PSBs. It validates
DB PCB segment names against the loaded DBDs. The ignored reference tests exercise Bank
of Z CUSTOMER updates and ACCOUNT/HISTORY atomic commit, plus CardDemo parent-child
insert and delete with their original DBD/PSB files. A single root can be inserted and fetched
with imsdbctl put-root WAL DBD SEGMENT RECORD_HEX and
imsdbctl get-root WAL DBD SEGMENT KEY_HEX. The Rust API supports child records and
multi-record transactions.

For a file of consecutive, fixed-length root segments, import and export the raw bytes:

    & ./extras/imsdb/target/debug/imsdbctl.exe import-fixed data.wal path/to/DBD ROOT input.bin
    & ./extras/imsdb/target/debug/imsdbctl.exe export-fixed data.wal path/to/DBD ROOT output.bin

The segment must have a DBD sequence field and a single fixed length. Import rejects a partial
final record and duplicate unique keys; all imported records commit in one unit of work.
Export writes records in sequence-key order, so a differently ordered input may not be
byte-for-byte identical as a whole file. Individual segment bytes are preserved, including
EBCDIC and packed or binary fields. This command handles raw fixed records, not the CSV
`LoadData/*.data` files in Bank of Z.

For DBDs with `DATATYPE` on each non-overlapping field, `import-csv` converts one CSV file
to a root segment. `import-csv-bundle` reads every `.asm` or `.dbd` file in a directory,
matches `<DBD-name>.data` in a data directory, and commits all records in one transaction:

    & ./extras/imsdb/target/debug/imsdbctl.exe import-csv-bundle bank.wal `
        reference/Bank-of-Z-main/src/base/ims/DBD `
        reference/Bank-of-Z-main/src/base/ims/LoadData

This imports the 9 Bank of Z DBDs and 1,169 sample records together. `INT` and `LONG` use
z/OS big-endian binary layout, `DECIMAL` uses signed packed decimal, and text in a `Cp1047`
DBD uses the GnuCOBOL default EBCDIC translation table. CSV text outside ASCII is rejected
until an explicit Japanese/multibyte codec is provided. The single-file CLI commands open
only one DBD per WAL and cannot inspect a multi-DBD bundle; the Rust API can use the
bundle with multiple PSBs.

For the original CardDemo `DFSURGU0` VB unload, use `imsdbctl import-dfsurgu0 WAL DBD FILE`.
The strict reader checks RDWs, header/trailer markers, DFSURGUF segment names and lengths,
then loads the original bytes and parent-child order in one transaction. The supported
variant is non-HALDB HDAM/HIDAM with one root and direct children. It does not materialize
the secondary index DBD. `split-dfsurgu0 DBD FILE ROOT_OUT CHILD_OUT` creates fixed-length
files for the original `PAUDBLOD.CBL` loader. `test-carddemo-load.sh` runs it unchanged and
reads back 22 roots and 202 children in a separate process. `test-carddemo-unload.sh` checks
that original `CBPAUP0C` returns DJ for its DLET after an intervening GNP on the same PCB,
and that the process rollback keeps all 224 segments.
`test-carddemo-roundtrip.sh` runs unchanged `PAUDBUNL.CBL` and compares its
fixed-record output byte-for-byte. The source unload's final root has an
all-space packed key, so the original `IS NUMERIC` condition writes 21 of 22
roots and all 202 children.
`test-carddemo-jcl-cycle.ps1` also runs the unchanged `UNLDPADB.JCL` and
`LOADPADB.JCL` through `jclsh`; the second step persists 21 roots and 202
children in a separate WAL. `imsdbctl count WAL DBD` reports the total.

## Remaining work

The storage format and API are experimental. Only one process may open a given WAL; the
planned imsdbd service will own it and coordinate concurrent COBOL/batch workers.
One transaction is limited to a 64 MiB WAL frame; larger imports must be split into
separately validated batches once a batch loader is available.
The current root lock manager is in-process and uses ordered acquisition; it does not
yet support shared locks or deadlock victim selection. The proposal's performance gates
have not been measured. There is no page cache, secondary-index materialization,
general child-segment export utility, complete failed-call PCB positioning, or
integration with PLITDLI. CICS can call `IMSDB_END_TASK` from its unit-of-work
coordinator when built with `COBCICS_IMSDB`; the Bank customer commit/rollback
path is tested through that coordinator. Cross-resource two-phase commit is
not implemented. The EXEC DLI translator covers the forms in four CardDemo
sources; the three online/MQ programs compile with CICS translation. The
batch `CBPAUP0C` runs to completion against an empty database. With the correctly
decoded 224-segment DFSURGU0 unload it reaches DJ on an invalid parent DLET after
GNP; the resulting rollback is covered by the integration test. The current
`CBLTDLI` shim implements local file-backed I/O PCB `GU`/`ISRT` and basic batch
`CHKP`/`ROLB`; it does not provide a live IMS TM queue, alternate PCBs,
variable-length GSAM, Japanese EBCDIC control text,
automatic program restart, or a complete IMS program lifecycle. Basic checkpoint
IDs are durable, but the application must implement restart positioning itself.
The application must select
a DB PCB by DBD name. `DFSRRC00` launches a profile-registered executable;
the profile must set the batch runner's IMS linkage mode explicitly.
Qualified GN and unsuccessful-call positioning need further IBM semantics tests.
The GSAM RSA is a local eight-byte big-endian record ordinal and is not an IMS
physical RSA. A GSAM output append is synced at commit, but it is not atomic with
the IMS DB WAL commit or with other GSAM output files.
