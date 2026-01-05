# p474 - USBモジュール分離とTabletドライバ追加

## 前回（p450）からの追加

p450ではKeyEventモジュールを実装した。今回はコードをリファクタリングし、USBモジュールを分離してTabletドライバを追加する。

---

## 変更の概要

```
xhci.rs (monolithic)
    ↓ 分離
├── usb.rs      - USB Descriptor構造体 + ヘルパー関数
├── keyboard.rs - KeyEvent + start_usb_keyboard()
├── tablet.rs   - USB Tablet検出（新規）
└── xhci.rs     - xHCIコントローラのみ
```

---

## 新規ファイル: src/usb.rs

### USB Descriptor構造体
```rust
pub enum UsbDescriptorType { Device = 1, Config = 2, ... }
pub enum UsbDescriptor { Config(...), Interface(...), Endpoint(...), Unknown {...} }
pub struct UsbDeviceDescriptor { ... }   // 18バイト
pub struct ConfigDescriptor { ... }       // 9バイト
pub struct InterfaceDescriptor { ... }    // 9バイト
pub struct EndpointDescriptor { ... }     // 7バイト
pub struct DescriptorIterator<'a> { ... }
pub enum UsbHidProtocol { BootProtocol = 0 }
```

### pick_interface_with_triple() - インターフェース選択の汎用関数
```rust
pub fn pick_interface_with_triple(
    descriptors: &[UsbDescriptor],
    triple: (u8, u8, u8),
) -> Option<(ConfigDescriptor, InterfaceDescriptor, Vec<EndpointDescriptor>)>
```

USB Descriptorリストから指定したトリプル（class, subclass, protocol）に
マッチするインターフェースを検索し、対応するConfig/Interface/Endpointを返す。

### USB HID トリプル一覧
| トリプル | 意味 |
|---------|------|
| (3, 1, 1) | HID Boot Keyboard |
| (3, 1, 2) | HID Boot Mouse |
| (3, 0, 0) | HID Generic (Tablet等) |

### USB Request関数（xhci.rsから移動）
```rust
pub async fn request_device_descriptor(...) -> Result<UsbDeviceDescriptor>
pub async fn request_string_descriptor(...) -> Result<String>
pub async fn request_string_descriptor_zero(...) -> Result<Vec<u8>>
pub async fn request_config_descriptor_and_rest(...) -> Result<Vec<UsbDescriptor>>
pub async fn request_hid_report(...) -> Result<Vec<u8>>
```

---

## 新規ファイル: src/tablet.rs

### start_usb_tablet() - USB Tablet検出
```rust
pub async fn start_usb_tablet(
    _xhc: &Rc<Controller>,
    _slot: u8,
    _ctrl_ep_ring: &mut CommandRing,
    device_descriptor: &UsbDeviceDescriptor,
    descriptors: &[UsbDescriptor],
) -> Result<()>
```

### USB Tablet判別条件（QEMUの場合）
| フィールド | 値 |
|-----------|-----|
| device_class | 0 |
| device_subclass | 0 |
| device_protocol | 0 |
| vendor_id | 0x0627 (QEMU) |
| product_id | 0x0001 |
| interface triple | (3, 0, 0) |

---

## src/keyboard.rs の変更

### start_usb_keyboard() の簡略化
```rust
// Before: 手動でDescriptorをループ検索
let mut last_config: Option<ConfigDescriptor> = None;
let mut boot_keyboard_interface: Option<InterfaceDescriptor> = None;
for d in descriptors { ... }

// After: pick_interface_with_triple()を使用
let (config_desc, interface_desc, _) =
    pick_interface_with_triple(descriptors, (3, 1, 1))
        .ok_or("No USB KBD Boot interface found")?;
```

---

## src/xhci.rs の変更

### 削除されたもの
- USB Descriptor構造体（usb.rsへ移動）
- request_*関数（usb.rsへ移動）
- キーボード処理ループ（keyboard.rsへ移動）

### 追加されたもの
```rust
use crate::keyboard::start_usb_keyboard;
use crate::tablet::start_usb_tablet;
use crate::usb;
```

### 公開された構造体
```rust
pub struct Controller { ... }
pub struct CommandRing { ... }
```

### ドライバ選択ロジック（run()内）
```rust
// キーボードドライバを試行
if start_usb_keyboard(&xhc, slot, &mut ctrl_ep_ring, &descriptors)
    .await.is_ok()
{
    return Ok(());
}

// タブレットドライバを試行
if start_usb_tablet(&xhc, slot, &mut ctrl_ep_ring,
    &device_descriptor, &descriptors)
    .await.is_ok()
{
    return Ok(());
}

info!("xhci: No available drivers...");
```

---

## その他の修正

### .cargo/config.toml のtypo修正
```toml
# Before (typo)
rustflag = ["-Cforce-unwind-tables", ...]

# After (正しい)
rustflags = ["-Cforce-unwind-tables", ...]
```

---

## 期待される出力

### USBキーボード接続時
```
[INFO] usb_keyboard: key down: 4 = Char('a')
[INFO] usb_keyboard: key up  : 4 = Char('a')
```

### USB Tablet（QEMUマウス）接続時
```
[INFO] src/tablet.rs:28 : USB tablet found
```

---

## ファイル構成

```
src/
├── lib.rs       - pub mod tablet; pub mod usb; 追加
├── usb.rs       - USB Descriptor + ヘルパー関数（新規）
├── tablet.rs    - USB Tabletドライバ（新規）
├── keyboard.rs  - KeyEvent + start_usb_keyboard()
└── xhci.rs      - xHCIコントローラ（簡略化）
```

---

## この段階でできること

- USBモジュールの分離による保守性向上
- pick_interface_with_triple()による汎用的なインターフェース検索
- USB Tablet（QEMUマウス）の検出
- 複数ドライバの順次試行パターン

## 次のステップ

- USB Tabletからの入力レポート取得
- マウスカーソル座標の処理
- Interrupt転送への移行
