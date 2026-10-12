# IMS TM ローカルメッセージキュー

`imsdb` の Rust キューは取引コードと LTERM をそれぞれ8バイトの宛先として扱います。プロセス間で共有する `COB_IMS_TM_DIR` を指定すると、`CBLTDLI` の I/O PCB `GU`／`GN` と `ISRT`、代替 PCB `ISRT`／`PURG`／`CHNG` がこのキューを使います。従来の `DD_IMSINPUT`／`DD_IMSOUTPUT` 方式は `COB_IMS_TM_DIR` を設定しない場合に使用します。

メッセージは、各セグメントを **2バイトのビッグエンディアン LL、2バイトの ZZ、本文**で連結したバイト列です。LL はそのセグメント自身の長さを含みます。本文の EBCDIC、DBCS シフト文字、パック十進数、バイナリ項目は変換しません。1セグメントは最大32,767バイト、1メッセージは最大1MiBです。

## 投入と受信

WSL または Linux で Rust バイナリをビルドした後、たとえば次のように投入します。

```sh
export COB_IMS_TM_DIR=/tmp/ims-tm
build/imsdb-linux/debug/imsdbctl tm-enqueue "$COB_IMS_TM_DIR" IBSCUDAT TERM0001 0008000041424344
build/imsdb-linux/debug/imsdbctl tm-list "$COB_IMS_TM_DIR" IBSCUDAT
```

`tm-enqueue` は投入したIDを表示します。`tm-list` は `ID 送信元 本文の16進数` を表示する読出し専用の確認コマンドです。COBOL側では `COB_IMS_TM_DEST=IBSCUDAT`、`COB_IMS_PSB`、`COB_IMS_ENTRY` を設定し、[imsbatch](../../extras/imsdb/imsbatch.c)で起動します。DBを使用する場合は `COB_IMS_WAL` と `COB_IMS_DBD_DIR` も設定します。`imsbatch` は元のPSBに記載された TP、DB、GSAM PCBを記載順に渡し、先頭に暗黙のI/O PCBを置きます。DB PCBだけのバッチプログラムには `COB_IMS_LINKAGE=DB` を指定します。PSBを使わない従来の単一DB PCB起動では `COB_IMS_DBD_NAME` を使用します。

I/O PCB の `GU` は取引宛先からメッセージを取得し、PCBの先頭8バイトへ送信元 LTERM を設定します。最初のセグメントを I/O 領域へ返し、後続セグメントは `GN` で取得します。対象が空なら `GU` は `QC`、後続セグメントが尽きたら `GN` は `QD` を返します。I/O PCB の `ISRT` は取得元 LTERM 宛ての応答を保留します。入力を取得しない送信専用プログラムでは `COB_IMS_TM_LTERM` に応答先を指定します。

代替 PCB は元の PSB の `PCB TYPE=TP,LTERM=...` から初期宛先を得ます。`MODIFY=YES` の PCB には `CHNG` で8バイトの宛先を設定できます。`ISRT` を続けて呼ぶと同じPCBのセグメントを1メッセージへ追加し、`PURG` でメッセージを区切ります。宛先変更前には `PURG` が必要です。入出力PCBの応答は1セグメントに制限します。IBMの [PCB順序](https://www.ibm.com/docs/en/ims/15.6.0?topic=db-program-communication-block-pcb-lists)、[メッセージ駆動プログラム](https://www.ibm.com/docs/en/ims/15.6.0?topic=tm-writing-message-driven-programs)、[代替PCBと呼出し](https://www.ibm.com/docs/en/ims/15.6.0?topic=tm-relationship-calls-pcb-types)のうち、このリポジトリの参照アプリで使う範囲を実装しています。

COBOLプログラムが次の `GU`、正常終了、`IMSDBCOM`、または基本 `CHKP` を実行すると、先行メッセージの入力を確認済みにし、保留中の出力をキューへ保存します。`ROLB` と異常終了では現在の作業単位の入力を再配送可能にし、出力を破棄します。受信の貸出しにはOSファイルロックを使い、プロセスが異常終了しても次の受信者が直ちに再取得できます。キューファイルはチェックサムを付け、一時ファイルの同期と置換で更新します。

IMS DB WALとTMキューは**別々の永続資源**です。両者の確定間にプロセス障害が起きると、DB更新後に入力が再配送される可能性があります。処理結果の完全な一括確定が必要な業務では、この構成だけに依存せず、業務キーで再実行を識別してください。MFS、OTMA、会話型端末制御、IMSのシンボリックチェックポイント／`XRST`、実IMSメッセージキューとの接続はこのローカル互換実装の対象外です。

## 検証

`extras/imsdb/test-bank-original.sh` は無修正の Bank of Z `IBSCUDAT` をキューから起動し、DB更新、LTERM宛て応答、正常／異常終了の確定境界を検証します。`extras/imsdb/test-tm-alt.sh` は代替PCBの `CHNG`／`ISRT`、IBM-037、PSB順の複数PCB、`PURG` と複数セグメントの `GU`／`GN` を検証します。詳細は[テストケース](../testcases/ims-tm.md)を参照してください。
