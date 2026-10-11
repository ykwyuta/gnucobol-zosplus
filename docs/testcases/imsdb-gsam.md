# IMS GSAM 固定長レコード結合試験

## 対象

CardDemo 原本の `DLIGSAMP.PSB` と `PASFLDBD.DBD` を読み、GnuCOBOL の `CBLTDLI` から固定長 GSAM ファイルを扱う。レコード長は DBD の 100 バイトを使用する。

## 実行

リポジトリのルートで Linux 用 Rust ライブラリをビルドする。

```powershell
docker run --rm --network none -v D:\workspace\gnucobol:/src -w /src -e CARGO_TARGET_DIR=/src/build/imsdb-linux rust:1-bookworm sh -lc 'PATH=/usr/local/cargo/bin:$PATH cargo build --manifest-path extras/imsdb/Cargo.toml --offline --lib --bin imsdbctl'
wsl --exec sh /mnt/d/workspace/gnucobol/extras/imsdb/test-gsam.sh
```

チェックアウト先が異なる場合は両方のパスを変更する。試験スクリプトは一時領域に100バイトの入力レコード2件を作り、`DD_PASFILIP` と `DD_PASFILOP` に割り当てる。

## 期待する結果

`IMS GSAM OK` と表示され、終了コードが0になる。COBOL の処理は `OPEN`、`GN` 2回、終端 `GB`、取得した8バイト RSA を使った `GU`、`ISRT`、確定、`CLSE` の順。出力ファイルは100バイトの1件で、先頭5バイトが `THIRD` になる。Rust 単体試験では未確定挿入のロールバックも確認する。

RSA はローカル実装の8バイトレコード番号であり、IMS 実機が返す物理 RSA との交換は検証対象外。IMS DB WAL と GSAM 出力の原子的な一括確定も未実装。
