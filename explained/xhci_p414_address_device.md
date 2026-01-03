# p414 - Address Device Command（デバイスアドレス割り当て）

## 前回（p404）からの追加

p404ではEnable Slot Commandでスロットを取得した。今回はAddress Device Commandを送信してUSBデバイスにアドレスを割り当てる。

---

## Address Device Commandとは？

USBデバイスと通信するには、デバイスに一意のアドレスを割り当てる必要がある。

```
┌─────────────────────────────────────────────────────────────┐
│ Enable Slot Command                                          │
│   → スロット番号取得（例: slot 2）                           │
│   → このスロットでデバイスを管理する準備                     │
└─────────────────────────────────────────────────────────────┘
                           ↓
┌─────────────────────────────────────────────────────────────┐
│ Address Device Command                                       │
│   → デバイスにUSBアドレスを割り当て                          │
│   → デフォルトコントロールエンドポイントを有効化             │
│   → これ以降、デバイスと通信可能に                           │
└─────────────────────────────────────────────────────────────┘
```

---

## Input Context の構造

Address Device Commandには**Input Context**を渡す必要がある。

```
┌─────────────────────────────────────────────────────────────┐
│ InputContext (4096バイト境界でアラインメント)                │
├─────────────────────────────────────────────────────────────┤
│ InputControlContext (32バイト)                               │
│   - add_context_bitmap: どのコンテキストを追加するか         │
│   - drop_context_bitmap: どのコンテキストを削除するか        │
├─────────────────────────────────────────────────────────────┤
│ DeviceContext                                                │
│   ├── SlotContext (32バイト)                                 │
│   │     - ポート番号、スピード、コンテキストエントリ数       │
│   ├── EP0 Context (32バイト) - デフォルトコントロールEP      │
│   ├── EP1 Context (32バイト)                                 │
│   └── ... (最大31エンドポイント)                             │
└─────────────────────────────────────────────────────────────┘
```

---

## InputControlContext

どのコンテキストを設定するかを指定する。

```rust
#[repr(C, align(32))]
pub struct InputControlContext {
    drop_context_bitmap: u32,  // 削除するコンテキスト
    add_context_bitmap: u32,   // 追加するコンテキスト
    data: [u32; 6],
    _pinned: PhantomPinned,
}

impl InputControlContext {
    pub fn add_context(&mut self, dci: usize) -> Result<()> {
        if dci < 32 {
            self.add_context_bitmap |= 1 << dci;
            Ok(())
        } else {
            Err("Input context index out of range")
        }
    }
}
```

### ビットマップの意味
```
add_context_bitmap:
bit 0: Slot Context を追加
bit 1: EP0 (Default Control Endpoint) を追加
bit 2: EP1
...
```

Address Device Commandでは bit 0 と bit 1 を立てる（Slot + EP0）。

---

## DeviceContext のSlot Context設定

```rust
impl DeviceContext {
    // USBスピードを設定（bit 20-23）
    fn set_port_speed(&mut self, mode: UsbMode) -> Result<()> {
        self.slot_ctx[0] &= !(0xF << 20);
        self.slot_ctx[0] |= mode.psi() << 20;
        Ok(())
    }

    // 最後の有効なDCI（Device Context Index）を設定
    fn set_last_valid_dci(&mut self, dci: usize) -> Result<()> {
        self.slot_ctx[0] &= !(0b11111 << 27);
        self.slot_ctx[0] |= (dci as u32) << 27;
        Ok(())
    }

    // ルートハブポート番号を設定
    fn set_root_hub_port_number(&mut self, port: usize) -> Result<()> {
        self.slot_ctx[1] &= !(0xFF << 16);
        self.slot_ctx[1] |= (port as u32) << 16;
        Ok(())
    }
}
```

### Slot Context のレイアウト
```
slot_ctx[0]:
┌────────────────────────────────────────────────────┐
│ bit 31-27 │ bit 26-24 │ bit 23-20 │ bit 19-0      │
│ ContextEnt│ (reserved)│ Speed     │ Route String  │
└────────────────────────────────────────────────────┘

slot_ctx[1]:
┌────────────────────────────────────────────────────┐
│ bit 23-16      │ bit 15-0                          │
│ Root Hub Port  │ Num Ports / ...                   │
└────────────────────────────────────────────────────┘
```

---

## EndpointContext（コントロールエンドポイント用）

```rust
impl EndpointContext {
    fn new_control_endpoint(
        max_packet_size: u16,
        tr_dequeue_ptr: u64,
    ) -> Result<Self> {
        let mut ep = Self::new();
        ep.set_ep_type(EndpointType::Control)?;
        ep.set_dequeue_cycle_state(true)?;
        ep.set_error_count(3)?;
        ep.set_max_packet_size(max_packet_size);
        ep.set_ring_dequeue_pointer(tr_dequeue_ptr)?;
        ep.average_trb_length = 8;  // コントロールEPは8固定
        Ok(ep)
    }
}
```

### 設定項目
| 項目 | 値 | 説明 |
|-----|-----|------|
| EP Type | Control (4) | 双方向コントロール転送 |
| DCS | true | Dequeue Cycle State |
| Error Count | 3 | エラー再試行回数 |
| Max Packet Size | スピード依存 | 下記参照 |
| Average TRB Length | 8 | コントロールEPは8バイト |

---

## UsbMode と Max Packet Size

```rust
pub enum UsbMode {
    Unknown(u32),
    FullSpeed,   // 12 Mbps
    LowSpeed,    // 1.5 Mbps
    HighSpeed,   // 480 Mbps
    SuperSpeed,  // 5 Gbps
}

impl PortScEntry {
    pub fn max_packet_size(&self) -> Result<u16> {
        match self.port_speed() {
            UsbMode::FullSpeed | UsbMode::LowSpeed => Ok(8),
            UsbMode::HighSpeed => Ok(64),
            UsbMode::SuperSpeed => Ok(512),
            _ => Err("Unknown Protocol Speed ID"),
        }
    }

    pub fn port_speed(&self) -> UsbMode {
        // PORTSC bit 10-13: Protocol Speed ID
        match extract_bits(self.value(), 10, 4) {
            1 => UsbMode::FullSpeed,
            2 => UsbMode::LowSpeed,
            3 => UsbMode::HighSpeed,
            4 => UsbMode::SuperSpeed,
            v => UsbMode::Unknown(v),
        }
    }
}
```

---

## address_device() の実装

```rust
async fn address_device(
    xhc: &Rc<Controller>,
    port: usize,
    slot: u8,
) -> Result<()> {
    // 1. InputControlContext設定
    let mut input_ctrl_ctx = InputControlContext::default();
    input_ctrl_ctx.add_context(0)?;  // Slot Context
    input_ctrl_ctx.add_context(1)?;  // EP0 Context

    // 2. InputContext作成
    let mut input_context = InputContext::default();
    input_context.set_input_ctrl_ctx(input_ctrl_ctx);

    // 3. Slot Context設定
    input_context.set_root_hub_port_number(port)?;
    input_context.set_last_valid_dci(1)?;  // EP0のみ

    // 4. EP0 Context設定
    let portsc = xhc.regs.portsc.get(port).ok_or("PORTSC was invalid")?;
    input_context.set_port_speed(portsc.port_speed())?;

    let ctrl_ep_ring = CommandRing::default();
    input_context.set_ep_ctx(
        1,
        EndpointContext::new_control_endpoint(
            portsc.max_packet_size()?,
            ctrl_ep_ring.ring_phys_addr(),
        )?,
    );

    // 5. Address Device Command送信
    let cmd = GenericTrbEntry::cmd_address_device(&input_context, slot);
    xhc.send_command(cmd).await?.cmd_result_ok()?;

    Ok(())
}
```

---

## Output Context

xHCはAddress Device Commandの結果を**Output Context**に書き込む。

```rust
fn set_output_context_for_slot(
    &self,
    slot: u8,
    output_context: Pin<Box<OutputContext>>,
) {
    self.device_context_base_array
        .lock()
        .set_output_context(slot, output_context);
}
```

DCBAAのインデックスは `slot - 1` であることに注意。

---

## データフロー全体像

```
┌──────────────────────────────────────────────────────────────┐
│ init_port()                                                   │
│   1. reset_port()                                             │
│   2. Enable Slot Command → slot取得                          │
│   3. set_output_context_for_slot()                           │
└──────────────────────────────────────────────────────────────┘
                           ↓
┌──────────────────────────────────────────────────────────────┐
│ address_device()                                              │
│   1. InputControlContext設定 (add Slot + EP0)                │
│   2. InputContext作成                                         │
│   3. Slot Context設定 (port, speed, dci)                     │
│   4. EP0 Context設定 (Control EP, max_packet_size)           │
│   5. Address Device Command送信                               │
└──────────────────────────────────────────────────────────────┘
                           ↓
┌──────────────────────────────────────────────────────────────┐
│ xHC (ハードウェア)                                            │
│   - Input Context読み取り                                     │
│   - デバイスにUSBアドレス割り当て                             │
│   - Output Contextに結果書き込み                              │
│   - Command Completion Event発行                              │
└──────────────────────────────────────────────────────────────┘
                           ↓
              デバイスと通信可能に！
```

---

## 期待される出力

```
[INFO] xhci: port 5 is connected
[INFO] xhci: resetting port 5
[INFO] xhci: port 5 has been reset
[INFO] xhci: port 5 is enabled
[INFO] slot 1 is assigned for port 5
[INFO] AddressDeviceCommand succeeded
```

---

## この段階でできること

- USBデバイスへのアドレス割り当て
- デフォルトコントロールエンドポイントの有効化
- デバイスとの基本的な通信準備完了

## 次のステップ

- Get Descriptor（デバイス情報取得）
- Control Transfer（セットアップ→データ→ステータス）
- デバイスドライバの初期化
