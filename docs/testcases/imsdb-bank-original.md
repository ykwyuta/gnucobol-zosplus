# Bank of Z 原本 IMS COBOL と I/O PCB の結合試験

## 対象

無修正の `reference/Bank-of-Z-main/src/base/ims/cobol/IBSCUDAT.cbl` を `DLITCBL` エントリーから起動し、I/O PCB の `GU`／`ISRT` と `CUSTOMER` DB PCB の `GHU`／`REPL` を組み合わせる。DBD／PSB は Bank of Z の原本を使う。

## 実行

Linux 版の `libimsdb` と、このリポジトリの `libcob`、`cobc` をビルドした後、PowerShell から実行する。

```powershell
wsl --exec sh /mnt/d/workspace/gnucobol/extras/imsdb/test-bank-original.sh
```

スクリプトは一時ディレクトリに Bank of Z の9 DBD／1,169件を投入し、1件の長さ付きメッセージを作る。顧客番号1の姓・名・住所を更新し、応答ファイルの長さと `TESTFIRST` の本文を照合する。顧客の検索成功と、入力の終端で `QC` が返ったことも原本プログラムのログで確認する。さらに別プロセスの COBOL から同じ顧客を `GU` で再取得し、更新内容の永続化を確かめる。

同じスクリプトは小さな I/O PCB 専用 COBOL プログラムも実行する。RC 8 の終了では出力ファイルが作られず、RC 0 では `0008 0000 TEST` の8バイトメッセージが確定する。期待する表示は `IMS BANK REOPEN OK`、`IMS BANK ORIGINAL OK`、`IMS IO PCB COMMIT/ROLLBACK OK`。

`IMSDEXEC CHKP` に `CHECK001` を渡した後、別プロセスの `imsdbctl latest-checkpoint` で同じIDを読み戻す。`test-cbltdli.sh` でも EBCDIC モードの `CBLTDLI CHKP` を同じ方法で検証する。Rust試験は、チェックポイントとDB更新を同じWALフレームで復元でき、`ROLB` されたIDが記録されないことを確認する。

このメッセージ経路はローカルファイルによるバッチ入力であり、IMS TM のオンライン端末キューや代替 PCB の代用にはならない。DB WAL と出力ファイルの確定は原子的な一操作ではない。

## 原本 `IBLOGIN1`／`IBLOGOUT` の IMS TM 操作

```powershell
wsl --exec sh /mnt/d/workspace/gnucobol/extras/imsdb/test-bank-login-original.sh
```

`IBLOGIN1` 原本には `PROGRAM-ID. IBLOGIN1` の句点がないため、ビルド用コピーだけ補正する。`LoadData` の9 DBD・1,169件を投入し、IBM-037 の端末メッセージを IMS TM キューへ送る。誤パスワード、正しいパスワード、ログイン済みの再要求に対する原本の3応答を確認し、別プロセスの `imsdbctl get-root` で CUSTOMER の状態が `1` に確定したことを確認する。続いて無修正 `IBLOGOUT.cbl` にログアウト要求を送り、成功応答と CUSTOMER の状態 `0` を照合する。2026-10-12 に両原本の実行と永続更新が通過した。`IBLOGIN.pli` と `PLITDLI` の経路は別途未対応である。

## 原本 `IBGCUDAT`／`IBACSUM` の照会

```powershell
wsl --exec sh /mnt/d/workspace/gnucobol/extras/imsdb/test-bank-inquiry-original.sh
```

無修正の2本を IBM-037 でコンパイルし、原本9 DBD・1,169件と IMS TM のメッセージを使う。`IBGCUDAT` は顧客1の名前と存在しない顧客のエラー応答を、`IBACSUM` は顧客1に紐づく口座2件を照合する。`CUSTACCS.data` の列順は `ACCID,CUSTID,ACCNUM` で DBD 上の `CUSTID,ACCID,ACCNUM` と異なるため、取込時に列を対応付ける。`PROCOPT=G` の PCB に対する Get Hold は更新ロックを保持しない。2026-10-12 に両原本の応答を確認した。
