# jclsh

Rust 製 JCL インタプリタ兼対話シェルです。Bank of Z の原本 `BNKSTMT.jcl` を無編集で解析・計画・実行できます。複数ステップ、インストリーム／カタログド／入れ子の `PROC`、`JCLLIB ORDER` と `INCLUDE MEMBER`、`&symbol`、`SET`、PROC の `step.ddname` 上書き、`COND`、`IF` / `ELSE` / `ENDIF`、`PARM`、`JOBLIB`、連結 DD、`DD *` / `DD DATA,DLM=`、`SYSOUT=*` を扱います。`cics-genapp` の `cobol.jcl` は 93 ステップ、`db2cre.jcl` は同名ステップを含む 10 ステップに展開できます。

各ステップの直接 `PGM=` または `IKJEFT01` の `DSN RUN PROGRAM(...)` はプロファイルの `program.NAME=...` に登録します。未登録プログラムと未マッピングのデータセット DD は計画段階でエラーになります。`COND` は比較が真ならステップをスキップします。通常の非ゼロ RC 後も後続ステップを判定します。ABEND 後は通常ステップをスキップし、`COND=EVEN` / `ONLY` と `IF ABEND` を評価します。ジョブ RC は実行した通常ステップの最大 RC と ABEND のローカル表現 16 の最大値です。`result.json` は実行済み／スキップ／ABEND と各 RC・ABEND コードを記録します。

`PGM=DFSRRC00` は `PARM='BMP|DLI|DBB,プログラム名,PSB名'` を解析し、`program.NAME` に登録した実行ファイルを起動します。`ims_psb.NAME`、`ims_dbd_dir`、`ims_wal` と任意の `ims_cbltdli` をプロファイルに設定してください。このリポジトリで再ビルドした `libcob` と `CBLTDLI` shim を使う場合、正常終了で IMS 更新を確定し、非ゼロ RC では破棄します。元の IMS エントリーに PCB ポインターを自動で渡す機能はまだありません。[IMS DB の手順](../../docs/userguide/imsdb.md)に結合試験があります。

DD データセットはジョブ専用の一時ファイルへ割り当て、`DISP` の `NEW` / `OLD` / `SHR` / `MOD` と通常／異常終了時の `PASS` / `KEEP` / `CATLG` / `DELETE` / `UNCATLG` を処理します。永続データセットの変更・削除は `dataset_root` 内に登録したパスに限ります。`JOBLIB` / `STEPLIB` は計画に表示するメタデータです。`IF` は RC、RUN、ABEND、ABENDCC の判定と AND / OR / NOT を扱います。`SYSTEM`、`PLAN`、`LIB` はプロファイルとの照合で、Db2 for z/OS のバインド操作ではありません。`IEFBR14`、`IEBGENER`、`IDCAMS`、`SORT`、`DFHCSDUP` は Rust 内蔵の互換機能を呼び出します。[対象 JCL と制限](../../docs/userguide/jcl-utilities.md)を参照してください。このリポジトリに資材がない IBM 提供 PROC の実行は引き続き対象外です。

プロファイルの `proclib.データセット名=ディレクトリ` は `JCLLIB ORDER` に対応し、そのディレクトリのメンバー名または `メンバー名.jcl` を探索します。`dataset.データセット名=ファイル` は永続 DD の対応付け、`dataset_root=ディレクトリ` は変更可能な領域です。`site.名前=値` または CLI の `--set 名前=値` で原本にある `<名前>` を展開します。値を指定しないプレースホルダーは計画・実行時にエラーです。[設定例](tests/datasets.conf)を参照してください。

## 準備

リポジトリのルートで実行します。Docker Compose の Db2、`gnucobol-db2-build` イメージ、`extras/db2sql/.runtime.env` を用意してください。[PL/I の手順](../pl1c/README.md)に従い、`BNKSTMT.pli` の生成 C と `build/pl1c/BNKSTMT` をビルドし、`PL1TEST` の検証データを投入します。

```powershell
cargo run --manifest-path extras/jclsh/Cargo.toml --offline -- lint reference/Bank-of-Z-main/src/base/batch/jcl/BNKSTMT.jcl
cargo run --manifest-path extras/jclsh/Cargo.toml --offline -- plan reference/Bank-of-Z-main/src/base/batch/jcl/BNKSTMT.jcl --profile extras/jclsh/bank.conf
cargo run --manifest-path extras/jclsh/Cargo.toml --offline -- run reference/Bank-of-Z-main/src/base/batch/jcl/BNKSTMT.jcl --profile extras/jclsh/bank.conf
```

`run` はインストリーム DD と登録済みデータセットをジョブ専用ファイルに展開し、登録済みプログラムを Docker 内で起動します。`PARM` は `JCL_PARM`、DD のパスは `DD_名前` で渡します。アダプターが `JCL_ABEND_FILE` のパスに `U1234` または `S0C4` 形式のコードを書いた場合、ABEND として扱います。Docker のシグナル終了コードもローカルの `Sxxx` として記録します。ジョブとステップの結果、SYSOUT DD と `result.json` は `build/jclsh/spool/<job-id>/` に保存します。ジョブ RC が非ゼロなら `run` コマンドの終了コードは 1 です。`run -` / `lint -` / `plan -` で標準入力からも投入できます。

引数なしの `jclsh` は対話シェルです。`submit <file>`、`plan <file>`、`jobs`、`status <job-id>`、`spool <job-id> [DD 名 | STEP.DD]`、`datasets [接頭辞]`、`set 名前 値`、`help`、`exit` を使えます。`submit` は同期実行します。

自動通し試験は次のコマンドで実行します。JCL 原本の SHA-256 が変わらないこと、DD の値、2 口座の帳票、RC、スプールを確認します。

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File extras/jclsh/test-bank-jcl.ps1
powershell -NoProfile -ExecutionPolicy Bypass -File extras/jclsh/test-multistep-jcl.ps1
powershell -NoProfile -ExecutionPolicy Bypass -File extras/jclsh/test-datasets-jcl.ps1
powershell -NoProfile -ExecutionPolicy Bypass -File extras/jclsh/test-genapp-lint.ps1
powershell -NoProfile -ExecutionPolicy Bypass -File extras/jclsh/test-utilities.ps1
```

WSL Ubuntu ではリポジトリのルートから `bash extras/jclsh/test-step-wsl.sh` で試験用アダプターの GCC ビルドと ABEND コードの受け渡しを確認できます。

詳細な期待値は [Bank of Z のテストケース](../../docs/testcases/jcl-bnkstmt.md)、[複数ステップのテストケース](../../docs/testcases/jcl-multistep.md)、[データセット・ABEND のテストケース](../../docs/testcases/jcl-datasets-abend.md)に記載しています。プロファイルのパスはリポジトリのルートからの相対パスです。資格情報は Git 管理対象外の `.runtime.env` から Docker に渡します。
CardDemo の IMS 原本 `UNLDPADB.JCL` と `LOADPADB.JCL` は
`extras/imsdb/test-carddemo-jcl-cycle.ps1` で連続実行できます。JCL の
[IBM の JCL 文フィールド仕様](https://www.ibm.com/docs/en/zos/2.5.0?topic=statements-jcl-statement-fields)に従い、73〜80桁の通し番号は解析対象から除外します。
