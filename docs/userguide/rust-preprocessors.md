# COBOL前処理器（Rust）

`EXEC CICS`、`EXEC SQL`、`EXEC DLI` の固定形式COBOL前処理器は `extras/preprocessors` のRust実装です。`cobc` は各コマンドを直接実行します。通常は3つの実行ファイルを `PATH` に置きます。

```sh
cargo build --locked --release --manifest-path extras/preprocessors/Cargo.toml --bins
export PATH="$PWD/extras/preprocessors/target/release:$PATH"
```

Windowsでは実行ファイルに `.exe` が付きます。DockerでLinux版を作る場合は、次のようにビルドしたバイナリをLinuxコンテナ内で使います。

```powershell
docker run --rm -v "${PWD}:/src" -w /src `
  -e CARGO_TARGET_DIR=/src/build/preprocessors-linux `
  --entrypoint cargo rust:1-bookworm build --locked --release `
  --manifest-path extras/preprocessors/Cargo.toml --bins
```

実行ファイルが `PATH` にない場合は絶対パスを設定します。コンパイラは `COB_CICS_PREPROCESSOR`、`COB_DB2_SQL_PREPROCESSOR`、`COB_IMS_DLI_PREPROCESSOR` を参照します。例えばLinuxでは次のとおりです。

```sh
export COB_CICS_PREPROCESSOR="$PWD/extras/preprocessors/target/release/cics-preprocess"
export COB_DB2_SQL_PREPROCESSOR="$PWD/extras/preprocessors/target/release/db2sql-preprocess"
export COB_IMS_DLI_PREPROCESSOR="$PWD/extras/preprocessors/target/release/imsdli-preprocess"
```

`--cics-map=MAP.cpy`、`--db2-sql` / `--postgres-sql`、`--ims-dli=DBD` のコンパイラオプションは従来どおりです。単体で使う場合の引数は以下のとおりです。

```text
cics-preprocess SOURCE MAP.cpy OUTPUT
db2sql-preprocess [-I COPY_DIR] SOURCE OUTPUT
imsdli-preprocess SOURCE OUTPUT --dbd DBD
```

`--save-temps` で変換後のCOBOLを確認できます。CIはRustの単体テストと旧実装との出力照合を実行します。旧Pythonモジュールは `tests/legacy_preprocessors` に回帰テストの比較用としてのみ残しています。

リポジトリ内の開発用スクリプトは、Windowsでは `extras/preprocessors/target/debug/*.exe`、WSL/Dockerでは `build/preprocessors-linux/debug/*` を参照します。実行前に `cargo build --locked --manifest-path extras/preprocessors/Cargo.toml --bins`、またはDockerの上記コマンドから `--release` を外したビルドを実施してください。
