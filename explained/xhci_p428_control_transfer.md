# p428 - Control Transfer と String Descriptor 取得

## 前回（p414）からの追加

p414ではAddress Device Commandでデバイスにアドレスを割り当てた。今回はControl Transferを実装し、Device DescriptorとString Descriptorを取得する。

---

## Control Transferとは？

USBの基本的なデータ転送方式の一つ。3つのステージで構成される：

```
┌─────────────────────────────────────────────────────────────┐
│ Control Transfer の構造                                      │
│                                                             │
│   Setup Stage  →  Data Stage  →  Status Stage              │
│   (8バイト固定)    (可変長)       (0バイト)                   │
│                                                             │
│   ・リクエスト内容   ・実データ     ・完了確認                 │
│   ・方向/長さ指定    ・IN/OUT      ・成功/失敗                │
└─────────────────────────────────────────────────────────────┘
```

---

## 新規構造体

### SetupStageTrb - Setup Stage TRB

```rust
#[repr(C, align(16))]
pub struct SetupStageTrb {
    request_type: u8,    // bmRequestType
    request: u8,         // bRequest (GET_DESCRIPTOR = 6)
    value: u16,          // wValue (Desc Type << 8 | Index)
    index: u16,          // wIndex (Language ID for String)
    length: u16,         // wLength (転送バイト数)
    option: u32,         // TRB Transfer Length (常に8)
    control: u32,        // TRB Type, IDT, TRT
}
```

**重要**: `option` は常に8。Setup Stage TRBのデータは8バイト固定。

### DataStageTrb - Data Stage TRB

```rust
#[repr(C, align(16))]
pub struct DataStageTrb {
    data_buf_ptr: u64,   // データバッファのアドレス
    option: u32,         // TRB Transfer Length
    control: u32,        // TRB Type, DIR, IOC
}
```

- **DIR** (bit 16): 1 = IN (device→host), 0 = OUT
- **IOC** (bit 5): Interrupt On Completion

### StatusStageTrb - Status Stage TRB

```rust
#[repr(C, align(16))]
pub struct StatusStageTrb {
    _rsvdz: u64,         // 予約（0）
    option: u32,         // 0
    control: u32,        // TRB Type, IOC
}
```

---

## transfer_result_ok() - Transfer Event の検証

```rust
pub fn transfer_result_ok(&self) -> Result<()> {
    if self.completion_code() != 1 && self.completion_code() != 13 {
        // 1 = Success
        // 13 = Short Packet (正常: 要求より少ないデータ)
        Err("CompletionCode was not Success")
    } else {
        Ok(())
    }
}
```

**Short Packet (13)** は、要求サイズより少ないデータが返された場合に発生。可変長データ（String Descriptorなど）では正常な動作。

---

## request_descriptor() - Control Transfer の実装

```rust
async fn request_descriptor(
    &self,
    slot: u8,
    ctrl_ep_ring: &mut CommandRing,
    desc_type: UsbDescriptorType,
    desc_index: u8,
    lang_id: u16,
    buf: &mut Pin<Box<[u8]>>,
) -> Result<()> {
    // 1. Setup Stage TRB
    let setup_trb = SetupStageTrb::new_get_descriptor(...);
    ctrl_ep_ring.push(setup_trb.into())?;

    // 2. Data Stage TRB
    let data_trb = DataStageTrb::new_in(buf.as_mut());
    let data_ptr = ctrl_ep_ring.push(data_trb.into())?;

    // 3. Status Stage TRB
    let status_trb = StatusStageTrb::new_out();
    let status_ptr = ctrl_ep_ring.push(status_trb.into())?;

    // 4. Doorbell を鳴らす前に Future を作成
    let data_future = EventFuture::new_for_trb(&self.primary_event_ring, data_ptr);
    let status_future = EventFuture::new_for_trb(&self.primary_event_ring, status_ptr);

    // 5. Doorbell を鳴らして転送開始
    self.notify_ep(slot, 1); // EP0 = DCI 1

    // 6. 両方の完了を待機
    data_future.await?.transfer_result_ok()?;
    status_future.await?.transfer_result_ok()?;

    Ok(())
}
```

---

## notify_ep() - エンドポイントへの通知

```rust
fn notify_ep(&self, slot: u8, dci: u8) {
    // Doorbell[slot].notify(dci, 0)
    // slot = デバイススロット番号
    // dci = Device Context Index (EP0 = 1)
    self.regs.doorbell_regs[slot as usize].notify(dci, 0);
}
```

Command Ring は `doorbell_regs[0]` だが、Transfer Ring は `doorbell_regs[slot]` を使う。

---

## USB Descriptor 構造

### Device Descriptor (18バイト)

```rust
pub struct UsbDeviceDescriptor {
    pub desc_length: u8,       // 常に18
    pub desc_type: u8,         // 常に1
    pub version: u16,          // USB版 (0x0200 = USB 2.0)
    pub device_class: u8,      // クラスコード
    pub device_subclass: u8,
    pub device_protocol: u8,
    pub max_packet_size: u8,   // EP0の最大パケットサイズ
    pub vendor_id: u16,        // ベンダーID
    pub product_id: u16,       // プロダクトID
    pub device_version: u16,
    pub manufacturer_idx: u8,  // String Descriptorのインデックス
    pub product_idx: u8,
    pub serial_idx: u8,
    pub num_of_config: u8,
}
```

### String Descriptor 0 (言語ID一覧)

```
┌─────────────────────────────────────────┐
│ byte 0: bLength (全体の長さ)            │
│ byte 1: bDescriptorType (= 3)           │
│ byte 2-3: wLANGID[0] (例: 0x0409 = 英語)│
│ byte 4-5: wLANGID[1] (省略可能)         │
│ ...                                     │
└─────────────────────────────────────────┘
```

### String Descriptor N (文字列)

```
┌─────────────────────────────────────────┐
│ byte 0: bLength                         │
│ byte 1: bDescriptorType (= 3)           │
│ byte 2-: Unicode文字列 (UTF-16LE)       │
│          ※NULLターミネートなし          │
└─────────────────────────────────────────┘
```

---

## データフロー全体像

```
┌──────────────────────────────────────────────────────────────┐
│ run()                                                         │
│   1. address_device() → ctrl_ep_ring 取得                    │
│   2. request_device_descriptor() → Device Descriptor取得     │
│   3. request_string_descriptor_zero() → 言語ID取得           │
│   4. request_string_descriptor() × 3 → 文字列取得            │
│      - manufacturer_idx → ベンダー名                         │
│      - product_idx → 製品名                                  │
│      - serial_idx → シリアル番号                             │
└──────────────────────────────────────────────────────────────┘
                           ↓
┌──────────────────────────────────────────────────────────────┐
│ request_descriptor() - Control Transfer                       │
│                                                               │
│   Setup TRB → Data TRB → Status TRB                          │
│        ↓           ↓           ↓                             │
│   (リクエスト)  (データ受信)  (完了確認)                      │
│        ↓           ↓           ↓                             │
│   Transfer   Transfer    Transfer                            │
│   Event待機   Event待機   Event待機                          │
└──────────────────────────────────────────────────────────────┘
```

---

## 期待される出力

```
[INFO] xhci: device detected: vid:pid = 0x0627:0x0001
[INFO] xhci: v/p/s = Some("QEMU")/Some("QEMU USB Keyboard")/Some("68284-0000:00:03.0-1")
```

---

## この段階でできること

- Control Transferによるデータ取得
- Device Descriptorの読み取り
- String Descriptorによる文字列取得
- USBデバイスの識別（ベンダー名、製品名、シリアル番号）

## 次のステップ

- Configuration Descriptor取得
- デバイスクラスの判定
- 各種ドライバの実装（HID, Mass Storageなど）
