# wc3270 と cobcicsd の接続機能

`cobcicsd` は BMS 画面を TN3270/TN3270E で配信する Rust 製ホストです。このページは端末サイズ、TLS、NVT、IND$FILE、3287 印刷の起動方法を示します。COBOL ワーカーのビルドと Bank of Z の業務画面については [CICS/BMS ガイド](cics-bms.md)を参照してください。

## ビルドと端末表示

リポジトリのルートから PowerShell で実行します。

```powershell
cargo build --manifest-path extras/cobcics/Cargo.toml --locked --bin cobcicsd
$bms = 'reference/Bank-of-Z-main/src/base/cics/bms/BNK1MAI.bms'
& extras/cobcics/target/debug/cobcicsd.exe --bms $bms --tn3270e `
  --listen 127.0.0.1:3270
```

別の端末で接続します。

```powershell
& 'C:\Program Files\wc3270\wc3270.exe' +S -model 3279-2 127.0.0.1:3270
```

`--tn3270e` は [TN3270E](https://www.rfc-editor.org/rfc/rfc2355.html) の端末タイプと基本機能を交渉します。`IBM-3278/3279` の Model 2～5、`IBM-DYNAMIC`、12/14 ビットアドレス、拡張色・強調表示に対応します。端末の Query Reply から usable area を取得するため、`-oversize 100x50` のような拡張画面にも BMS マップのサイズが一致すれば表示できます。従来の TN3270 接続は `N:127.0.0.1:3270` で確認できます。

## TLS トンネルと STARTTLS

PEM 形式の証明書と秘密鍵を用意し、次のいずれかでホストを起動します。証明書には接続先の DNS 名または IP アドレスを SAN として入れてください。

```powershell
# 接続直後から TLS を使用するポート
& extras/cobcics/target/debug/cobcicsd.exe --bms $bms --tn3270e `
  --tls-cert cert.pem --tls-key key.pem --listen 127.0.0.1:3271
& 'C:\Program Files\wc3270\wc3270.exe' +S L:127.0.0.1:3271

# Telnet STARTTLS の交渉直後に TLS へ切り替えるポート
& extras/cobcics/target/debug/cobcicsd.exe --bms $bms --tn3270e `
  --tls-cert cert.pem --tls-key key.pem --starttls --listen 127.0.0.1:3272
& 'C:\Program Files\wc3270\wc3270.exe' +S 127.0.0.1:3272
```

`--starttls` は端末が [Telnet START_TLS](https://datatracker.ietf.org/doc/html/draft-ietf-tn3270e-telnet-tls-04) を拒否したら接続を終了します。自己署名証明書でローカル試験をする場合、wc3270 の `Y:` プレフィックスを使うと証明書検証を無効にできます。本番接続では信頼できる証明書を使ってください。TLS は表示、ファイル転送、プリンターの接続にも適用できます。

## NVT から 3270 への切り替え

```powershell
& extras/cobcics/target/debug/cobcicsd.exe --bms $bms --nvt `
  --tn3270e --listen 127.0.0.1:3273
& 'C:\Program Files\wc3270\wc3270.exe' +S -nvt 127.0.0.1:3273
```

NVT の行入力では `HELP`、`3270`、`QUIT` が使えます。`3270` を入力すると Telnet の BINARY/EOR と TN3270E の交渉を始め、BMS 画面へ切り替わります。これは [wc3270 の NVT モード](https://x3270.bgp.nu/Windows/wc3270-man.html)に対する接続入口です。3270 プリンターは同じ NVT 入口を通さず、3270 用ポートへ接続してください。

## IND$FILE 転送

ホスト側のファイル置き場を `--indfile-root` で指定します。画面には `IND$FILE ...` コマンドを入力できる非保護フィールドが必要です。Bank of Z の 1 桁の Action 項目では長さが足りないため、転送用の BMS マップを別に用意します。

次の内容を `transfer.bms` として保存します。

```text
FT      DFHMSD TYPE=MAP
SCREEN  DFHMDI SIZE=(24,80)
TITLE   DFHMDF POS=(1,1),LENGTH=18,ATTRB=PROT,*
          INITIAL='IND$FILE COMMAND'
COMMAND DFHMDF POS=(3,1),LENGTH=74,ATTRB=(UNPROT,IC)
```

```powershell
New-Item -ItemType Directory -Force build/indfile-data | Out-Null
& extras/cobcics/target/debug/cobcicsd.exe --bms transfer.bms `
  --tn3270e --indfile-root build/indfile-data --listen 127.0.0.1:3274
& 'C:\Program Files\wc3270\wc3270.exe' +S 127.0.0.1:3274
```

wc3270 の Transfer アクションで `Host=cics`、ホスト名 `DOWNLOAD` などを指定します。受信はホストの `build/indfile-data/DOWNLOAD` をローカルへ取得し、送信は同じディレクトリへ保存します。ホスト名は英数字、`.`、`_`、`-`、括弧からなる単一の名前に制限し、大文字に正規化します。送信中のデータは一時ファイルに書き、正常終了時に確定します。

転送は [IND$FILE の DFT structured field 方式](https://x3270.bgp.nu/SS-HCS12-1372-00.pdf)です。GET/PUT のバイナリ転送と単純な ASCII テキスト転送を確認しています。日本語の UTF-8 または IBM-939 ファイルは `Mode=binary` で転送してください。日本語の `Mode=ascii` は今回の s3270 試験で文字化けしたため、対応済みとは扱いません。CUT 方式、z/OS データセットのレコード属性やカタログ管理は実装していません。テキストの改行変換は wc3270 の `Cr` オプションに従います。

## 3287 プリンター

`--printer-queue` は COBOL の `EXEC CICS WRITEQ TD` が作る TDQ を 3287 の印刷データ源にします。ホストとワーカーで同じ `COBCICS_STORAGE_DIR` を使用してください。キュー名は最大 8 文字です。

```powershell
$env:COBCICS_STORAGE_DIR = (Resolve-Path build).Path
& extras/cobcics/target/debug/cobcicsd.exe --bms $bms --tn3270e `
  --printer-queue PRNT --printer-input utf8 --listen 127.0.0.1:3275
& 'C:\Program Files\wc3270\pr3287.exe' -assoc COBCICS1 127.0.0.1:3275
```

COBOL 側の例です。`genapp_queue.c` をワーカーにリンクします。

```cobol
       01  PRINT-Q     PIC X(8) VALUE 'PRNT'.
       01  PRINT-LINE  PIC X(80).
           EXEC CICS WRITEQ TD
                QUEUE(PRINT-Q) FROM(PRINT-LINE)
                LENGTH(80)
           END-EXEC
```

プリンター接続は `IBM-3287-1` を認識し、[TN3270E の SCS-CTL-CODES または DATA-STREAM-CTL](https://www.rfc-editor.org/rfc/rfc2355.html) を交渉します。`--printer-input utf8` は標準の GnuCOBOL がキューに書く UTF-8 テキストを `--charset` のホストコードページに変換します。EBCDIC 互換モードで既にホストコードにした記録を送る場合は `--printer-input host`（既定）を使います。ジョブの区切りで `PRINT-EOJ` を送ります。端末 LU とプリンター LU の厳密なリソース管理、および CICS/JES の本物のスプールはありません。
送信済み位置は `COBCICS_STORAGE_DIR` の `.cursor` ファイルに保存されるため、プリンターを再接続しても同じ記録を初めから再送しません。同じキューに対する同一サーバープロセス内のプリンター接続は 1 件に制限します。

## 回帰試験

Windows に wc3270 一式と Git for Windows の OpenSSL がある環境で実行します。生成物は `build/` 以下に置かれます。

```powershell
cargo test --manifest-path extras/cobcics/Cargo.toml --locked
cargo build --manifest-path extras/cobcics/Cargo.toml --locked --bin cobcicsd
python extras/cobcics/test-wc3270-models.py
python extras/cobcics/test-wc3270-transport.py
python extras/cobcics/test-indfile-client.py
python extras/cobcics/test-japanese-wc3270-host.py
```

モデル 2～5 と 100×50 oversize、TLS トンネル/STARTTLS、NVT、実際の `pr3287` 交渉、TDQ からの印刷レコード、IND$FILE の両方向を確認します。日本語試験は IBM-939 の wc3270 画面入出力を通常接続、TLS、STARTTLS、NVT 切替で確認し、転送試験は UTF-8 と IBM-939 の日本語バイト列を `Mode=binary` で照合します。印刷試験は UTF-8 入力とホスト EBCDIC 入力の両方から IBM-939 の SCS データを確認します。
