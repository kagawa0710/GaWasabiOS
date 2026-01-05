# p501 - USB HID Report Descriptor パーシング

## 前回（p477）からの追加

p477ではHID Report Descriptorの取得（バイト列としてのhexdump表示）を実装した。今回はReport Descriptorをパースして、各フィールドの意味を解釈できるようにする。

---

## 変更の概要

```
usb.rs:
  - HidDescriptor構造体追加
  - UsbDescriptor::Hid バリアント追加
  - pick_interface_with_triple() 戻り値変更
  - request_hid_report_descriptor() にdesc_sizeパラメータ追加

tablet.rs:
  - Report Descriptor パーシング実装
  - UsbHidReportInputItem構造体
```

---

## USB HID Report Descriptor の構造

### Item の基本構造
```
+--------+--------+--------+--------+
| Prefix |  Data (0-4 bytes)        |
+--------+--------+--------+--------+

Prefix byte:
  bit[0..=1]: bSize (0=0, 1=1, 2=2, 3=4 bytes)
  bit[2..=3]: bType (0=Main, 1=Global, 2=Local, 3=Reserved)
  bit[4..=7]: bTag (item-specific tag)
```

### Item Types

| bType | 名前 | 説明 |
|-------|------|------|
| 0 | Main | Input/Output/Collection等の構造定義 |
| 1 | Global | Usage Page/Report Size/Count等（次のMainまで有効） |
| 2 | Local | Usage/Usage Min/Max等（次のMainで消費） |
| 3 | Reserved | 未使用 |

### Main Items (bTag)

| bTag | 名前 | 説明 |
|------|------|------|
| 0b1000 | Input | 入力レポートのフィールド定義 |
| 0b1001 | Output | 出力レポートのフィールド定義 |
| 0b1010 | Collection | コレクション開始 |
| 0b1100 | End Collection | コレクション終了 |

### Global Items (bTag)

| bTag | 名前 | 説明 |
|------|------|------|
| 0b0000 | Usage Page | 用途ページ（0x01=Generic Desktop, 0x09=Button） |
| 0b0001 | Logical Minimum | 論理値の最小値 |
| 0b0010 | Logical Maximum | 論理値の最大値 |
| 0b0111 | Report Size | 1フィールドのビット数 |
| 0b1001 | Report Count | フィールドの繰り返し回数 |

### Local Items (bTag)

| bTag | 名前 | 説明 |
|------|------|------|
| 0 | Usage | 用途（X/Y/Button等） |
| 1 | Usage Minimum | 用途範囲の最小 |
| 2 | Usage Maximum | 用途範囲の最大 |

---

## 新規追加: HidDescriptor

### usb.rs への追加
```rust
#[derive(Debug, Copy, Clone, Default)]
#[repr(packed)]
pub struct HidDescriptor {
    pub desc_length: u8,           // 9
    pub desc_type: u8,             // 0x21 (HID)
    pub hid_release: u16,          // HID仕様バージョン
    pub country_code: u8,
    pub num_descriptors: u8,       // 通常1
    pub descriptor_type: u8,       // 0x22 (Report)
    pub report_descriptor_length: u16,  // ← これが重要！
}
```

### UsbDescriptor enumへの追加
```rust
pub enum UsbDescriptor {
    Config(ConfigDescriptor),
    Endpoint(EndpointDescriptor),
    Hid(HidDescriptor),           // 追加
    Interface(InterfaceDescriptor),
    Unknown { desc_len: u8, desc_type: u8 },
}
```

---

## pick_interface_with_triple() の変更

### 変更前
```rust
) -> Option<(..., Vec<EndpointDescriptor>)>
```

### 変更後
```rust
) -> Option<(..., Vec<UsbDescriptor>)>
```

これにより、Endpoint以外のDescriptor（HidDescriptor等）も返せるようになる。

---

## tablet.rs の実装

### パーサーの状態変数
```rust
let mut usage_page: Option<UsbHidUsagePage> = None;  // Global: 保持
let mut usage_queue = VecDeque::new();               // Local: Mainで消費
let mut usage_min = None;                            // Local: Mainで消費
let mut usage_max = None;                            // Local: Mainで消費
let mut report_size = 0;                             // Global: 保持
let mut report_count = 0;                            // Global: 保持
let mut logical_min = 0;                             // Global: 保持
let mut logical_max = 0;                             // Global: 保持
let mut bit_offset = 0;                              // 累積ビット位置
```

### UsbHidReportInputItem
```rust
pub struct UsbHidReportInputItem {
    pub usage: UsbHidUsage,     // X, Y, Button(1), etc.
    pub bit_size: usize,        // フィールドのビット数
    pub is_array: bool,         // 配列フラグ
    pub is_absolute: bool,      // 絶対座標フラグ
    pub bit_offset: usize,      // レポート内のビット位置
    pub logical_min: u32,       // 論理最小値
    pub logical_max: u32,       // 論理最大値
}
```

### パーシングの流れ
```
1. Prefixバイトを読む
2. bSize, bType, bTagを抽出
3. Dataバイトを読む（bSizeバイト）
4. bTypeに応じて処理:
   - Global: 状態変数を更新（次のMainまで保持）
   - Local: usage_queue等に追加（次のMainで消費）
   - Main(Input): report_count回ループしてUsbHidReportInputItemを生成
5. Main後にLocal状態をクリア
```

---

## QEMU USB Tablet の Report Descriptor 例

```
05 01       Usage Page (Generic Desktop)
09 01       Usage (Pointer)
A1 01       Collection (Application) {
  09 01       Usage (Pointer)
  A1 00       Collection (Physical) {
    05 09       Usage Page (Button)
    19 01       Usage Minimum (1)
    29 03       Usage Maximum (3)
    15 00       Logical Minimum (0)
    25 01       Logical Maximum (1)
    95 03       Report Count (3)
    75 01       Report Size (1)
    81 02       Input (Data, Variable, Absolute) ← Button x3
    95 01       Report Count (1)
    75 05       Report Size (5)
    81 01       Input (Constant) ← パディング5bit
    05 01       Usage Page (Generic Desktop)
    09 30       Usage (X)
    09 31       Usage (Y)
    15 00       Logical Minimum (0)
    26 FF 7F   Logical Maximum (0x7FFF)
    35 00       Physical Minimum (0)
    46 FF 7F   Physical Maximum (0x7FFF)
    75 10       Report Size (16)
    95 02       Report Count (2)
    81 02       Input (Data, Variable, Absolute) ← X, Y
  } End Collection
} End Collection
```

### パース結果
```
UsbHidReportInputItem { usage: Button(1), bit_size: 1, bit_offset: 0, ... }
UsbHidReportInputItem { usage: Button(2), bit_size: 1, bit_offset: 1, ... }
UsbHidReportInputItem { usage: Button(3), bit_size: 1, bit_offset: 2, ... }
UsbHidReportInputItem { usage: Constant,  bit_size: 5, bit_offset: 3, ... }
UsbHidReportInputItem { usage: X, bit_size: 16, bit_offset: 8, is_absolute: true, ... }
UsbHidReportInputItem { usage: Y, bit_size: 16, bit_offset: 24, is_absolute: true, ... }
```

---

## Input レポートの構造（パース結果から判明）

```
Byte 0:    [Button3][Button2][Button1][---padding 5bit---]
Bytes 1-2: X座標 (16bit little-endian, 0-0x7FFF)
Bytes 3-4: Y座標 (16bit little-endian, 0-0x7FFF)
```

Total: 5 bytes (40 bits)

---

## この段階でできること

- HidDescriptorからreport_descriptor_lengthを取得
- 正しいサイズでReport Descriptorを要求
- Report Descriptorをパースしてフィールド情報を抽出
- 各フィールドのビット位置・サイズ・用途を把握

## 次のステップ

- UsbHidReportInputItemを使ってInputレポートを解釈
- マウス座標の絶対値→画面座標への変換
- ボタン状態の取得
- Interrupt転送への移行（ポーリングから割り込みへ）
