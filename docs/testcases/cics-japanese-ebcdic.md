# CICS 日本語 EBCDIC 入出力の試験

## 対象

Rust 製 TN3270 ホスト `cobcicsd` の IBM-930、IBM-939、IBM-1390、IBM-1399 対応、IBM-1399 のスクリプト化した TN3270 クライアント、IBM-939 の実際の `wc3270` と COBOL ワーカーの画面入出力を確認する。BMS 原本は UTF-8、端末データは SO/SI 付き EBCDIC、Rust と COBOL の間のワーカープロトコルは UTF-8 を使う。

## 自動シナリオ

| 操作 | 期待結果 |
| --- | --- |
| 4種類のコードページで `A日本語B` を変換して復元する | 元の文字列に戻り、日本語部分を SO/SI で囲む |
| SO/SI が閉じていない入力を送る | 入力エラーとなる |
| UTF-8 の `INITIAL='日本語'` を含む BMS を読み込む | 初期値と BMS の `LENGTH` を正しく保持する |
| IBM-1399 の TN3270 画面を表示する | `日本語を入力` が IBM-1399 の端末データに含まれる |
| TN3270 の ACTION 項目から `日本語` を送る | COBOL が受け取り、返送した `日本語` が端末データに含まれる |
| `wc3270 -codepage cp939 -utf8` で画面を開く | 初期画面に `日本語を入力` が表示される |
| `wc3270` の ACTION 項目へ `日本語` を入力して Enter を押す | SO/SI 付き IBM-939 がホストに届き、COBOL が返した `日本語` が画面に表示される |
| 従来の CP037 と IBM-037 GenApp 画面を操作する | 既存の画面入力、Db2 照会、更新が成功する |

## 実行方法

リポジトリのルートで次を実行する。Docker、`gnucobol-db2-build`、`rust:1-alpine` が必要。

```powershell
cargo test --offline --manifest-path extras/cobcics/Cargo.toml
& extras/cobcics/test-japanese-tn3270.ps1
& extras/cobcics/test-japanese-wc3270.ps1
```

最初のコマンドはコードページ変換、BMS、TN3270 の単体試験を含む。2番目は IBM-1399 でスクリプト化した TN3270 クライアントを使う。3番目は Windows の `C:\Program Files\wc3270\wc3270.exe` を起動し、IBM-939 の画面表示と日本語入力をスクリプトポートで確認する。インストール先が異なるときは `-Wc3270Path` で指定する。いずれも Db2 は使わない。
