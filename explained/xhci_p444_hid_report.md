# p444 - HID Report取得 / USBキーボード

## 前回（p437）からの追加

p437ではEndpoint Descriptorを実装した。今回はUSBキーボードを設定してHID Reportを読み取る。

---

## 期待される出力

```
[INFO] xhci: hid report: [0, 0, 0, 0, 0, 0, 0, 0]
[INFO] xhci: hid report: [0, 0, 4, 0, 0, 0, 0, 0]  // 'A'キー押下
```

---

## 処理の流れ

```
1. Config Descriptor取得（p434）
2. Boot Keyboard Interface検索 (3, 1, 1)
3. SET_CONFIGURATION
4. SET_INTERFACE
5. SET_PROTOCOL (Boot Protocol)
6. GET_REPORT ループ（ポーリング）
```

---

## Boot Keyboard Interface の判別

| フィールド | 値 | 意味 |
|-----------|-----|------|
| interface_class | 3 | HID (Human Interface Device) |
| interface_subclass | 1 | Boot Interface |
| interface_protocol | 1 | Keyboard (2ならMouse) |

```rust
if let (3, 1, 1) = e.triple() {
    boot_keyboard_interface = Some(e)
}
```

---

## 新規追加: UsbHidProtocol enum

```rust
// [hid_1_11]:
// 7.2.5 Get_Protocol Request
// 7.2.6 Set_Protocol Request
#[repr(u8)]
pub enum UsbHidProtocol {
    BootProtocol = 0,
    ReportProtocol = 1,
}
```

---

## SetupStageTrb の汎用化

```rust
impl SetupStageTrb {
    // Request type bits
    pub const REQ_TYPE_DIR_DEVICE_TO_HOST: u8 = 1 << 7;
    pub const REQ_TYPE_TYPE_CLASS: u8 = 1 << 5;
    pub const REQ_TYPE_TO_INTERFACE: u8 = 1;

    // Standard requests
    pub const REQ_SET_CONFIGURATION: u8 = 9;
    pub const REQ_SET_INTERFACE: u8 = 11;

    // HID class requests
    pub const REQ_GET_REPORT: u8 = 1;
    pub const REQ_SET_PROTOCOL: u8 = 11;

    pub fn new(request_type: u8, request: u8, value: u16, index: u16, length: u16) -> Self
}
```

---

## StatusStageTrb::new_in()

No Data Transfer（データなし転送）でデバイスからの応答を待つ場合に使用。

```rust
pub fn new_in() -> Self {
    Self {
        _rsvdz: 0,
        option: 0,
        control: (TrbType::StatusStage as u32) << 10
            | Self::CTRL_BIT_DIR_IN
            | Self::CTRL_BIT_IOC,
    }
}
```

---

## Controller への新規メソッド

### request_set_config()
```rust
// SET_CONFIGURATION: デバイスの構成を選択
pub async fn request_set_config(&self, slot: u8, ctrl_ep_ring: &mut CommandRing, config_value: u8)
```

### request_set_interface()
```rust
// SET_INTERFACE: インターフェースの代替設定を選択
pub async fn request_set_interface(&self, slot: u8, ctrl_ep_ring: &mut CommandRing,
    interface_number: u8, alt_setting: u8)
```

### request_set_protocol()
```rust
// SET_PROTOCOL: HIDプロトコル（Boot/Report）を設定
pub async fn request_set_protocol(&self, slot: u8, ctrl_ep_ring: &mut CommandRing,
    interface_number: u8, protocol: u8)
```

### request_report_bytes()
```rust
// GET_REPORT: HID Reportを取得
// [HID] 7.2.1 Get_Report Request
async fn request_report_bytes(&self, slot: u8, ctrl_ep_ring: &mut CommandRing,
    buf: Pin<&mut [u8]>)
```

---

## HID Boot Keyboard Report フォーマット (8バイト)

```
Byte 0: Modifier Keys
  Bit 0: Left Ctrl
  Bit 1: Left Shift
  Bit 2: Left Alt
  Bit 3: Left GUI (Windows/Command)
  Bit 4: Right Ctrl
  Bit 5: Right Shift
  Bit 6: Right Alt
  Bit 7: Right GUI

Byte 1: Reserved (常に0)

Byte 2-7: Keycodes (最大6キー同時押し)
  0x00: No key
  0x04: 'A'
  0x05: 'B'
  ...
```

---

## 出力例の解説

```
[0, 0, 0, 0, 0, 0, 0, 0]  // キー押下なし
[0, 0, 4, 0, 0, 0, 0, 0]  // 'A'キー押下 (keycode 0x04)
[2, 0, 4, 0, 0, 0, 0, 0]  // Shift + 'A' (modifier=0x02, keycode=0x04)
```

---

## run() の変更点

1. Descriptorをパースして Boot Keyboard Interface (3, 1, 1) を検索
2. SET_CONFIGURATION で構成を有効化
3. SET_INTERFACE でインターフェースを選択
4. SET_PROTOCOL で Boot Protocol を設定
5. 無限ループで GET_REPORT をポーリング

---

## この段階でできること

- USBキーボードからのキー入力取得
- Boot Protocol による8バイトレポート読み取り
- Modifier Keys（Shift, Ctrl, Alt等）の検出
- 最大6キー同時押しの検出

## 次のステップ

- Interrupt転送によるレポート取得（ポーリングではなく）
- キーコードから文字への変換
- キーボードドライバの分離
