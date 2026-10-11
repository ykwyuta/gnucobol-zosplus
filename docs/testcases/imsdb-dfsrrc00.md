# IMS DB: CBLTDLI と DFSRRC00 結合試験

## 対象

- Bank of Z の9 DBD、`LoadData` の1,169件、更新権限のある `IBSCUDAT` PSB
- GnuCOBOL の `CBLTDLI` と Rust 製 IMS DB の共有ライブラリ
- `jclsh` の `PGM=DFSRRC00,PARM='BMP,IMSSMOKE,IBSCUDAT'`

## 実行

リポジトリのルートから PowerShell で実行する。

```powershell
wsl --exec sh /mnt/d/workspace/gnucobol/extras/imsdb/test-cbltdli.sh
./extras/imsdb/test-dfsrrc00.ps1
```

WSL 試験の前に Linux 版の `libimsdb.so` と `imsdbctl` をビルドする手順は [利用ガイド](../userguide/imsdb.md)を参照する。2番目のスクリプトは Docker でビルドから実行まで行う。実行ごとに専用の試験 WAL を初期化する。

| 操作 | 期待結果 |
| --- | --- |
| `GU` で顧客 ID 1 を取得 | `Antonelli`、PCB 状態は空白 |
| `GHU → REPL → IMSDBCOM` | `Updated` が別プロセスから取得できる |
| 更新後、`IMSDBCOM` なしで正常終了 | 別プロセスでは `Updated` のまま |
| `GHU → REPL → CHKP` | `Checkpoint` が別プロセスから取得できる |
| `GHU → REPL → ROLB` | 別プロセスでは `Checkpoint` のまま |
| JCL の `DFSRRC00` 起動 | `IMSSMOKE` が実行され、ステップ RC は0 |
| JCL から明示的 `IMSDBCOM` を使わず RC 0 で終了 | `AutoCommit` が別プロセスから取得できる |
| 自動確定モードで更新後に RC 8 で終了 | 別プロセスでは元の値のまま |

`CHKP` と `ROLB` はバッチ用 I/O PCB の基本呼び出しを対象とする。チェックポイント ID の再始動処理、シンボリック・チェックポイント、メッセージ PCB、GSAM、原本 IMS エントリーへの PCB ポインター引き渡しはこの試験では成功条件に含めない。
