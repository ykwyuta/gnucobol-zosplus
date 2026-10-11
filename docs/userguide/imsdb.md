# IMS DB コアと Bank of Z データ投入（開発中）

Rust 製 `imsdb` は DBD/PSB の解析、階層セグメントの保管、DL/I の基本的な取得・更新、UOW とルートロックを実装中です。Rust の CALL アダプターでは DB PCB の9フィールドに状態・セグメント名・キー情報を返し、EBCDIC の関数名と SSA を処理します。GnuCOBOL の `CBLTDLI` から DB PCB と GSAM PCB の基本操作、バッチ用 `CHKP`／`ROLB` を呼べます。`jclsh` は `DFSRRC00` の登録済みプログラムを起動できます。CardDemo で使う `EXEC DLI` は前処理器で接続しました。PL/I の `PLITDLI` は未対応です。

## ビルドとデータ投入

リポジトリのルートから PowerShell で実行します。

```powershell
cargo build --manifest-path extras/imsdb/Cargo.toml --offline
& ./extras/imsdb/target/debug/imsdbctl.exe import-csv-bundle bank.wal `
    reference/Bank-of-Z-main/src/base/ims/DBD `
    reference/Bank-of-Z-main/src/base/ims/LoadData
```

このコマンドは Bank of Z の DBD 9本と `.data` 9本を読み、1,169件を同じ WAL に一括投入します。各ファイルを変換・検証してからトランザクションを開始し、重複キーなどで失敗した場合は投入した全レコードをロールバックします。既存の同じデータを再投入すると重複キーで失敗します。

CSV の列順は DBD の重複しないフィールド順です。`INT` と `LONG` は z/OS 形式のビッグエンディアン、`DECIMAL` は符号付きパック十進、`ENCODING=Cp1047` の文字項目は GnuCOBOL の既定 EBCDIC 変換表で符号化します。現在の CSV 変換は ASCII の入力文字に限ります。ロード後のセグメント本文は文字列に変換せず、バイト列のまま保管します。

単一 DBD の CSV は `import-csv WAL DBD SEGMENT FILE`、すでに固定長の生レコードになっているファイルは `import-fixed WAL DBD SEGMENT FILE` で投入できます。`export-fixed WAL DBD SEGMENT FILE` は固定長ルートセグメントをキー順で書き出します。これら単一 DBD 用コマンドは、複数 DBD を格納した `bank.wal` には使えません。複数 DBD の参照は現在 Rust API から行います。

CardDemo の IMS 原本アンロードは `DFSURGU0` が出力した VB レコードです。`imsdbctl import-dfsurgu0 WAL DBD FILE` で、ヘッダー・トレーラー、親子関係、レコード長とセグメント名を検証してから投入します。対象は現在、非 HALDB の HDAM／HIDAM で、親1種とその直接の子に限ります。CardDemo の `DBPAUTP0` では次のように実行します。

```powershell
& ./extras/imsdb/target/debug/imsdbctl.exe import-dfsurgu0 card.wal `
    reference/aws-mainframe-modernization-carddemo/app/app-authorization-ims-db2-mq/ims/DBPAUTP0.dbd `
    reference/aws-mainframe-modernization-carddemo/app/app-authorization-ims-db2-mq/data/EBCDIC/AWS.M2.CARDDEMO.IMSDATA.DBPAUTP0.dat
```

原本には親22件・子202件が含まれます。`imsdbctl split-dfsurgu0 DBD FILE ROOT_OUT CHILD_OUT` は同じアンロードを原本 `PAUDBLOD.CBL` 用の固定長入力ファイルに分けます。`test-carddemo-load.sh` は、このロードプログラムを無修正で実行し、親22件・子202件を読み戻します。`test-carddemo-roundtrip.sh` は無修正 `PAUDBUNL.CBL` の出力をバイト単位で照合します。親の最終1件は packed decimal キーが空白のため、原本の `IS NUMERIC` 条件で除外されます。出力は親21件・子202件です。`test-carddemo-unload.sh` は無修正 `CBPAUP0C` が親削除時に `DJ` で停止し、変更がロールバックされることを確認します。これは同じPCBで子を読んだ後に親を `DLET` する原本の呼出順序によるものです。レコード本文は EBCDIC、packed decimal、二進数を含めて生バイトを保持します。

CardDemo の `DBPAUTX0` は、`DBPAUTP0` と `DBPAUTX0` の両DBDを開いたとき、親の `ACCNTID` から二次索引をメモリ上に構築します。挿入・更新・削除の確定時に索引を更新し、WAL再読込時にも再構築します。`imsdbctl get-secondary WAL DBD_DIR DBPAUTX0 KEY_HEX` は対象の親セグメントIDと本文の16進数を出力します。`DBD_DIR` には両DBDを含むディレクトリを指定します。CardDemo の `LCHILD INDEX=ACCNTID` が直接フィールドを指す形に加え、[IBM IMS の XDFLD 定義](https://www.ibm.com/docs/en/ims/15.6.0?topic=indexes-example-defining-secondary)の `SEGMENT`、1～5個の通常の `FIELD` を連結する `SRCH`、省略可能な `SUBSEQ` を扱います。子セグメントのフィールドをキーにして親を照会する場合も、対象の親を返します。

DB PCB の `PROCSEQ=索引DBD名` で、[IBM の二次処理順序 PCB](https://www.ibm.com/docs/en/ims/15.6.0?topic=statements-full-function-fast-path-database-pcb-statement)と同様に対象ルートを索引順で取得できます。現在は対象DBDに対する索引定義が1個、索引対象がルートセグメントの場合に限ります。`GU` は索引フィールド名の等値 SSA、`GN` は索引順の逐次取得を扱います。索引位置付け後の `GNP` は元の階層の子を取得します。`KEYLEN` は `SUBSEQ` を含む索引ポインターキー全長以上に設定してください。例:

```text
 PCB TYPE=DB,DBDNAME=DBPAUTP0,PROCOPT=G,PROCSEQ=DBPAUTX0,KEYLEN=6
 SENSEG NAME=PAUTSUM0,PARENT=0
 PSBGEN PSBNAME=IDXPAUT,LANG=COBOL
 END
```

`COB_IMS_DBD_DIR` に対象と索引の両DBDを含むディレクトリ、`COB_IMS_PSB` にこのPSBを指定します。COBOL から `CBLTDLI` の `GU` に `PAUTSUM0(ACCNTID EQ<6バイト値>)` 形式の SSA を渡せます。IBM-037 モードでは SSA の制御文字を復号し、索引値の生バイトを保持します。同じ UOW の未確定更新も索引取得に反映します。

[IBM の索引DBD直接 PCB](https://www.ibm.com/docs/en/ims/15.6.0?topic=indexes-processing-secondary-index-as-separate-database)の読出しも、ポインターセグメントが固定長で**全体がキー**の場合に対応します。CardDemo では次の PSB を指定し、`CBLTDLI` の `GN` で `PAUTINDX` の6バイトのポインター本文を順に取得します。`GU` の修飾 SSA は `PAUTINDX(INDXSEQ EQ<6バイトの完全キー>)` とします。`SRCH` と `SUBSEQ` を持つ索引では両方を連結した完全キーが必要です。直接 PCB からの更新と、ポインター本文にユーザーデータや重複データを持つ索引は未対応です。

```text
 PCB TYPE=DB,DBDNAME=DBPAUTX0,PROCOPT=G,KEYLEN=6
 SENSEG NAME=PAUTINDX,PARENT=0
 PSBGEN PSBNAME=PTRPAUT,LANG=COBOL
 END
```

通常の物理順 PCB でも、[IBM の `SENSEG INDICES=`](https://www.ibm.com/docs/en/ims/15.6.0?topic=statements-senseg-statement)で許可したルート索引フィールドの等値 SSA を `GU` に指定できます。この経路の `GN` は物理順を維持します。現在は索引対象がルートである場合の `GU` に限り、索引フィールドで修飾した `GN` は対象外です。

```text
 PCB TYPE=DB,DBDNAME=DBPAUTP0,PROCOPT=G,KEYLEN=6
 SENSEG NAME=PAUTSUM0,PARENT=0,INDICES=DBPAUTX0
 PSBGEN PSBNAME=PHYPAUT,LANG=COBOL
 END
```

未確定変更があるときの索引読出しは現在メモリ内の状態を複製して再構築するため、大量更新を挟む逐次読出しの性能は未測定です。`NULLVAL`、システム関連フィールド、複数索引エントリーは未対応です。

原本の `UNLDPADB.JCL` と `LOADPADB.JCL` を続けて動かす場合は `./extras/imsdb/test-carddemo-jcl-cycle.ps1` を実行します。前者が書いた親21件・子202件の固定長ファイルを後者が読み、別の IMS WAL に計223件を投入します。`jclsh` のプロファイルで `COB_IMS_LINKAGE=DB`／`IO,DB` を選びます。`imsdbctl count WAL DBD` は確定済みセグメント総数を出力します。

## 検証

```powershell
cargo test --manifest-path extras/imsdb/Cargo.toml --offline
cargo test --manifest-path extras/imsdb/Cargo.toml --offline --test reference -- --ignored
```

後者は Bank of Z と CardDemo の参照資材を必要とし、Bank of Z の全9データファイルの投入、IBTRAN PSB での口座照会、ACCOUNT/HISTORY の UOW、CardDemo の親子セグメント操作を検証します。実装範囲と残作業は [crate の README](../../extras/imsdb/README.md) と [提案書](../proposal/imsdb-rust.md) を参照してください。

## GnuCOBOL の CBLTDLI 結合試験（WSL）

Linux 版の Rust ライブラリと CLI を Docker でビルドしてから、WSL の GCC と、このリポジトリでビルドした GnuCOBOL を使います。PowerShell で実行します。

```powershell
docker run --rm --network none -v D:\workspace\gnucobol:/src -w /src `
    -e CARGO_TARGET_DIR=/src/build/imsdb-linux rust:1-bookworm `
    sh -lc 'PATH=/usr/local/cargo/bin:$PATH cargo build --manifest-path extras/imsdb/Cargo.toml --offline --lib --bin imsdbctl'
wsl --exec sh /mnt/d/workspace/gnucobol/extras/imsdb/test-cbltdli.sh
```

チェックアウト先が異なる場合は両方のパスを変更します。試験スクリプトは Bank of Z の9 DBD と1,169件を新しい WAL に投入し、`IBSCUDAT` PSB の更新権限を使って COBOL から `GU → GHU → REPL → IMSDBCOM → GU` を実行します。別プロセスで未コミット更新が破棄されることに加え、基本 `CHKP` による確定と `ROLB` による破棄も確認します。`test-carddemo-load.sh` は原本の親22件・子202件をロードした後、COBOL から `PROCSEQ=DBPAUTX0`、`SENSEG INDICES=DBPAUTX0`、`DBDNAME=DBPAUTX0` の3種類の PCB で取得を確認します。

個別プログラムでは [C shim](../../extras/imsdb/cbltdli.c) を `libimsdb`、`libcob` にリンクした共有ライブラリにし、`COB_PRE_LOAD` へ指定します。`COB_IMS_WAL`、`COB_IMS_DBD_DIR`、`COB_IMS_PSB` に WAL、DBD ディレクトリ、PSB ファイルを設定します。呼び出し側は DB PCB 領域の先頭8バイトに DBD 名を設定します。`CBLTDLI` は4バイトの関数名、DB PCB、I/O 領域、および最大16個の SSA を受け取ります。更新の確定には暫定 CALL `IMSDBCOM` も使用できます。通常はプロセス終了時に未確定更新を破棄します。`COB_IMS_AUTOCOMMIT=1` とこのリポジトリで再ビルドした `libcob` を使うと、RC 0 の `STOP RUN` で確定し、非ゼロ RC では破棄します。

現在の shim は ASCII と GnuCOBOL の IBM-037 制御文字に対応します。セグメント本文と SSA のバイナリ値はバイト列を保持し、必要な `COMP-5` のエンディアン変換は呼び出し境界で行います。I/O PCB は基本バッチ `CHKP`／`ROLB` と、ローカルファイルに対する `GU`／`ISRT` に対応します。入力ファイルは `DD_IMSINPUT`、出力ファイルは `DD_IMSOUTPUT` で指定し、各メッセージを2バイトのビッグエンディアン長、2バイトの制御領域、本文の順に連結します。`GU` は終端で `QC` を返し、`ISRT` は確定まで出力を保持します。`imsbatch.c` は `COB_IMS_ENTRY=DLITCBL`、`COB_IMS_DBD_NAME=CUSTOMER` などを指定して原本の I/O PCB と一つの DB PCB を引数として渡すバッチランチャーです。DB PCB のみを受け取る原本には `COB_IMS_LINKAGE=DB` を指定します。COBOL の `GOBACK` 後は `cob_call` の戻り値に従い IMS UOW を明示的に確定または取消します。`test-bank-original.sh` で Bank of Z の無修正 `IBSCUDAT.cbl` の顧客更新・応答出力を確認できます。基本 `CHKP` の8バイトIDはDB更新と同じWALフレームに保存され、`imsdbctl latest-checkpoint WAL DBD_OR_DIR` で確認できます。`DBD_OR_DIR` にはDBDファイルまたはDBDディレクトリを指定します。[IBM の基本 CHKP](https://www.ibm.com/docs/en/ims/15.6.0?topic=services-chkp-basic-call)と同様、アプリケーション側で再開位置を管理する必要があります。シンボリックチェックポイント、`XRST`、ライブの IMS TM キュー、代替 PCB は未対応です。

固定長 GSAM では CardDemo の `PASFLDBD`／`PADFLDBD` DBD と GSAM PCB を読み取り、`CBLTDLI` の `OPEN`／`CLSE`、`GN`、RSA を指定した `GU`、`ISRT` を実行できます。DBD の `DD1`／`DD2` に対応する `DD_<DD名>` 環境変数へ通常の固定長バイナリファイルを指定します。たとえば `PASFLDBD` では `DD_PASFILIP` と `DD_PASFILOP` です。`./extras/imsdb/test-gsam.sh` は WSL で元の `DLIGSAMP` PSB を使用して COBOL から順次取得・再取得・挿入を確認します。RSA はこの実装内でのみ使える8バイトのレコード番号であり、IBM IMS の物理 RSA と交換できません。出力は確定時に追記されますが、IMS DB WAL やほかの GSAM 出力との原子的な一括確定はまだできません。

CardDemo の固定形式 `EXEC DLI` は `cobc --ims-dli=DBPAUTP0` でコンパイルできます。ソースツリーから実行する場合は `bin` を `PATH` に追加するか、`COB_IMS_DLI_PREPROCESSOR` に [前処理器](../../bin/imsdli-preprocess.py)の絶対パスを指定します。前処理器は `GU/GN/GNP`、`ISRT/REPL/DLET`、`CHKP/SCHD/TERM` を明示的な呼び出しへ変換し、`DIBSTAT` に状態を返します。`GU/GN/GNP` は IBM の [EXEC DLI Get 規則](https://www.ibm.com/docs/en/ims/15.4.0?topic=programs-comparing-exec-dli-commands-dli-calls)に合わせて Hold 取得として実行します。`SCHD` は PSB 名を照合し、二重スケジュールに `TC` を返します。`TERM` は変更を確定して PSB を解放します。`test-carddemo-execdli.sh` は無修正の `CBPAUP0C.cbl` を変換・コンパイルし、空の `DBPAUTP0` に対する終端 `GB` と `CHKP` を確認します。`test-carddemo-jcl.ps1` は原本 `CBPAUP0J.jcl` を `jclsh` と Docker で実行し、SYSIN と RC 0 を確認します。`test-carddemo-online-build.sh` はオンライン／MQ 側の原本 `COPAUS0C`、`COPAUS1C`、`COPAUA0C` を `--cics-map` と `--ims-dli` を併用してコンパイルします。これら3本の画面・MQ業務フローは未実行です。

CICS ワーカーに IMS の確定・取消を接続する場合は `extras/cobcics/bridge.c` を `-DCOBCICS_IMSDB` でビルドし、`extras/imsdb/cbltdli.c` と Rust の `libimsdb` を同じワーカーへリンクします。`COBCICS_SYNCPOINT` とタスク終了時に IMS の保留更新が確定し、`SYNCPOINT ROLLBACK` と異常終了時に取り消されます。`test-cics-uow.sh` は Bank of Z の顧客レコードに対して両方向を別プロセス再読込で確認します。Db2、VSAM、IMS をまたぐ二相コミットは実装していないため、いずれかの確定が失敗した場合の全資源一括復旧は保証されません。

## JCL の DFSRRC00 から起動する

`jclsh` は `EXEC PGM=DFSRRC00,PARM='BMP,プログラム名,PSB名'` を解釈します。`DLI` と `DBB` も指定できます。プロファイルには `program.<プログラム名>`、`ims_psb.<PSB名>`、`ims_dbd_dir`、`ims_wal` を設定します。GnuCOBOL の `CBLTDLI` を使う場合は `ims_cbltdli` に共有ライブラリを設定します。JCL の DD は従来どおり `DD_<名前>` としてプログラムへ渡します。

```powershell
./extras/imsdb/test-dfsrrc00.ps1
```

このスクリプトは Docker 上で Linux 版と `libcob` をビルドし、[試験 JCL](../../extras/jclsh/tests/ims-dfsrrc00.jcl) と [プロファイル](../../extras/jclsh/tests/ims-dfsrrc00.conf)を使って Bank of Z の顧客更新を起動します。`plan`、ステップ RC 0、`IMSDBCOM` を呼ばない正常終了の自動確定、別プロセスからの再照会を確認します。現段階の `DFSRRC00` は登録済み実行ファイルを起動するもので、原本の IMS エントリーに I/O PCB と DB PCB のポインターを自動引き渡ししません。
