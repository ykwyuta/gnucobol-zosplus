# jclsh

Rust 製 JCL インタプリタ兼対話シェルです。Bank of Z の原本 `BNKSTMT.jcl` を無編集で解析・計画・実行できます。複数の `EXEC PGM=` ステップ、インストリーム `PROC` / `PEND` と `EXEC procname`、`&symbol`、`SET`、プロシージャーの `step.ddname` 上書き、`COND`、`IF` / `ELSE` / `ENDIF`、`PARM`、`JOBLIB`、連結 DD、`DD *` / `DD DATA,DLM=`、`SYSOUT=*` を扱います。`cics-genapp` の `cobol.jcl` は 93 ステップまで無編集で展開できます。

各ステップの直接 `PGM=` または `IKJEFT01` の `DSN RUN PROGRAM(...)` はプロファイルの `program.NAME=...` に登録します。未登録プログラムと未マッピングのデータセット DD は計画段階でエラーになります。`COND` は比較が真ならステップをスキップします。通常の非ゼロ RC 後も後続ステップを判定し、ジョブ RC は実行したステップの最大 RC とします。`result.json` は実行済み/スキップと各 RC を記録します。

データセットの割り当て・`DISP` の状態遷移、カタログド PROC / `INCLUDE`、未登録の z/OS ユーティリティー、ABEND はまだ実行できません。`JOBLIB` / `STEPLIB` は計画に表示するメタデータです。`JCLLIB` は読み取りますが、ライブラリー探索は未実装です。`IF` は RC 比較、AND / OR / NOT と括弧を扱います。`SYSTEM`、`PLAN`、`LIB` はプロファイルとの照合で、Db2 for z/OS のバインド操作ではありません。

## 準備

リポジトリのルートで実行します。Docker Compose の Db2、`gnucobol-db2-build` イメージ、`extras/db2sql/.runtime.env` を用意してください。[PL/I の手順](../pl1c/README.md)に従い、`BNKSTMT.pli` の生成 C と `build/pl1c/BNKSTMT` をビルドし、`PL1TEST` の検証データを投入します。

```powershell
cargo run --manifest-path extras/jclsh/Cargo.toml --offline -- lint reference/Bank-of-Z-main/src/base/batch/jcl/BNKSTMT.jcl
cargo run --manifest-path extras/jclsh/Cargo.toml --offline -- plan reference/Bank-of-Z-main/src/base/batch/jcl/BNKSTMT.jcl --profile extras/jclsh/bank.conf
cargo run --manifest-path extras/jclsh/Cargo.toml --offline -- run reference/Bank-of-Z-main/src/base/batch/jcl/BNKSTMT.jcl --profile extras/jclsh/bank.conf
```

`run` はインストリーム DD をジョブ専用ファイルに展開し、登録済みプログラムを Docker 内で起動します。`PARM` は `JCL_PARM` 環境変数で渡します。ジョブとステップの RC、SYSOUT DD と `result.json` は `build/jclsh/spool/<job-id>/` に保存します。ジョブ RC が非ゼロなら `run` コマンドの終了コードは 1 です。`run -` / `lint -` / `plan -` で標準入力からも投入できます。

引数なしの `jclsh` は対話シェルです。`submit <file>`、`plan <file>`、`jobs`、`status <job-id>`、`spool <job-id> [DD 名 | STEP.DD]`、`help`、`exit` を使えます。`submit` は同期実行します。

自動通し試験は次のコマンドで実行します。JCL 原本の SHA-256 が変わらないこと、DD の値、2 口座の帳票、RC、スプールを確認します。

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File extras/jclsh/test-bank-jcl.ps1
powershell -NoProfile -ExecutionPolicy Bypass -File extras/jclsh/test-multistep-jcl.ps1
```

詳細な期待値は [Bank of Z のテストケース](../../docs/testcases/jcl-bnkstmt.md)と[複数ステップのテストケース](../../docs/testcases/jcl-multistep.md)に記載しています。プロファイルのパスはリポジトリのルートからの相対パスです。資格情報は Git 管理対象外の `.runtime.env` から Docker に渡します。
