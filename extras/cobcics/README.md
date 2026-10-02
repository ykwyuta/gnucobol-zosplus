# cobcics prototype

This crate starts a small TN3270 host in Rust. It reads a BMS source file,
renders one 24x80 map to a 3278/3279 model 2 terminal, and accepts AID keys
and modified, unprotected fields. `cobcics-mapgen` creates a fixed-format
GnuCOBOL symbolic map copybook from the same BMS model.

## Build and try

From the repository root:

```sh
mkdir -p build
cargo test --manifest-path extras/cobcics/Cargo.toml --offline
cargo run --manifest-path extras/cobcics/Cargo.toml --bin cobcics-mapgen -- \
  reference/Bank-of-Z-main/src/base/cics/bms/BNK1MAI.bms build/BNK1MAI.cpy
cargo run --manifest-path extras/cobcics/Cargo.toml --bin cobcicsd -- \
  --bms reference/Bank-of-Z-main/src/base/cics/bms/BNK1MAI.bms \
  --listen 127.0.0.1:3270
```

Connect c3270 or wc3270 to `N:127.0.0.1:3270` with model `3279-2`.
The `N:` prefix disables TN3270E, which this host does not yet implement.

### Bank of Z syntax audit

From the repository root, after building the `gnucobol-db2-build` Docker
image, run:

```powershell
python extras/cobcics/audit-bank-syntax.py
```

The audit generates all 10 BMS copybooks, translates CICS and SQL, compiles
all 32 Bank of Z COBOL sources to objects, and exercises the channel and lock
bridge. Compiler logs and objects are written to `build/bank-syntax`.
IMS calls are outside this audit; object compilation does not execute or link
the complete Bank application.

### Bank of Z menu and next screens on Windows

From PowerShell in the repository root, with the local `gnucobol-db2-build`
and `rust:1-alpine` Docker images available:

```powershell
& extras/cobcics/build-bank-menu.ps1
docker run -d --rm --name bank-menu-demo -p 127.0.0.1:3270:3270 `
  -e LICENSE=accept -e LD_LIBRARY_PATH=/src/build/libcob/.libs `
  --entrypoint sh -v "${PWD}:/src" -w /src gnucobol-db2-build -lc `
  'exec build/bank-demo/cobcicsd --bms reference/Bank-of-Z-main/src/base/cics/bms/BNK1MAI.bms --transid OMEN --applid BANKAPP --route OMEN=build/bank-demo/BNKMENU --route ODAC=build/bank-demo/BNK1DAC --map-route ODAC=reference/Bank-of-Z-main/src/base/cics/bms/BNK1DAM.bms --route OTFN=build/bank-demo/BNK1TFN --map-route OTFN=reference/Bank-of-Z-main/src/base/cics/bms/BNK1TFM.bms --listen 0.0.0.0:3270'
& 'C:\Program Files\wc3270\wc3270.exe' +S -model 3279-2 `
  -tn IBM-3279-2 -charset cp037 -trace `
  -tracefile build/bank-demo/wc3270-trace.txt `
  -scriptport 127.0.0.1:33271 N:127.0.0.1:3270
```

At the menu, enter `2` for Display Account or `7` for Transfer Funds and
press Enter. PF3 on either screen returns to the menu. To run the scripted
wc3270 checks against the command above, run
`python extras/cobcics/test-wc3270.py --account-transition` and
`python extras/cobcics/test-wc3270.py --transfer-transition`.
`--map-route ID=BMS-FILE` loads the physical map for the target transaction;
the host switches back to the menu map on `RETURN TRANSID('OMEN') IMMEDIATE`.

These two destination screens are verified for display and PF3 navigation.
Entering account or transfer details still reaches Bank of Z business logic
whose linked `INQACC`, `DELACC`, and `XFRFUN` programs are not yet built into
this demo. Other menu destinations are not yet compiled into this demo.

Enter an Action value and press Enter, Clear, PF3, or PF12. A demo worker
may be supplied with `--worker PATH`; it is launched once for the initial
display and again for each AID. The worker
uses `bridge.c` to exchange versioned, length-delimited binary messages over
stdin/stdout. It reads the AID, previous TRANSID, configured APPLID, COMMAREA,
and modified fields,
then returns named output fields and the next TRANSID/COMMAREA. The Rust host
keeps this state within one terminal session and validates names and lengths before
rendering them. `menu-worker.cob` shows the C ABI directly.
The request pipe format is `CCI3` and the response format is `CCO5`.
The response screen action selects full map,
screen erase, a data-only field update with optional alarm, text output, or no screen change.
Its IMMEDIATE flag starts the returned TRANSID without waiting for another AID.
The host limits consecutive immediate tasks to 32.
Use `--transid OMEN --route OMEN=PATH` to select the initial transaction
and its worker; repeat `--route ID=PATH` for other transaction IDs. The
saved TRANSID selects the worker after each AID. `--worker PATH` is a
fallback for IDs without a route.
`--applid BANKAPP` sets the eight-character CICS region identifier reported by
`ASSIGN APPLID`; the default is `COBCICS`. `ASSIGN PROGRAM` reports the worker's
`PROGRAM-ID`, which must fit in eight characters for this command
([IBM's ASSIGN reference](https://www.ibm.com/docs/en/cics-ts/6.x?topic=summary-assign)).

`bin/cics-preprocess.py` translates the first `EXEC CICS` subset. The generated
copybook filename must match the mapset name. For the menu demo:

```sh
python bin/cics-preprocess.py \
  extras/cobcics/menu-worker-cics.cob build/BNK1MAI.cpy \
  build/menu-worker-cics.cob
python bin/cics-preprocess.py \
  extras/cobcics/menu-linked-cics.cob build/BNK1MAI.cpy \
  build/menu-linked-cics.cob
python bin/cics-preprocess.py \
  extras/cobcics/menu-immediate-cics.cob build/BNK1MAI.cpy \
  build/menu-immediate-cics.cob
cobc -x -I build -I extras/cobcics/copy -o build/menu-worker-cics \
  build/menu-worker-cics.cob build/menu-linked-cics.cob \
  extras/cobcics/bridge.c
cobc -x -I build -I extras/cobcics/copy -o build/menu-immediate-cics \
  build/menu-immediate-cics.cob extras/cobcics/bridge.c
cargo run --manifest-path extras/cobcics/Cargo.toml --bin cobcicsd -- \
  --bms reference/Bank-of-Z-main/src/base/cics/bms/BNK1MAI.bms \
  --transid OMEN --applid BANKAPP \
  --route OMEN=build/menu-worker-cics \
  --route ODCS=build/menu-immediate-cics
```

The preprocessor currently accepts `RECEIVE MAP` with literal `MAP`/`MAPSET`
and optional `INTO`, `SEND MAP` with literal `MAP`/`MAPSET` and optional `FROM`,
and `RETURN` with optional literal `TRANSID`, paired `COMMAREA`/`LENGTH`,
and `IMMEDIATE` when `TRANSID` is present. `IMMEDIATE` runs the next worker
with `EIBAID` zero and the returned COMMAREA. A task that does not issue SEND
leaves the terminal screen unchanged.
`ASSIGN APPLID(...)` and `ASSIGN PROGRAM(...)` return eight-character values.
Other `ASSIGN` options fail with the original source line.
`LINK PROGRAM(...) COMMAREA(...)` calls a COBOL program in the same worker,
passing the COMMAREA by reference. `LENGTH` can be supplied or defaults to the
area's size. The linked program uses `PROCEDURE DIVISION USING DFHCOMMAREA`;
its `RETURN` resumes the caller. Both programs must be linked into the worker.
`BIF DEEDIT FIELD(...) LENGTH(...)` currently removes non-ASCII digits,
right-aligns the result, and zero-fills the rest. Zoned sign conversion is
not yet implemented.
`LINK SYNCONRETURN` uses local `LINK` behavior when no remote region is
configured: the called program runs in the caller's task, and the link itself
does not commit Db2 work. This follows [IBM's LINK definition](https://www.ibm.com/docs/en/cics-ts/6.x?topic=summary-link),
which ignores `SYNCONRETURN` for a local link. A worker linked with
`COBCICS_DB2SQL` and `libdb2sql` commits its active Db2 connection at normal
task `RETURN` and rolls it back on bridge failure or `ABEND`. Remote DPL
(`SYSID` or a remote program definition) remains unsupported and is rejected
by the preprocessor. The Db2 transaction test in
`test-sync-link-db2.py` verifies commit at task end, rollback after an
ABEND following the local link, and explicit `SYNCPOINT` boundaries.
With the Compose Db2 and `DB2SQL_DEMO` table from
[the Db2 setup](../db2sql/README.md) running, execute
`& extras/cobcics/test-sync-link-db2.ps1` in PowerShell to regenerate the
COBOL and SQL translations, build the linked worker, and run that test.
Remote LINK, channels, and LINK without COMMAREA are not yet supported
([IBM's LINK reference](https://www.ibm.com/docs/en/cics-ts/6.x?topic=summary-link)).
`SEND CONTROL ERASE FREEKB` sends an Erase/Write record that clears the screen
and unlocks the keyboard.
`SEND TEXT FROM(...) ERASE FREEKB` sends a protected text field on an erased
screen and supports `RESP`/`RESP2`. The COBOL data item's length is used when
`LENGTH` is omitted, as in the Bank of Z termination message. The current
text subset accepts at most 1919 encoded EBCDIC bytes of printable text and does not implement
BMS text pagination or word wrapping
([IBM SEND TEXT](https://www.ibm.com/docs/en/cics-ts/6.x?topic=output-send-text-command)).
`ASKTIME ABSTIME(...)` returns the host's local time as milliseconds since
1900-01-01 in an eight-byte packed decimal field. `FORMATTIME` accepts that
field with `DDMMYYYY(...)`, `TIME(...)`, and optional `DATESEP` (default `/`
when present without a value). The implemented time format is six digits
`hhmmss`, with no `TIMESEP`. These commands follow the Bank of Z menu's data
items ([IBM ASKTIME](https://www.ibm.com/docs/en/cics-ts/6.x?topic=summary-asktime),
[IBM FORMATTIME](https://www.ibm.com/docs/en/cics-ts/6.x?topic=summary-formattime)).
`ABEND` accepts optional `ABCODE`, `NODUMP`, and `CANCEL`. `HANDLE ABEND
LABEL(...)` transfers an explicit `ABEND` in the same program to its exit;
`ASSIGN ABCODE(...)` reads that code. `HANDLE ABEND CANCEL` disables the exit,
and `HANDLE ABEND RESET` re-enables it. `ABEND CANCEL` bypasses the exit and
terminates the worker. The synthetic `test-abend-handler.ps1` checks the exit,
the code, and cancellation at runtime. Dump creation and trapping failures
that originate outside an explicit `EXEC CICS ABEND` are not implemented.
Db2 rollback on an unhandled ABEND is verified in the local link test
([IBM ABEND](https://www.ibm.com/docs/en/cics-ts/6.x?topic=summary-abend)).
`SYNCPOINT` commits the active Db2 unit of work and `SYNCPOINT ROLLBACK`
backs it out. Both close the current Db2 connection so later SQL starts a new
unit of work; `RESP` and `RESP2` receive the result. The Db2 test covers both
forms, including a commit that survives a later ABEND. `DELAY FOR SECONDS(n)`
supports literal values from 0 through 60. The Bank error handler's
`WRITE FILE('ABNDFILE') FROM(...) RIDFLD(...)` appends a two-byte big-endian
length followed by the record to the path in `COBCICS_ABEND_LOG`. It checks
that the 12-byte key matches the record prefix. If the path is unset or cannot
be written, it returns a nonnormal `RESP`. This diagnostic log does not provide
VSAM keyed lookup, duplicate-key detection, or Db2 transaction rollback.

To build the Bank menu, account inquiry/deletion, transfer, and error-handler
workers together with Db2 support, run
`& extras/cobcics/build-bank-business.ps1`. The outputs are under
`build/bank-business`. Run `& extras/cobcics/test-abend-handler.ps1` for the
error-path test. The TN3270 transition test can be run in the Db2 build image:

```powershell
docker run --rm --env-file extras/db2sql/.runtime.env -e LICENSE=accept `
  -e COB_HOST_CODESET=UTF-8 `
  -e 'LD_LIBRARY_PATH=/src/build/libcob/.libs:/src/build/bank-business:/usr/local/lib64/python3.9/site-packages/clidriver/lib:/usr/local/lib64/python3.9/site-packages/ibm_db.libs' `
  --entrypoint sh -v "${PWD}:/src" -w /src gnucobol-db2-build -c `
  'python3 extras/cobcics/test-menu-transition.py build/bank-business/cobcicsd reference/Bank-of-Z-main/src/base/cics/bms/BNK1MAI.bms reference/Bank-of-Z-main/src/base/cics/bms/BNK1DAM.bms build/bank-business/BNKMENU build/bank-business/BNK1DAC reference/Bank-of-Z-main/src/base/cics/bms/BNK1TFM.bms build/bank-business/BNK1TFN'
```

The original Bank of Z account and transfer SQL also requires its `ACCOUNT`
and `PROCTRAN` tables and sample data. The transition test verifies the
compiled workers and 3270 navigation; it does not execute those business SQL
paths.
To open this build with wc3270 without replacing the earlier menu demo, use
port 3271:

```powershell
docker run -d --rm --name bank-business-demo `
  --env-file extras/db2sql/.runtime.env -e LICENSE=accept `
  -e COB_HOST_CODESET=UTF-8 `
  -e COBCICS_ABEND_LOG=/src/build/bank-business/abend-records.bin `
  -e 'LD_LIBRARY_PATH=/src/build/libcob/.libs:/src/build/bank-business:/usr/local/lib64/python3.9/site-packages/clidriver/lib:/usr/local/lib64/python3.9/site-packages/ibm_db.libs' `
  -p 127.0.0.1:3271:3270 --entrypoint sh `
  -v "${PWD}:/src" -w /src gnucobol-db2-build -c `
  'exec build/bank-business/cobcicsd --bms reference/Bank-of-Z-main/src/base/cics/bms/BNK1MAI.bms --transid OMEN --applid BANKAPP --route OMEN=build/bank-business/BNKMENU --route ODAC=build/bank-business/BNK1DAC --map-route ODAC=reference/Bank-of-Z-main/src/base/cics/bms/BNK1DAM.bms --route OTFN=build/bank-business/BNK1TFN --map-route OTFN=reference/Bank-of-Z-main/src/base/cics/bms/BNK1TFM.bms --listen 0.0.0.0:3270'
& 'C:\Program Files\wc3270\wc3270.exe' +S -model 3279-2 N:127.0.0.1:3271
```
`SEND MAP` accepts `ERASE`, `DATAONLY`, `ALARM` with `DATAONLY`, and `FREEKB`.
`DATAONLY` sends only the output fields changed by the worker, preserving the
terminal's existing field attributes. `ALARM` sets the 3270 WCC alarm bit
([IBM's output data stream guide](https://www.ibm.com/docs/en/cics-ts/5.5.0?topic=terminals-output-datastream)).
These commands accept `RESP`/`RESP2` data names. `DFHRESP(NORMAL)` and
`DFHRESP(MAPFAIL)` expand in COBOL code. `RECEIVE MAP` reports MAPFAIL (36)
for Clear, PA keys, or input with no modified fields; without `RESP`/`RESP2`
the worker terminates with an unhandled condition. This is a basic subset of
[IBM's MAPFAIL behavior](https://www.ibm.com/docs/en/cics-ts/5.6.0?topic=data-mapfail-other-exception-conditions):
named-field cursor exceptions and BMS field tags are not yet modeled.
Other commands and options are diagnosed with the source line. It inserts a
startup call and `EIBAID`, `EIBCALEN`, `EIBTRNID`, and COMMAREA data
items. `EIBTASKN` is populated from the worker process ID as a temporary task
number. `ASKTIME` does not yet update `EIBDATE` and `EIBTIME`. The current
`SEND MAP` overlays named fields containing nonzero bytes;
low-values retain BMS initial data.

## Current coverage

- Telnet BINARY, TERMINAL-TYPE, EOR negotiation and record framing;
  fragmented input and IAC escaping.
- Basic 3270 Erase/Write and Write, SBA, SF, IC, CP037 or Japanese EBCDIC text, Enter/Clear/PF and
  modified-field parsing.
- BMS mapset, map, named/unnamed fields, attributes needed for basic
  input, continuation lines, `TIOAPFX`, `DSATTS`, `OCCURS`, and `PICOUT`.
- COBOL symbolic input `L/F/A/I` and output `O` plus extended attribute
  subfields; `OCCURS` groups.
- An unrecognized BMS option or unsupported `EXEC CICS` command is an error
  with its source line.

Extended field colors, highlighting, outline and programmed symbols appear
in the symbolic copybook but are not yet encoded as extended field attributes
in the terminal stream. Japanese unprotected fields use SFE input control for
automatic SO/SI creation. `--charset` selects CP037 (default),
IBM-930, IBM-939, IBM-1390, or IBM-1399; Japanese text uses SO/SI in the
terminal stream and UTF-8 in the worker pipe. Run
`extras/cobcics/test-japanese-tn3270.ps1` for an IBM-1399 COBOL round trip
or `extras/cobcics/test-japanese-wc3270.ps1` for an IBM-939 round trip using
the installed Windows wc3270 emulator.
The command ABI is an initial subset: EIB fields other than `EIBAID`,
`EIBCALEN`, `EIBTRNID`, `EIBRESP`, and `EIBRESP2`, Db2 unit of work,
broader CICS response codes and conditions,
full BMS SEND/RECEIVE options and TN3270E are not yet implemented.
The session COMMAREA is limited to 2048 bytes. The prototype exposes it in
`COBCICS-COMMAREA`; it does not yet populate a program's `DFHCOMMAREA` linkage
item. Routes currently select executable paths supplied at startup; CICS
resource definitions and dynamic program loading are not implemented.
The preprocessor is also available from `cobc --cics-map=build/BNK1MAI.cpy`.
When combined with `--db2-sql`, CICS conversion runs before SQL conversion.
When compiling from a source checkout, set `COB_CICS_PREPROCESSOR` to the
absolute path of `bin/cics-preprocess.py` unless it is on `PATH`. Add the
map copybook and `extras/cobcics/copy` directories with `-I`, and link
`bridge.c` into the worker. The latter directory contains the supported
`DFHAID` AID constants. The combined driver order has been checked at the
generated-source level. The local linked-worker test also verifies a live Db2
INSERT, task-end commit, and ABEND backout. The original Bank of Z business
programs remain to be compiled and exercised.

```sh
COB_CICS_PREPROCESSOR="$PWD/bin/cics-preprocess.py" cobc \
  --cics-map=build/BNK1MAI.cpy -x -I build -I extras/cobcics/copy \
  -o build/menu-worker-cics extras/cobcics/menu-worker-cics.cob \
  extras/cobcics/menu-linked-cics.cob \
  extras/cobcics/bridge.c
```
The worker reserves stdout for its binary protocol; COBOL diagnostics
should go to stderr. The original Bank of Z `BNKMENU` has also been checked
with `wc3270` 4.0ga14 on Windows, using model `3279-2`, code page 037, and
the `N:` host prefix. Its script port returned the main menu and the invalid
Action message; the terminal trace confirmed `Session Ended` after PF3.
`test-wc3270.py` repeats these checks against a running `wc3270` instance
started with `-scriptport` and, for the PF3 screen assertion, `-trace`.
The protocol and COBOL integration tests also use a small scripted TN3270
client. The Linux host executable used in Docker was built in the
`rust:1-alpine` image.

See [the proposal](../../docs/proposal/cics-rust-tn3270.md) for the target
architecture and remaining milestones.
