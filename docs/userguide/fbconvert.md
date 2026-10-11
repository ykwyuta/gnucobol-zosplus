# 固定長ファイルとローカル FB／VSAM の相互変換

`fbconvert` は、外部ツールが扱う **改行・レコード長ヘッダーなしの固定長バイト列**と、`jclsh`／CICS が使うローカル FB／VSAM ファイルを相互変換します。データ本体は同じバイト配置です。変換ではレコードを検査してコピーし、インポート時に `<ファイル名>.jclsh-meta` を作成します。EBCDIC、日本語、パック十進数、バイナリ項目の内容を変換しません。

## ビルド

```sh
cargo build --manifest-path extras/jclsh/Cargo.toml --bin fbconvert
```

実行ファイルは `extras/jclsh/target/debug/fbconvert`（Windows は `fbconvert.exe`）です。以下では、そのディレクトリを PATH に追加した場合のコマンドを示します。
CI の `zosplus-tools-Linux.zip`／`zosplus-tools-Windows.zip` にも `bin/fbconvert`／`bin/fbconvert.exe` を同梱します。

## インポート

```sh
fbconvert import external.fb local.dat --lrecl 225 --key-length 10 --key-offset 0
```

`external.fb` の全長が 225 の倍数であることを確認し、各 225 バイトを1レコードとして読みます。キー付きファイルではキーがレコード内に収まることと、キーの重複がないことも検査します。順ファイルでは `--key-length` と `--key-offset` を省略します。

CICS のローカル VSAM 実装に合わせ、キー付きファイルは 64 MiB までとします。

成功時に `local.dat` と `local.dat.jclsh-meta` を作成します。既存の出力は上書きしません。途中で検査や書き込みが失敗した場合、作成途中の一時ファイルを削除します。

## エクスポート

```sh
fbconvert export local.dat external-copy.fb --lrecl 225
```

メタデータの LRECL とデータ本体のレコード長を検査してから、データ本体だけを出力します。`--lrecl` は省略できますが、指定すると期待値との一致を追加で確認します。外部ファイルの各レコードは LRECL バイトで、改行や BOM は付きません。

## 外部ツールで扱う際の条件

- バイナリモードで読み書きし、LRECL バイトずつ区切ってください。CR/LF、文字コード変換、空白の自動補完・削除を行わないでください。レコード内に `0x0a` や `0x0d` があっても、それはデータです。
- このツールは項目の値を解釈しません。EBCDIC や `COMP-3` を表示・編集するには、外部ツール側で対応するレコード定義とエンコーディングを使用してください。
- ローカル VSAM を CICS が使用している間はインポート・エクスポート・外部更新を行わないでください。外部ツールの書き込みは CICS のロックや COMMIT／ROLLBACK に参加せず、同時読み出しも一貫したスナップショットを保証できません。交換は停止中の新しいファイルで行い、配置を切り替えてください。
- JCL ソースや `DD *` の入力カードは行単位のテキストです。このツールの対象は、そのカードを FB データセットへ格納した後の固定長ファイルです。

ローカル VSAM の形式は IBM の物理 VSAM データセット形式とは異なります。z/OS の実データセットと交換する際は、先に REPRO などで固定長レコードのストリームへ取り出してください。
