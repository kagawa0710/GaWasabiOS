# p437 - EndpointDescriptor実装

## 前回（p434）からの追加

p434ではConfig DescriptorとDescriptorIteratorを実装した。今回はEndpoint Descriptorを追加し、USBデバイスの完全な構成情報を取得できるようにする。

---

## Endpoint Descriptorとは？

エンドポイント（データ転送の終点）の属性を定義するDescriptor。

```
┌─────────────────────────────────────────────────────────────┐
│ Endpoint Descriptor (7バイト)                                │
│                                                             │
│  endpoint_address: 転送方向とエンドポイント番号              │
│  attributes: 転送タイプ（Control/Bulk/Interrupt/Isochronous）│
│  max_packet_size: 最大パケットサイズ                         │
│  interval: ポーリング間隔                                    │
└─────────────────────────────────────────────────────────────┘
```

---

## 新規構造体: EndpointDescriptor (7バイト)

```rust
#[derive(Debug, Copy, Clone, Default)]
#[allow(unused)]
#[repr(packed)]
pub struct EndpointDescriptor {
    pub desc_length: u8,      // 常に7
    pub desc_type: u8,        // 常に5

    // endpoint_address:
    //  - bit[0..=3]: endpoint number
    //  - bit[7]: direction (0: OUT, 1: IN)
    pub endpoint_address: u8,

    // attributes:
    //  - bit[0..=1]: transfer type
    //    (0: Control, 1: Isochronous, 2: Bulk, 3: Interrupt)
    pub attributes: u8,

    pub max_packet_size: u16,

    // interval:
    // [xhci] Table 6-12
    // interval_ms = interval (For FS/LS Interrupt)
    // interval_ms = 2^(interval-1) (For FS Isoch)
    // interval_ms = 2^(interval-1) (For SSP/SS/HS)
    pub interval: u8,
}
const _: () = assert!(size_of::<EndpointDescriptor>() == 7);
```

---

## UsbDescriptor enumの更新

```rust
#[derive(Debug, Copy, Clone)]
pub enum UsbDescriptor {
    Config(ConfigDescriptor),
    Endpoint(EndpointDescriptor),  // 追加
    Interface(InterfaceDescriptor),
    Unknown { desc_len: u8, desc_type: u8 },
}
```

---

## DescriptorIterator::next()の更新

```rust
let desc = match desc_type {
    e if e == UsbDescriptorType::Config as u8 => {
        UsbDescriptor::Config(ConfigDescriptor::copy_from_slice(buf).ok()?)
    }
    e if e == UsbDescriptorType::Interface as u8 => {
        UsbDescriptor::Interface(InterfaceDescriptor::copy_from_slice(buf).ok()?)
    }
    e if e == UsbDescriptorType::Endpoint as u8 => {
        UsbDescriptor::Endpoint(EndpointDescriptor::copy_from_slice(buf).ok()?)
    }
    _ => UsbDescriptor::Unknown { desc_len, desc_type },
};
```

---

## 期待される出力

```
[INFO] xhci: [
  Config(ConfigDescriptor { num_of_interfaces: 1, config_value: 1, ... }),
  Interface(InterfaceDescriptor {
    interface_number: 0,
    num_of_endpoints: 1,
    interface_class: 3,      // HID
    interface_subclass: 1,   // Boot Interface
    interface_protocol: 1,   // Keyboard
    ...
  }),
  Unknown { desc_len: 9, desc_type: 33 },  // HID Descriptor (0x21)
  Endpoint(EndpointDescriptor {
    endpoint_address: 129,   // 0x81 = EP1 IN
    attributes: 3,           // Interrupt
    max_packet_size: 8,
    interval: 7
  })
]
```

---

## フィールド解説

### endpoint_address (例: 129 = 0x81)
```
bit 7    6    5    4    3    2    1    0
  │                         └────────────┘
  │                          endpoint number (1)
  └── direction: 1 = IN (device→host)
```
- 0x81 = bit7=1(IN), endpoint=1 → EP1 IN

### attributes (例: 3)
```
bit 1    0
  └──────┘
   transfer type
   0: Control
   1: Isochronous
   2: Bulk
   3: Interrupt  ← キーボードはこれ
```

### interval (例: 7)
- ポーリング間隔を指定
- FS/LS Interrupt: そのまま ms
- HS/SS: 2^(interval-1) * 125μs

---

## HIDデバイスの判別（InterfaceDescriptor）

| フィールド | 値 | 意味 |
|-----------|-----|------|
| interface_class | 3 | HID (Human Interface Device) |
| interface_subclass | 1 | Boot Interface |
| interface_protocol | 1 | Keyboard (2ならMouse) |

---

## Unknown { desc_type: 33 }

- desc_type: 33 (0x21) = HID Descriptor
- まだ未実装なのでUnknownとして処理
- HID Descriptorには Report Descriptorのサイズなどが含まれる

---

## この段階でできること

- Endpoint Descriptor のパース
- USBデバイスの完全な構成情報の取得
- HIDデバイス（キーボード/マウス）の判別
- Interrupt転送のエンドポイント情報取得

## 次のステップ

- HID Descriptor の実装
- SET_CONFIGURATION コマンド
- Configure Endpoint コマンド
- Interrupt転送によるキー入力取得
