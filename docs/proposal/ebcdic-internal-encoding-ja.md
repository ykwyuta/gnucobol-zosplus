# 提案書: 日本語 IBM ホスト EBCDIC を内部コードとする GnuCOBOL 実行モデル

| 項目 | 内容 |
|------|------|
| 対象 | GnuCOBOL 3.x（`cobc` コンパイラ / `libcob` ランタイム） |
| 目的 | ASCII 系ホスト（Linux / Windows 等）上で、データ項目の**メモリ内表現を日本語 EBCDIC（CCSID 930 / 939 / 1390 / 1399）**にして動かす |
| 状態 | 検討・提案（未実装） |
| 作成日 | 2026-09-30 |

---

## 1. 背景と目的

IBM メインフレーム（z/OS、Enterprise COBOL）から GnuCOBOL へ移行する日本の案件では、次の要件がよく出ます。

* ホストから受け取るファイル（EBCDIC、ゾーン 10 進、パック 10 進、DBCS 漢字を含む）を**バイト単位でそのまま**読み書きしたい。
* `REDEFINES` や部分参照、`X"C1"` のような 16 進定数、`HIGH-VALUE` と `LOW-VALUE` の境界、キーの**照合順序（EBCDIC 順）**など、「バイトの値」に依存したロジックを**書き換えずに**動かしたい。
* `SORT` や索引ファイルのキー順、`IF A < B` の大小比較の結果をホストと一致させたい。

今の GnuCOBOL にも EBCDIC 関連の機能はいくつかあります（第 2 章）。ただしどれも「**内部は ASCII のまま、入口と出口で変換する**」か「**比較のときだけ照合順を切り替える**」という補助的な仕組みです。上のような「バイト値そのもの」に依存したロジックは、完全には再現できません。

この提案書では、**内部データ表現そのものを日本語 EBCDIC に切り替える**ために、GnuCOBOL で何をどう変えればよいかを整理し、実装方針・段階的な計画・リスクを示します。

---

## 2. 現状分析

### 2.1 すでにある EBCDIC 関連機能

| 機能 | 場所 | 概要 | 限界 |
|------|------|------|------|
| `COB_EBCDIC_MACHINE` | `libcob/common.h:586-592` | `' ' == 0x40` かどうかで**C コンパイラの文字セット**を判定する（z/OS 等でネイティブにビルドする場合） | ASCII ホスト上では常に無効。**ホストの文字コードとデータの文字コードが同じ**という前提 |
| `-fsign=EBCDIC` / `cb_ebcdic_sign` | `cobc/cobc.c:3967`, `libcob/move.c:2877`, `libcob/common.c:1759-` | ゾーン 10 進の符号オーバーパンチを EBCDIC 流（`{ABCDEFGHI` / `}JKLMNOPQR`）にする | 数字部分は ASCII（0x30-0x39）のまま |
| `ALPHABET ... IS EBCDIC`、`PROGRAM COLLATING SEQUENCE`、`-fdefault-colseq=EBCDIC` | `cobc/typeck.c:3860-3910`, `cobc/codegen.c:3590-3780` | 比較・SORT のときだけ変換表を通して EBCDIC 順にする | 比較のたびに表を引くため遅い。データ自体は ASCII |
| `CODE-SET` 句 | `cobc/parser.y:6945`, `cobc/codegen.c:9580-` | ファイルの読み書き時に ASCII と EBCDIC を変換する | レコード全体を変換するので、COMP-3 などバイナリ項目があると壊れる（`FOR` 句で項目を指定するのは部分対応、custom CODE-SET は PENDING） |
| `-febcdic-table` と `config/*.ttbl` | `cobc/cobc.c:4286`, `libcob/cconv.c:131` | 256 バイトの SBCS 変換表を読み込む（EBCDIC 500 用が同梱） | **SBCS 専用**。SO/SI を使う DBCS（CCSID 930/939）は表せない |
| `--gentable=<ebcdic>,<ascii>` | `cobc/gentable.c` | iconv を使って `.ttbl` を生成する | 1 バイト単位でしか変換しないので、ステートフルな混在コード（930/939）には使えない |
| `binary-byteorder: big-endian`、`hostsign` など | `config/ibm-strict.conf` | バイナリと符号を IBM 互換にする | 文字コードとは関係ない |

### 2.2 ASCII を前提にしているコード（変更が必要な箇所）

ランタイムもコンパイラも、C の文字定数（`' '`、`'0'`、`'+'` など）を**そのままデータに書き込む**作りになっています。ASCII ホストでは、これらはすべて ASCII のコード値になります。

| 分類 | 代表的な箇所 | 規模の目安 |
|------|--------------|-----------|
| 数字と整数の変換 | `libcob/coblocal.h:251-252` の `COB_D2I` / `COB_I2D`（`'0' + x`） | マクロ 1 か所だが、呼び出し元はランタイム全体 |
| 空白・ゼロの埋め | `memset (p, ' ', n)` / `memset (p, '0', n)` | libcob 内に約 53 か所 |
| 文字定数による比較・代入 | `' '`、`'0'` などの文字定数 | libcob 内に約 340 か所（common.c 82、intrinsic.c 77、screenio.c 49、move.c 44 など） |
| C ライブラリの文字クラス判定 | `isdigit` / `isalpha` / `isspace` / `toupper` など | libcob 内に約 50 か所 |
| 編集記号 | `libcob/move.c` の編集処理（`+ - * . , / B 0 CR DB $`） | move.c の中心部分 |
| 符号処理 | `libcob/common.c:1631-1900`、`libcob/numeric.c:4440-4600` | `#ifdef COB_EBCDIC_MACHINE` の分岐がすでにある |
| 行順ファイル | `libcob/fileio.c:449-460`（`COB_CHAR_*`）、`fileio.c:2579`（`IS_BAD_CHAR`） | LF・CR・TAB などを判定するマクロ |
| 定数の生成 | `cobc/codegen.c:651` の `output_string`（ソース上のバイト列を C 文字列としてそのまま出力） | 英数字定数はすべてここを通る |
| 表意定数・記号文字 | `cobc/typeck.c:3790-3910`（`low_value` / `high_value`、`SYMBOLIC CHARACTERS`） | |
| モジュール情報 | `libcob/common.h:1283-1286` の `cob_module` にある `decimal_point` / `currency_symbol` / `numeric_separator` | ソース上の文字コードの値が入る |

### 2.3 分析から見えたこと

1. **`#ifdef COB_EBCDIC_MACHINE` の分岐は「EBCDIC を内部コードにしたときに変わる箇所」の一覧として使える。** z/OS 上でのネイティブビルドを想定して書かれた分岐なので、「データが EBCDIC ならこう動くべき」という処理がすでに書かれています（例: `libcob/common.c:1641`、`libcob/numeric.c:4440`、`libcob/fileio.c:449`）。
2. ただしこれらは**プリプロセッサの判定**なので、ASCII ホストでは使えません。**「C コンパイラの文字セット」と「COBOL データの文字セット」を分けて考える**ことが、この提案の出発点になります。
3. 数字の**下位 4 ビットは ASCII（0x30-0x39）でも EBCDIC（0xF0-0xF9）でも 0-9 になる**ので、`COB_D2I` はそのまま使えます。変える必要があるのは `COB_I2D` だけです。

---

## 3. 日本語 IBM EBCDIC の特性

### 3.1 対象とする CCSID

| CCSID | 1 バイト部分 | 2 バイト部分 | 特徴 |
|-------|------------|------------|------|
| **930** | 290 系（半角カナ拡張） | 300 | **英小文字の位置が標準 EBCDIC と違う**（半角カナを優先）。古いシステムに多い |
| **939** | 1027（英小文字拡張） | 300 | 英小文字は CCSID 037 と同じ位置。半角カナは別の位置 |
| **1390** | 930 の拡張 | 16684 | 930 に €（ユーロ記号）と JIS X 0213 などの拡張文字を追加 |
| **1399** | 939 の拡張 | 16684 | 939 に €（ユーロ記号）と拡張漢字を追加。**新規システムで推奨** |

いずれも**SO（0x0E）/ SI（0x0F）で 1 バイトと 2 バイトを切り替える**ステートフルな混在コードです。

### 3.2 実際の配置の確認（glibc iconv で確認）

```
                     'A'  'Z'  '0'  '9'  ' '  'a'  'z'  'ｱ'  '$'  '\'
IBM-037 (米国)        C1   E9   F0   F9   40   81   A9   --   5B   E0
IBM-930              C1   E9   F0   F9   40   62   B9   81   E0   5B(¥)
IBM-939              C1   E9   F0   F9   40   81   A9   59   5B   B2

IBM-930 の a〜z : 62 63 64 65 66 67 68 69 71 72 73 74 75 76 77 78 8B 9B AB B3 B4 B5 B6 B7 B8 B9
IBM-939 の a〜z : 81 82 83 84 85 86 87 88 89 91 92 93 94 95 96 97 98 99 A2 A3 A4 A5 A6 A7 A8 A9

"漢字ｱa" (930) : 0E 4F58 48F2 0F 81 62
"漢字ｱa" (939) : 0E 4F58 48F2 0F 59 81
全角空白 (939)  : 0E 40 40 0F
€ (1399)        : E1
```

設計に影響する点は次のとおりです。

* 英大文字・数字・空白の位置は全 CCSID で共通です（`A`=C1、`0`=F0、空白=40）。
* **英小文字は 930 と 939 で配置がまったく違います。** 930 では `a`〜`z` が飛び飛びに並ぶので、`'a' <= c && c <= 'z'` のような範囲判定は使えません。「汎用 EBCDIC」という固定の表を 1 つ用意するだけでは足りず、**CCSID ごとに文字クラス表を切り替える**必要があります。
* 930 では 0x5B が「¥」、939 では 0x5B が「$」です。`CURRENCY SIGN` や編集記号 `$` の既定値は CCSID によって変わります。
* EBCDIC では英字の並びも連続していません（A-I=C1-C9、J-R=D1-D9、S-Z=E2-E9）。
* 2 バイト空白は 0x4040 です。混在の英数字項目では、SO/SI を含めた**バイト数**で桁数が決まります（IBM の仕様どおり）。

### 3.3 対象外とするもの

富士通 JEF、日立 KEIS、NEC JIPS などのベンダー独自コードは、この提案の直接の対象にはしません。ただし第 4 章の「文字セット記述子」を CCSID 名で切り替えられるようにしておけば、後から追加できます。

---

## 4. 方式の比較と推奨

### 4.1 候補

| 案 | 概要 | 長所 | 短所 |
|----|------|------|------|
| **A. 境界変換を強化する** | 内部は ASCII のまま。CODE-SET、照合順、符号処理を拡充する | 変更が少ない | バイト値に依存するロジック（16 進定数、REDEFINES、HIGH/LOW-VALUE の境界、バイナリ混在レコード）は**本質的に再現できない** |
| **B. モジュール単位で内部コードを選べるようにする（推奨）** | コンパイル時にモジュールごとの「データ文字セット」を決め、libcob は**文字セット記述子**を参照して動く | ASCII モジュールと EBCDIC モジュールを同じ libcob で扱える。段階的に移行できる | libcob の文字定数を全面的に置き換える必要がある。同じプロセス内で両方が混在するときは、CALL の境界で注意が要る |
| **C. libcob をビルド時に EBCDIC 専用にする** | `configure --with-data-charset=ibm-1399` で libcob 全体を EBCDIC にする | ランタイムの分岐がないので速い。既存の `#ifdef` を流用しやすい | ASCII 用と EBCDIC 用の libcob を別々に配る必要がある。同じプロセスで混在できない |

### 4.2 推奨: B（C の利点を取り込む形で実装する）

* libcob の文字定数は、すべて**マクロ経由**（例: `COB_CH_SPACE`）で参照するように書き換えます。
* マクロの中身は、既定では**モジュール単位の記述子**を参照します（案 B）。
* 性能が重要な利用者向けには、configure オプションで**定数に固定**できるようにします（案 C）。

こうすれば、マクロ化という同じ作業から両方の方式を実現できます。

---

## 5. 設計

### 5.1 全体像

```
   COBOL ソース (UTF-8 / Shift_JIS / EUC-JP)
          │  -fsource-codeset=...
          ▼
 ┌───────────────────────────────┐
 │ cobc                          │
 │  ・英数字定数、表意定数、      │
 │    記号文字、編集記号を         │  ← コンパイル時に内部コードへ変換
 │    内部コード (EBCDIC) に変換  │     (iconv / 内蔵の表)
 │  ・X"..." は変換しない         │
 │  ・module->charset を出力      │
 └───────────────────────────────┘
          │  生成された C ソース (データ定数は 8 進エスケープで出力)
          ▼
 ┌───────────────────────────────┐
 │ libcob                        │
 │  cob_charset 記述子            │  ← 空白、ゼロ、符号、文字クラス、
 │   ・MOVE/編集/比較/INSPECT    │     大文字小文字、変換表
 │   ・数値 ⇔ DISPLAY            │
 │  外部との境界でだけ変換:       │
 │   端末 / 環境変数 / コマンド行 │  ← COB_TERMINAL_CODESET など
 │   ファイル名 / CALL 先の名前   │
 │   LINE SEQUENTIAL (任意)       │
 └───────────────────────────────┘
          │
          ▼
   データファイル: EBCDIC のまま (ホストとバイト単位で互換)
```

基本原則は次の 2 つです。

* **プログラムの中（メモリ上のデータ、データファイル）はすべて内部コード（EBCDIC）で扱う。**
* **OS や人と接する境界（端末、環境変数、ファイル名、動的 CALL の名前解決、C 言語との連携）では、ホストのコード（UTF-8 など）に変換する。**

### 5.2 文字セット記述子 `cob_charset`（libcob）

```c
/* libcob/common.h (案) */
typedef struct __cob_charset {
	const char		*name;		/* "IBM-1399" など */
	unsigned int		ccsid;		/* 1399 */
	unsigned char		flags;		/* COB_CS_EBCDIC | COB_CS_MIXED_DBCS ... */

	/* よく使う文字 */
	unsigned char		space, zero, quote, apost;
	unsigned char		plus, minus, period, comma, slash, asterisk;
	unsigned char		currency;	/* 既定の通貨記号 (930: 0x5B=¥) */
	unsigned char		so, si;		/* 0x0E / 0x0F (DBCS がない場合は 0) */
	unsigned char		dbcs_space[2];	/* 0x40 0x40 */

	/* 文字クラス表と大文字小文字の変換表 (256 バイト) */
	const unsigned short	*ctype;		/* COB_CT_DIGIT | COB_CT_UPPER | COB_CT_LOWER | COB_CT_SPACE ... */
	const unsigned char	*to_upper;
	const unsigned char	*to_lower;

	/* ホストのコード (1 バイト部分) との変換表 */
	const unsigned char	*to_host;	/* 内部 → ホスト */
	const unsigned char	*from_host;	/* ホスト → 内部 */

	/* DBCS / 混在の変換 (iconv がなければ内蔵の表を使う) */
	int (*encode) (const char *host, size_t hlen, unsigned char *out, size_t olen);
	int (*decode) (const unsigned char *in, size_t ilen, char *host, size_t hlen);
} cob_charset;
```

* `cob_module` の**末尾**（`statement` の後ろ）に `const cob_charset *charset;` を追加します。末尾に足すのは、既存の拡張（`xml_mode`、`frame_ptr`、`section_name` など）と同じやり方です。libcob は `gc_version` を見て、このメンバーを持たない古いモジュールでは参照しないようにします。値が `NULL` のときは従来どおり ASCII（ネイティブ）として動くので、**既存のバイナリとの互換性を保てます**。
* 記述子の組み込みインスタンスとして `cob_charset_ascii`、`cob_charset_ibm930`、`cob_charset_ibm939`、`cob_charset_ibm1390`、`cob_charset_ibm1399`、`cob_charset_ibm037` を libcob 内に**静的に**持ちます。表は `--gentable` を拡張して生成します（5.6 節）。

### 5.3 libcob の書き換え方

#### (1) 文字定数をマクロにする

```c
/* libcob/coblocal.h (案) */
#define COB_CS            (COB_MODULE_PTR->charset ? COB_MODULE_PTR->charset : &cob_charset_native)
#define COB_CH_SPACE      (COB_CS->space)
#define COB_CH_ZERO       (COB_CS->zero)
#define COB_D2I(x)        ((x) & 0x0F)                  /* ASCII でも EBCDIC でも同じ */
#define COB_I2D(x)        (char)(COB_CH_ZERO | (x))     /* 0x30|x または 0xF0|x */
#define COB_ISDIGIT(c)    (COB_CS->ctype[(unsigned char)(c)] & COB_CT_DIGIT)
#define COB_TOUPPER(c)    (COB_CS->to_upper[(unsigned char)(c)])

#ifdef COB_FIXED_DATA_CHARSET  /* 案 C: configure でビルド時に固定する場合 */
#undef  COB_CS
#define COB_CS            (&COB_FIXED_DATA_CHARSET)
#endif
```

* `memset (p, ' ', n)` は `memset (p, COB_CH_SPACE, n)` に書き換えます（約 53 か所）。
* `isdigit` / `isalpha` / `toupper` などは `COB_ISDIGIT` などに書き換えます。**C のロケール関数は使いません**（ASCII ホストでは EBCDIC のバイトを正しく判定できないため）。
* ホストの文字列（ファイル名やエラーメッセージ）を扱う箇所は**今のまま C の文字定数を使います**。どちらの文字列を扱っているかをコードレビューで明確に分けることが、この作業で一番大事な点です。

#### (2) `#ifdef COB_EBCDIC_MACHINE` を 2 つに分ける

| 新しいマクロまたは判定 | 意味 |
|--------------------|------|
| `COB_EBCDIC_HOST`（今の `COB_EBCDIC_MACHINE` の名前を変えたもの） | C コンパイラやホスト OS が EBCDIC かどうか。ファイル名、端末、エラーメッセージの扱いに使う |
| `COB_DATA_IS_EBCDIC`（実行時の判定: `COB_CS->flags & COB_CS_EBCDIC`） | COBOL データが EBCDIC かどうか。符号、数字、編集、行順ファイルの内容判定に使う |

`libcob/common.c`、`libcob/numeric.c`、`libcob/move.c`、`libcob/fileio.c` にある `#ifdef COB_EBCDIC_MACHINE` の分岐を一つずつ見直し、どちらに当たるかを振り分けます。

#### (3) 符号処理

内部が EBCDIC なら、ゾーン 10 進の符号は**ゾーン部（上位 4 ビット）の C / D / F**で表せます（`{`=C0、`A`-`I`=C1-C9、`}`=D0、`J`-`R`=D1-D9 なので、今のオーバーパンチの表はそのままゾーン部の操作と同じ意味になります）。`cob_get_sign_ebcdic` / `cob_put_sign_ebcdic`（`libcob/common.c:1769`、`1874`）は、EBCDIC 内部のときは**ビット演算による高速版**を使うようにします。`SIGN SEPARATE` の `+`（0x4E）と `-`（0x60）は記述子から取ります。

#### (4) 比較と照合順

内部が EBCDIC で `PROGRAM COLLATING SEQUENCE` が NATIVE なら、`memcmp` で比較するだけで**EBCDIC 順になります**（今のように表を引く必要がなくなり、速くなります）。`ALPHABET ... IS STANDARD-1` / `ASCII` を指定したときに変換表を引く、という今とは逆の関係になります（`cobc/codegen.c:3590-3780` の `cob_ascii_ebcdic` と `cob_ebcdic_ascii` の使い方を入れ替えます）。

#### (5) 境界での変換（外部インターフェース）

| 境界 | 変換 | 該当箇所 |
|------|------|--------|
| `DISPLAY` / `ACCEPT`（端末）、`screenio` | 内部 → ホスト（DBCS なら SO/SI を外して UTF-8 や Shift_JIS にする） | `libcob/termio.c`, `libcob/screenio.c` |
| `ACCEPT FROM ENVIRONMENT` / `ARGUMENT-VALUE` / `COMMAND-LINE` | ホスト → 内部 | `libcob/common.c` |
| `ASSIGN` のファイル名（データ項目のとき） | 内部 → ホスト | `libcob/fileio.c` |
| `CALL identifier` で呼ぶプログラム名 | 内部 → ホスト（`dlsym` で探すため） | `libcob/call.c:1118` 付近 |
| `FUNCTION CURRENT-DATE`、`WHEN-COMPILED`、例外メッセージなど、libcob が作る文字列 | ホスト → 内部 | `libcob/intrinsic.c`, `libcob/common.c` |
| `cob_get_*` / `cob_put_*` の C API | 引数で変換するかどうかを選べるようにする | `libcob/common.c` |
| XML / JSON GENERATE・PARSE | 生成する文書の CCSID を選べるようにする | `libcob/mlio.c` |

#### (6) ファイル入出力

* **順ファイル（固定長・可変長）・相対ファイル・索引ファイル**: 変換しません。ホストのデータとバイト単位で互換になります。索引ファイルのキーも EBCDIC 順で並びます。
* **LINE SEQUENTIAL**: 実行時設定 `COB_LS_CODESET`（`native` / `host`）を追加し、テキストファイルとしてホストのコードで読み書きするかどうかを選べるようにします。レコードの区切り（LF）の判定は `fileio.c:449` の `COB_CHAR_*` を記述子から取るように変えます。
* **CODE-SET 句**: 内部が EBCDIC のとき、`CODE-SET IS STANDARD-1` は「ASCII のファイルを読み書きする」という意味になります。今の実装を逆向きにすればそのまま使えます。

### 5.4 cobc の変更

| 項目 | 内容 | 主な箇所 |
|------|------|--------|
| オプションの追加 | `-fdata-codeset=<name>`（`native` / `ibm-930` / `ibm-939` / `ibm-1390` / `ibm-1399` / `ibm-037` / `ttbl:<file>`）と `-fsource-codeset=<name>`（`utf-8` / `shift_jis` / `cp932` / `euc-jp`）。設定ファイルのキー `data-codeset:` も追加する | `cobc/flag.def`, `cobc/config.def`, `cobc/cobc.c` |
| 方言の既定値 | `ibm-strict.conf` と `mvs-strict.conf` に `data-codeset: native` を明示する（既定値は変えない）。`ibm-jp.conf`（CCSID 1399 と `binary-byteorder: big-endian` などを組み合わせたもの）を新しく作る | `config/` |
| 英数字定数 | 字句解析はソースのコードのまま行い、**型チェックのあと、コード生成の前**に内部コードへ変換する（定数の比較や長さの検査はソース側のコードで済ませるため）。変換したあとの長さでもう一度桁あふれを検査する | `cobc/typeck.c`, `cobc/tree.c` の `cb_build_alphanumeric_literal` |
| 16 進定数 `X"..."` | **変換しない**（IBM の意味どおり、書いたバイトがそのままデータになる） | |
| 日本語定数 `N"..."` / `G"..."` / 混在定数 | DBCS（PIC G / PIC N の DISPLAY-1）は SO/SI を付けない 2 バイト列にする。英数字項目の混在定数は SO/SI を入れた長さで数える | `cobc/scanner.l`, `cobc/typeck.c` |
| 表意定数 | `SPACE`、`ZERO`、`QUOTE` は記述子の値にする。`LOW-VALUE` / `HIGH-VALUE` は、照合順が NATIVE なら 0x00 / 0xFF のまま | `cobc/typeck.c:3790-3910` |
| `SYMBOLIC CHARACTERS` | 序数を内部コードの位置として解釈する（`ebcdic-symbolic-characters: yes` と同じ意味になる） | 同上 |
| 編集記号、`CURRENCY SIGN`、`DECIMAL-POINT IS COMMA` | `cob_module` の `currency_symbol` / `decimal_point` / `numeric_separator` は、PICTURE 記号と同じく**ホストのコードのまま**にする。libcob がデータに書き込む・データと比べるときに `COB_CS_FROM_HOST` で変換する（P0 の実装で決定。当初案の「内部コードの値を入れる」から変更） | `libcob/move.c` |
| C ソースの生成 | `output_string`（`codegen.c:651`）は、データ定数を**すべて 8 進エスケープで出力する**（EBCDIC のバイトを C のソースに文字として書くと、C コンパイラが解釈を誤るため） | `cobc/codegen.c` |
| モジュール情報 | `module->charset = &cob_charset_ibm1399;` などを出力する | `cobc/codegen.c` |
| `CLASS` 条件 | `ALPHABETIC` / `ALPHABETIC-LOWER` / `ALPHABETIC-UPPER` / 利用者定義の CLASS の範囲（`'A' THRU 'Z'`）は、**ソースのコードの文字集合**として展開してから、内部コードの集合に変換する（EBCDIC では英字の並びが連続していないため） | `cobc/typeck.c` |
| 組込み関数 | `ORD` / `CHAR` / `UPPER-CASE` / `LOWER-CASE` / `NUMVAL` / `NATIONAL-OF` / `DISPLAY-OF` は記述子を使う | `libcob/intrinsic.c` |

### 5.5 日本語（DBCS）への対応

* **PIC G / PIC N USAGE DISPLAY-1**: IBM と同じく **SO/SI を付けない 2 バイトの DBCS** として扱います。空白で埋めるときは 0x4040 にします。
* **英数字項目の混在データ**: 実行時には**バイト列として扱います**（IBM と同じ）。境界で変換するときにだけ SO/SI を解釈します。
* **USAGE NATIONAL（UTF-16）**: `NATIONAL-OF` / `DISPLAY-OF` で、CCSID 1399 と UTF-16 の間を iconv で変換します（今は `typeck.c:4072` で UTF-16BE を仮定しています）。
* **端末への出力**: `DISPLAY` するとき、SO から SI までを DBCS として UTF-8 などへ変換します。変換できないバイトは実行時設定で置き換え文字にします。

### 5.6 変換表と iconv

* libcob の iconv は**任意**のままにします。iconv がない環境のために、930/939/1390/1399 の**1 バイト部分の表**と、**DBCS ⇔ Unicode の表**（約 1.2 万字）を `--gentable` を拡張して作り、libcob に組み込みます（または `config/*.ttbl` として同梱します）。
* `--gentable` の拡張: `--gentable=IBM-1399,UTF-8,dbcs` のようにして、ステートフルなコードを SO/SI で囲みながら 1 文字ずつ変換し、2 バイト部分の表を出力します。
* **利用者定義文字（外字）**: 0x6941-0x7FFE などの外字領域は、**利用者が用意した対応表**（`COB_DBCS_USER_TABLE`）で Unicode の私用領域などに対応付けられるようにします。移行案件でもっとも問題になる点なので、最初から仕組みを用意しておきます。

---

## 6. 段階的な実装計画

| 段階 | 内容 | 成果物 | 規模の目安 |
|------|------|--------|-----------|
| **P0: 準備** | `COB_EBCDIC_MACHINE` を `COB_EBCDIC_HOST` と `COB_DATA_IS_EBCDIC` に分け、libcob の文字定数をマクロに置き換える（**動作は変えない**）。今のテストがすべて通ることを確認する | リファクタリングのパッチ | 約 2〜3 人月 |
| **P1: SBCS の EBCDIC 内部化** | `cob_charset` と `-fdata-codeset=ibm-037 / ibm-939`。定数変換、表意定数、符号、編集、比較、INSPECT、組込み関数、端末との境界での変換 | 英数字と数値だけのプログラムが EBCDIC 内部で動く | 約 3〜4 人月 |
| **P2: 日本語 SBCS** | 930 / 1390（英小文字の配置が違うもの）、半角カナ、`CURRENCY SIGN` の ¥ | 日本語の SBCS に対応 | 約 1 人月 |
| **P3: DBCS** | PIC G / N、混在定数、NATIONAL との変換、端末での DBCS 表示、外字の対応表 | 漢字を含むプログラムが動く | 約 3〜4 人月 |
| **P4: ファイルと周辺機能** | LINE SEQUENTIAL の選択、逆向きの CODE-SET、XML / JSON、REPORT WRITER、SCREEN SECTION | 周辺機能の対応 | 約 2 人月 |
| **P5: 性能と固定ビルド** | `--with-data-charset` による固定ビルド（案 C）とベンチマーク | 性能を重視する利用者向け | 約 1 人月 |

※ 人月は目安です。P0 は機械的な作業が多いので、差分を小さく分けて提出すれば、上流（GnuCOBOL プロジェクト）にも取り込んでもらいやすくなります。

---

## 7. 試験の方針

1. **既存のテストスイートを EBCDIC モードでも回す**: `tests/testsuite.src/*.at` を `COBC_FLAGS=-fdata-codeset=ibm-1399` を付けて実行します。端末との境界で変換するので、**DISPLAY の出力は ASCII モードのときと同じになるはず**です。これが大きな回帰テストになります。違いが出るのは照合順とバイト値に依存するテストだけなので、それらは期待値を 2 つ用意します。
2. **新しいテスト `run_ebcdic.at`**:
   * `MOVE`、`REDEFINES`、`X"C1"` と `'A'` の等価性、HIGH / LOW-VALUE、符号のゾーン部（C / D / F）のバイト値の検証
   * 930 と 939 での英小文字、半角カナ、`ALPHABETIC-LOWER`、`UPPER-CASE` の結果
   * SO/SI を含む混在定数の長さ、PIC G の 0x4040 埋め、DBCS の DISPLAY
   * SORT、索引ファイルのキー順が EBCDIC 順になること
3. **ホストとの突き合わせ**: z/OS の Enterprise COBOL で作ったファイルと GnuCOBOL の出力を、**バイト単位で比較**します（移行案件の受入試験の雛形として `tests/` の外で用意します）。
4. **性能測定**: `tests/cobol85`（NIST）と主要なベンチマークで、ASCII モード・EBCDIC モード（記述子あり）・固定ビルドの 3 つを比べます。

---

## 8. リスクと対策

| リスク | 影響 | 対策 |
|--------|------|------|
| libcob の中で「ホストの文字列」と「COBOL のデータ」を取り違える | 文字化け、誤動作 | P0 で型や命名を分ける（`host_` という接頭辞を付けるなど）。EBCDIC モードでテストスイート全体を回す |
| 記述子を参照するぶん遅くなる | 数 % 程度の性能低下（見込み） | よく使う経路はローカル変数にキャッシュする。固定ビルド（案 C）も用意する |
| C プログラムとの連携（外部の C 関数が ASCII を期待している） | 引数が文字化けする | `CALL ... USING BY REFERENCE` ではデータを変換しない（IBM と同じ）ことを明記する。変換用のヘルパー API（`cob_to_host` / `cob_from_host`）を提供する |
| ASCII のモジュールと EBCDIC のモジュールが CALL で混在する | 引数の中身が食い違う | モジュールの記述子を CALL 時に検査し、違っていれば警告する（実行時設定で無効にもできる） |
| 外字、ベンダー独自の文字 | 変換できない | 利用者定義の対応表、置き換え文字、変換エラーを検出する機能を用意する |
| 既存の索引ファイルのキー順が変わる | ASCII 版で作った索引ファイルと互換がなくなる | EBCDIC モードでは**新しく作り直す**ことを移行手順に明記する。`cobfile` などの変換ツールも用意する |
| 上流に取り込まれない | 保守の負担が増える | P0 のリファクタリングは上流に提案する。記述子の API は小さく保つ |

---

## 9. 利用イメージ

```sh
# ソースは UTF-8、内部データは CCSID 1399、IBM 方言
cobc -x -std=ibm -fdata-codeset=ibm-1399 -fsource-codeset=utf-8 prog.cbl

# 実行時: 端末は UTF-8、LINE SEQUENTIAL はホストのコード (UTF-8) で読み書き
COB_TERMINAL_CODESET=UTF-8 COB_LS_CODESET=host ./prog
```

```cobol
       01  WK-REC.
           05  WK-CODE     PIC X(4)  VALUE 'AB12'.   *> C1 C2 F1 F2
           05  WK-KANJI    PIC G(2)  VALUE G'漢字'.  *> 4F58 48F2
           05  WK-AMT      PIC S9(3) VALUE -123.     *> F1 F2 D3
       ...
           IF WK-CODE(1:1) = X'C1'                   *> 真 (ホストと同じ)
```

---

## 10. 進捗状況

### P0（動作を変えない下準備）: 完了（screenio.c / mlio.c / reportio.c は P4 に延期）

ブランチ `claude/ebcdic-code-proposal-oqkyol` で実施しています。どの段階でも、既存のテストスイートの結果が変わらないこと（1367 件が期待どおり、10 件スキップ）を確認しています。

| 項目 | 状態 | 内容 |
|------|------|------|
| 文字セット記述子 | 完了 | `cob_charset`（`libcob/common.h`）、`cob_charset_native` と `cob_module_charset`（`libcob/charset.c`）、`cob_module->charset`（`NULL` ならネイティブ） |
| マクロ | 完了 | `COB_CS`、`COB_CH_SPACE` / `COB_CH_ZERO` など、`COB_CS_ISDIGIT` などの文字クラス判定、`COB_CS_FROM_HOST` / `COB_CS_TO_HOST`、`COB_CS_I2D`（`libcob/coblocal.h`） |
| `COB_EBCDIC_HOST` | 完了 | `COB_EBCDIC_MACHINE` の別名として追加（互換のため旧名も残す） |
| move.c | 完了 | MOVE、数値編集、逆編集、C API（`cob_put_s64_pic9`、`cob_get_picx` など） |
| numeric.c | 完了 | DISPLAY と 10 進の相互変換（GMP にはホストの数字を渡す）、数値 DISPLAY の比較での符号処理 |
| common.c | 完了 | 符号処理、`cob_correct_numeric`、NUMERIC / ALPHABETIC 判定、SPACE / ZERO との比較、CBL_TOUPPER / CBL_TOLOWER、C$JUSTIFY、C$CALLEDBY、SYSTEM、CHAINING、ACCEPT FROM 環境変数・コマンド行 |
| cconv.c | 完了 | `cob_field_to_string` はホストのコードに変換する。変換しない `cob_field_to_string_raw` を追加し、reportio.c ではこちらを使う |
| strings.c | 完了 | INSPECT / STRING / UNSTRING の表意定数 |
| 変換ヘルパー | 完了 | `cob_charset_from_host` / `cob_charset_to_host`（バッファを変換）、`cob_charset_field_to_host` / `cob_charset_field_release`（項目のホスト用一時コピー） |
| intrinsic.c | 完了 | ホストで組み立てた結果（日付、例外情報、モジュール情報など）はデータ文字コードに変換する。解析が必要な引数（NUMVAL 系、日付・時刻の書式、LOCALE-COMPARE）はホスト用コピーにしてから解析する。TRIM、UPPER-CASE / LOWER-CASE、BIT-OF / BIT-TO-CHAR、HEX-OF / HEX-TO-CHAR などはデータ文字コードで処理する |
| termio.c | 完了 | DISPLAY はホストへ、ACCEPT はホストから変換する。`-fdump` の 16 進ダンプは未対応（FIXME を記載済み） |
| call.c | 完了 | CALL 先の名前は `cob_field_to_string` 経由でホストに変換済み。`cob_put_field_str` は C 文字列をデータ文字コードに変換する |
| fileio.c | 完了 | LINE SEQUENTIAL はホストのテキストファイルとして、読み書き時にレコードを変換する（CODE-SET 指定時は変換しない）。FILE STATUS、ファイル名、CBL_ 系のファイル操作ルーチン、CBL_GET_CURRENT_DIR を変換する。EXTFH / FCD のファイル名は未対応（FIXME を記載済み） |
| screenio.c、mlio.c、reportio.c | P4 に延期 | SCREEN SECTION、XML / JSON、REPORT WRITER は提案どおり P4 で対応する |
| CBL_GC_GETOPT | 未着手 | 長いオプション名をホストのコードへ変換する必要がある（FIXME を記載済み） |

### P0 で分かったこと

* ALPHABETIC などのクラス判定と CBL_TOUPPER / CBL_TOLOWER は、今の実装では**実行時のロケール（LC_CTYPE）に依存**しています。動作を変えないよう、ネイティブ文字セットのときは従来どおり C ライブラリの関数を使い、それ以外のときだけ記述子の表を使います。
* `COB_EBCDIC_MACHINE` の分岐の中に、ASCII ホストでは一度もコンパイルされていなかったバグがありました（符号付き `char` と `0xC0` を比べていた）。実行時の分岐に変えたので、ここは修正しました。
* cobc が生成するコードは `COB_SPACES_ALPHABETIC` / `COB_ZEROES_ALPHABETIC`（ネイティブの SPACE / ZERO）を直接参照しています（`cobc/typeck.c:7196` 付近）。P1 では、EBCDIC のモジュールでこの最適化を使わないか、記述子の `spaces` / `zeroes` を参照するように変える必要があります。

### P1（SBCS の EBCDIC 内部化）: 実験的に動作

`cobc -fdata-codeset=IBM-1399 prog.cob` のように指定すると、データを EBCDIC で持つプログラムが作れます。指定できるのは `NATIVE`、`IBM-037`、`IBM-930`、`IBM-939`、`IBM-1390`、`IBM-1399` です（DBCS は P3 のため、現時点では 1 バイト部分のみ）。

| 項目 | 内容 |
|------|------|
| 変換表 | `build_aux/gen-charset-tables.py` が iconv から `libcob/charset_tables.h` を生成する。ホスト側は 037 が ISO-8859-1、日本語 CCSID が Shift_JIS の 1 バイト部分（ASCII と半角カナ）。対応する文字がない位置は空いているバイトに割り当て、往復で失われないようにした |
| 記述子 | `cob_charset_ibm037` など 5 つと、名前から探す `cob_get_charset`（`libcob/charset.c`） |
| cobc | 英数字定数、数値定数、表意定数、VALUE / INITIALIZE、MOVE の最適化、CLASS 条件、DEBUG-ITEM、`-fdefaultbyte` の文字指定、SUPPRESS WHEN を、コード生成の直前にデータ文字コードへ変換する。16 進定数（`X"..."`）は変換しない（`cb_literal.raw`）。リテラル同士の比較はコンパイル時に畳み込まない |
| 符号 | EBCDIC データでは常にゾーン部の符号（C / D / F）を使う（`cb_ebcdic_sign` を強制） |
| 試験 | `tests/testsuite.src/run_ebcdic.at` に 7 件を追加（16 進定数、ゾーン 10 進 / パック 10 進のバイト値、照合順、930 / 939 の英小文字と CLASS、ACCEPT / DISPLAY、SEQUENTIAL / LINE SEQUENTIAL ファイル） |

確認できた動作（CCSID 1399 / 930）:

```
'AB12' → C1 C2 F1 F2      X'C1' = 'A'      S9(3) -123 → F1 F2 D3      COMP-3 -123 → 12 3D
英小文字 'abz' → 81 82 A9（1399 / 939）、62 63 B9（930）
'a' < 'A'、'9' > 'Z'（EBCDIC の照合順）
DISPLAY / ACCEPT と LINE SEQUENTIAL はホストのテキストとして読み書き
```

#### 既存テストスイートを EBCDIC モードで流した結果

`COBOL_FLAGS=-fdata-codeset=IBM-037 make check` で既存テストをすべて EBCDIC モードで実行しました（ネイティブでは全件成功のまま）。想定外の失敗は、最初の 153 件から修正を重ねて 112 件になり、残りはすべて次のどれかに当たります。

| 分類 | 件数 | 理由 |
|------|------|------|
| レポートライター | 23 | P4 で対応（reportio.c は未変換） |
| XML / JSON | 27 | P4 で対応（mlio.c は未変換） |
| EXTFH / FCD | 11 | FCD のファイル名などのホスト変換が未対応（FIXME） |
| C との連携 | 12 | C 側が ASCII の文字列やバイト値を直接読み書きしている。IBM と同じく CALL の引数は変換しないので、想定どおりの差分 |
| ASCII のバイト値・符号を前提にしたテスト | 19 | `X"313233"` を "123" として表示する、16 進ダンプで 0x20 を期待する、ASCII の符号オーバーパンチを期待する、など |
| 照合順・序数 | 14 | EBCDIC 順になるため（ALPHABET / COLLATING SEQUENCE、CHAR / ORD、CLASS の序数指定など） |
| その他 | 6 | コンパイル時の定数比較の警告（畳み込みをやめたため）、`-fdump` の 16 進値、CBL_GC_GETOPT（FIXME）、ACU 方言の既定バイト（空白）で初期化した索引の値 |

この過程で、次の変換漏れや旧コードの不具合を見つけて直しました。

* エラーメッセージの行番号が、データ用の数字変換を使っていたため文字化けしていた
* EBCDIC の符号処理で、符号付きのゾーン 10 進から符号を外す（unpunch）処理が抜けていた（旧 `COB_EBCDIC_MACHINE` の分岐と同じ不具合）。符号付き数値を英数字項目に MOVE すると、符号のゾーンが残っていた
* `cob_get_s64_pic9` が EBCDIC で末尾の空白を数字として数えていた
* DEBUG-ITEM、`-fdefaultbyte=<文字>`、SUPPRESS WHEN の文字、C$PRINTABLE、CBL_GC_HOSTED、CBL_ERROR_PROC のメッセージ、VFILE の状態コード、LOCALE-DATE / LOCALE-TIME、SUBSTITUTE-CASE、`-fdump` の表示

#### P1 の残り（完了）

* `ALPHABET ... IS STANDARD-1 / ASCII / EBCDIC`、既定の照合順（`-fdefault-colseq`）、`CODE-SET` は、`COB_EBCDIC_MACHINE` ではなく「データが EBCDIC か」で判断するようにした。ASCII と EBCDIC の変換表も、データ文字コードの表を使う。これで EBCDIC データのプログラムでも、`CODE-SET IS STANDARD-1` のファイルは ASCII で読み書きされ、`SEQUENCE IS STANDARD-1` の SORT は ASCII 順になる
* 設定ファイルの項目 `data-codeset:` を追加した（すべての方言ファイルに `native` を明記。`-fdata-codeset` が優先）
* 方言 `ibm-jp`（`config/ibm-jp.conf`：IBM 方言＋CCSID 1399）を追加した。`cobc -std=ibm-jp` で使える
* 見つけた既存の不具合: COLLATING SEQUENCE を指定しない SORT / MERGE が、直前の SORT で指定した照合順を引き継いでいた（ネイティブでは ASCII 指定が「表なし」になるため表面化していなかった）

### P3（日本語 DBCS）: 混在データを実装（PIC G / N は未対応）

GnuCOBOL 3.x では PIC G / DISPLAY-1（DBCS）が未実装（PENDING）、PIC N（NATIONAL）も UTF-16 の仮実装です。PIC G / N の本格対応はコンパイラの型の作業になるため、まず実務でいちばん多い **PIC X 項目の中の日本語（SO / SI で囲まれた混在データ）** に対応しました。

| 項目 | 内容 |
|------|------|
| 実行時の変換 | DISPLAY、ACCEPT、LINE SEQUENTIAL、環境変数・コマンド行は、iconv でデータ文字コード（例: `IBM1399`）とホストの文字コードを変換する。ホストの文字コードは環境変数 `COB_HOST_CODESET`、なければロケール（`nl_langinfo (CODESET)`）から決める。ASCII のロケール（`C` など）では従来どおり 1 バイトの表を使う |
| 日本語の定数 | ソースの英数字定数に ASCII 以外の文字があれば、字句解析の時点で iconv によりソースの文字コード（`-fsource-codeset`、既定 UTF-8）からデータ文字コードへ変換する。`'日本語'` は `0E 4562 4566 48E7 0F` のように SO / SI 付きの 8 バイトになり、項目の長さの検査もこの長さで行う |
| 連結 | 16 進定数や変換済みの定数と通常の定数を `&` で連結した場合も正しく変換する |
| 試験 | 日本語の定数、ACCEPT（UTF-8 入力）、LINE SEQUENTIAL の往復、`-fsource-codeset` の誤り |

確認できた動作（CCSID 1399、端末 UTF-8 / Shift_JIS）:

```
VALUE 'ｺﾝﾆﾁﾊ 漢字ABC' → 70 BC 8D 77 9B 40 0E 4F58 48F2 0F C1 C2 C3 ...
ACCEPT '日本ｶﾅx'     → 0E 4562 4566 0F 66 8C A7
DISPLAY / LINE SEQUENTIAL はホストの文字コードの日本語として出力される
```

### 次の作業

* P3 の残り: PIC G / PIC N（USAGE DISPLAY-1、SO / SI なしの DBCS）、G"..." / N"..." 定数、NATIONAL-OF / DISPLAY-OF、外字の対応表、DBCS の途中で切れた場合の扱い
* P4: SCREEN SECTION、XML / JSON、REPORT WRITER、EXTFH

## 11. まとめ

* 今の GnuCOBOL の EBCDIC 対応は「**内部は ASCII のまま、境界で変換する**」ものです。バイト値に依存するホストのロジックは完全には再現できません。
* `#ifdef COB_EBCDIC_MACHINE` の分岐と `-fsign=EBCDIC` などの既存の仕組みには、EBCDIC 内部化に**使える部品がそろっています**。足りないのは次の 2 つです。
  * **「C の文字セット」と「データの文字セット」を分けること**
  * **CCSID ごとの文字セット記述子**
* 推奨は **案 B（モジュール単位の文字セット記述子）** です。まず P0（動作を変えないマクロ化）で土台を作り、SBCS → 日本語 SBCS → DBCS の順に段階的に実装します。
* 日本語の IBM EBCDIC では、**930 と 939 で英小文字の配置がまったく違う**ことと、**SO/SI を使う DBCS と外字**が最大の注意点です。新しく導入するなら CCSID 1399 を既定にすることを勧めます。
