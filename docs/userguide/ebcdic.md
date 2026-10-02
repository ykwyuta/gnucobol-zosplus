# EBCDIC 互換モード

`-fdata-codeset` は、COBOL の `PIC X` などのデータ項目をメモリ上で保持する文字コードを選ぶコンパイル時オプションである。指定しなければ `NATIVE` になる。ソースの文字コードと、実行時の端末・外部テキストの文字コードは別に指定する。

| 指定 | 用途 |
| --- | --- |
| `NATIVE` | 従来のホスト文字コード。既定値。 |
| `HOST` | ホスト文字コードのデータを使い、NATIONAL の対応を有効にする。 |
| `IBM-037` | 英数字を中心とする EBCDIC。日本語のソース定数は表現できない。 |
| `IBM-930`、`IBM-939`、`IBM-1390`、`IBM-1399` | 日本語 EBCDIC。DBCS 文字も扱う。 |

## コンパイルと実行

次の固定形式 COBOL を `ebcdic-quick.cbl` として保存する。`HEX-OF` はデータ項目の実際のバイト値を確認するために使用する。

```cobol
       IDENTIFICATION DIVISION.
       PROGRAM-ID. EBCDIC-QUICK.
       DATA DIVISION.
       WORKING-STORAGE SECTION.
       01 WK-CODE PIC X(4) VALUE 'AB12'.
       PROCEDURE DIVISION.
           DISPLAY FUNCTION HEX-OF(WK-CODE)
           DISPLAY WK-CODE
           STOP RUN.
```

このブランチからビルドした `cobc` を使ってコンパイルする。以下は PATH にインストール済みの場合の例である。

```powershell
cobc -x -fdata-codeset=IBM-1399 -fsource-codeset=UTF-8 -o ebcdic-quick ebcdic-quick.cbl
$env:COB_HOST_CODESET = 'UTF-8'
./ebcdic-quick
```

最初の表示は `C1C2F1F2`、次は `AB12` になる。`C1C2F1F2` はデータ項目中の EBCDIC バイトであり、`DISPLAY` は端末向けの文字コードへ変換する。Windows の実行ファイル名が `ebcdic-quick.exe` の場合は、その名前で起動する。

日本語の定数を含む UTF-8 ソースでは、たとえば次のように指定する。

```powershell
cobc -x -fdata-codeset=IBM-1399 -fsource-codeset=UTF-8 -o japanese japanese.cbl
$env:COB_HOST_CODESET = 'UTF-8'
./japanese
```

`-fsource-codeset` の既定値は UTF-8。`-std=ibm-jp` は `IBM-1399` を既定のデータコードセットにする。明示した `-fdata-codeset` は方言設定より優先される。日本語 DBCS では `PIC G` と、既定で `PIC N` が DBCS データとなる。`PIC N` を UTF-16 の NATIONAL として使う場合は `-fnsymbol=national` を指定する。

端末と外部テキストの変換には、実行環境に合わせて `COB_HOST_CODESET` を設定する。順ファイルのレコードは EBCDIC バイトのまま保存し、LINE SEQUENTIAL のテキストはホスト側の文字コードで読み書きする。既存の ASCII データファイルをそのまま EBCDIC モードで読ませることはできない。

## Db2 と併用する

[Db2 ガイド](db2.md)の `build-demo.sh` は `ebcdic-demo.cbl` を `IBM-1399` と UTF-8 ソースでコンパイルする。`COB_HOST_CODESET=UTF-8` で実行すると、`日本語` を Db2 のホスト変数に渡して読み戻し、`DB2 EBCDIC OK` を表示する。Db2 接続の設定と実行コマンドは同ガイドに記載する。

文字化けした場合は、コンパイル時のデータコードセット、ソースファイルの実際の文字コード、実行時の `COB_HOST_CODESET` をそれぞれ確認する。指定した EBCDIC に表現できないソース定数はコンパイル時にエラーとなる。
