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
