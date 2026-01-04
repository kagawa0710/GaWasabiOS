# p434 - Config Descriptor取得とDescriptorIterator

## 前回（p428）からの追加

p428ではControl TransferでDevice DescriptorとString Descriptorを取得した。今回はConfig Descriptorを取得し、DescriptorIteratorでパースする。

---

## Config Descriptorとは？

USBデバイスの構成情報を格納するDescriptor。後続のInterface/Endpoint Descriptorを含む。

```
┌─────────────────────────────────────────────────────────────┐
│ Config Descriptor の構造                                     │
│                                                             │
│   ┌──────────────────┐                                      │
│   │ Config Descriptor │ 9バイト                              │
│   │  └─ total_length  │ ← 全体のサイズ                       │
│   ├──────────────────┤                                      │
│   │ Interface Desc   │ 9バイト                               │
│   ├──────────────────┤                                      │
│   │ Endpoint Desc    │ 7バイト                               │
│   ├──────────────────┤                                      │
│   │ ...              │                                      │
│   └──────────────────┘                                      │
└─────────────────────────────────────────────────────────────┘
```

---

## 新規構造体

### ConfigDescriptor (9バイト)

```rust
#[derive(Debug, Copy, Clone, Default)]
#[repr(packed)]
pub struct ConfigDescriptor {
    desc_length: u8,           // 常に9
    desc_type: u8,             // 常に2
    total_length: u16,         // 全体のサイズ（後続含む）
    num_of_interfaces: u8,     // インターフェース数
    config_value: u8,          // SET_CONFIGURATIONで使用
    config_string_index: u8,   // String Descriptorのインデックス
    attribute: u8,             // 属性（Self-Powered等）
    max_power: u8,             // 最大消費電流（2mA単位）
}
```

**重要**: `total_length` は Config Descriptor 自身 + 後続の全 Descriptor のサイズ。

### InterfaceDescriptor (9バイト)

```rust
#[derive(Debug, Copy, Clone, Default)]
#[repr(packed)]
pub struct InterfaceDescriptor {
    desc_length: u8,           // 常に9
    desc_type: u8,             // 常に4
    interface_number: u8,      // インターフェース番号
    alt_setting: u8,           // 代替設定
    num_of_endpoints: u8,      // エンドポイント数
    interface_class: u8,       // クラスコード (HID=3等)
    interface_subclass: u8,    // サブクラス
    interface_protocol: u8,    // プロトコル
    interface_string_index: u8,// String Descriptorのインデックス
}
```

---

## UsbDescriptor enum

各Descriptorを統一的に扱うための列挙型：

```rust
#[derive(Debug, Copy, Clone)]
pub enum UsbDescriptor {
    Config(ConfigDescriptor),
    Interface(InterfaceDescriptor),
    Unknown { desc_len: u8, desc_type: u8 },
}
```

未対応のDescriptor（Endpoint等）は `Unknown` として保持。

---

## DescriptorIterator

バッファをパースして各Descriptorを順に返すイテレータ：

```rust
pub struct DescriptorIterator<'a> {
    buf: &'a [u8],
    index: usize,
}

impl<'a> Iterator for DescriptorIterator<'a> {
    type Item = UsbDescriptor;
    fn next(&mut self) -> Option<Self::Item> {
        // buf[index] = desc_len
        // buf[index+1] = desc_type
        // desc_typeに応じてパース
        // index += desc_len で次へ
    }
}
```

**パース手順:**
1. `buf[0]` (desc_len) と `buf[1]` (desc_type) を読む
2. desc_type に応じて適切な構造体にコピー
3. index を desc_len 分進める
4. 繰り返し

---

## Sliceable trait

バイト列から構造体をコピーするためのトレイト（slice.rs）：

```rust
pub unsafe trait Sliceable: Sized + Copy + Clone {
    fn copy_from_slice(data: &[u8]) -> Result<Self> {
        if size_of::<Self>() > data.len() {
            Err("data is too short")
        } else {
            Ok(unsafe { *(data.as_ptr() as *const Self) })
        }
    }
}
```

---

## request_config_descriptor_and_rest()

2段階でConfig Descriptorを取得：

```rust
async fn request_config_descriptor_and_rest(...) -> Result<Vec<UsbDescriptor>> {
    // 1. まずConfigDescriptor（9バイト）だけ取得
    let config_buf = vec![0u8; size_of::<ConfigDescriptor>()];
    xhc.request_descriptor(..., &mut config_buf).await?;

    // 2. total_lengthを取得
    let config_descriptor = ConfigDescriptor::copy_from_slice(&config_buf)?;

    // 3. total_lengthサイズのバッファで再取得
    let buf = vec![0u8; config_descriptor.total_length()];
    xhc.request_descriptor(..., &mut buf).await?;

    // 4. DescriptorIteratorでパース
    let iter = DescriptorIterator::new(&buf);
    let descriptors: Vec<UsbDescriptor> = iter.collect();
    Ok(descriptors)
}
```

**なぜ2回取得？**
- 1回目: total_length を知るため（最初の9バイトに含まれる）
- 2回目: 全体を取得するため

---

## データフロー

```
┌──────────────────────────────────────────────────────────────┐
│ run()                                                         │
│   1. request_device_descriptor() → Device Descriptor          │
│   2. request_string_descriptor_zero() → 言語ID                │
│   3. request_string_descriptor() × 3 → 文字列                 │
│   4. request_config_descriptor_and_rest() ← NEW!              │
│      └─→ DescriptorIterator                                   │
│          └─→ Vec<UsbDescriptor>                               │
└──────────────────────────────────────────────────────────────┘
```

---

## 期待される出力

```
[INFO] xhci: v/p/s = Some("QEMU")/Some("QEMU USB Keyboard")/Some("...")
[INFO] xhci: [Config(ConfigDescriptor { ... }), Interface(InterfaceDescriptor { ... }), Unknown { desc_len: 9, desc_type: 33 }, Unknown { desc_len: 7, desc_type: 5 }]
```

- `desc_type: 33 (0x21)` = HID Descriptor
- `desc_type: 5` = Endpoint Descriptor

---

## この段階でできること

- Config Descriptor の取得
- Interface Descriptor のパース
- 未知の Descriptor のスキップ
- デバイス構成の全体像把握

## 次のステップ

- Endpoint Descriptor の実装
- HID Descriptor の実装
- SET_CONFIGURATION コマンド
- デバイスドライバの実装
