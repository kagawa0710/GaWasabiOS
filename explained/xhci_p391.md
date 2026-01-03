# p391 - PORTSC レジスタの読み取り

## 前回（p390）からの追加

p390ではイベントポーリングを実装した。今回はPORTSCレジスタを読み取ってUSBポートの状態を表示できるようになった。

---

## XhcRegisters構造体

レジスタ群を1つの構造体にまとめた：

```rust
struct XhcRegisters {
    cap_regs: Mmio<CapabilityRegisters>,
    op_regs: Mmio<OperationalRegisters>,
    rt_regs: Mmio<RuntimeRegisters>,
    portsc: PortSc,  // ← 新規追加
}
```

### なぜまとめる？

- 個別のレジスタを別々に渡すより構造がシンプル
- `Controller`構造体の変更（`cap_regs`, `op_regs`, `rt_regs` → `regs`）
- 将来のリファクタリングが容易に

---

## PORTSC (Port Status and Control) レジスタ

各USBポートの状態を示すレジスタ。

### メモリレイアウト

```
[xhci] 5.4.8: PORTSC
OperationalBase + (0x400 + 0x10 * (n - 1))
where n = Port Number (1, 2, ..., MaxPorts)

例: 8ポートの場合
OperationalBase + 0x400: Port 1 の PORTSC
OperationalBase + 0x410: Port 2 の PORTSC
OperationalBase + 0x420: Port 3 の PORTSC
...
OperationalBase + 0x470: Port 8 の PORTSC
```

### PORTSCのビットフィールド（主要なもの）

```
PORTSC (32bit)
┌──────────────────────────────────────────────────────┐
│ bit 0:   CCS (Current Connect Status) - デバイス接続中？  │
│ bit 1:   PED (Port Enabled/Disabled)                 │
│ bit 4:   PR  (Port Reset)                            │
│ bit 9:   PP  (Port Power)                            │
│ bit 10-13: Port Speed                                │
│ bit 17:  CSC (Connect Status Change) - 接続状態変化    │
└──────────────────────────────────────────────────────┘
```

---

## PortSc構造体

```rust
struct PortSc {
    entries: Vec<Rc<PortScEntry>>,
}

impl PortSc {
    fn new(bar: &BarMem64, cap_regs: &CapabilityRegisters) -> Self {
        // PORTSCのベースアドレス = OperationalBase + 0x400
        let base = unsafe {
            bar.addr().add(cap_regs.caplength()).add(0x400)
        } as *mut u32;

        let num_ports = cap_regs.num_of_ports();
        let mut entries = Vec::new();

        for port in 1..=num_ports {
            // 各ポートは 0x10 (16バイト) 間隔
            // u32ポインタなので4を掛ける
            let ptr = unsafe { base.add((port - 1) * 4) };
            entries.push(Rc::new(PortScEntry::new(ptr)));
        }
        Self { entries }
    }

    fn port_range(&self) -> Range<usize> {
        1..self.entries.len() + 1  // 1-indexed
    }

    fn get(&self, port: usize) -> Option<Rc<PortScEntry>> {
        // port=1 → index=0
        self.entries.get(port.wrapping_sub(1)).cloned()
    }
}
```

### ポート番号が1始まりの理由

xHCI仕様でポート番号は1から始まる。これはxHCIの仕様書に従った設計。

---

## PortScEntry構造体

```rust
#[repr(C)]
struct PortScEntry {
    ptr: Mutex<*mut u32>,
}

impl PortScEntry {
    fn new(ptr: *mut u32) -> Self {
        Self { ptr: Mutex::new(ptr) }
    }

    fn value(&self) -> u32 {
        let portsc = self.ptr.lock();
        unsafe { read_volatile(*portsc) }
    }
}
```

### なぜMutex<*mut u32>？

- 複数箇所からポートにアクセスする可能性
- 将来的にポート操作（リセット等）を行う際の排他制御
- `Rc<PortScEntry>`で共有されるため

---

## CapabilityRegistersへの追加

```rust
impl CapabilityRegisters {
    fn num_of_ports(&self) -> usize {
        extract_bits(self.hcsparams1.read(), 24, 8) as usize
    }
}
```

### HCSPARAMS1レジスタ

```
HCSPARAMS1 (32bit)
┌────────────────────────────────────────┐
│ bit 0-7:   MaxSlots - 最大デバイススロット数  │
│ bit 8-18:  MaxIntrs - 最大インターラプタ数   │
│ bit 24-31: MaxPorts - 最大ポート数          │
└────────────────────────────────────────┘
```

---

## Controller構造体の変更

```rust
// 変更前
struct Controller {
    cap_regs: Mmio<CapabilityRegisters>,
    op_regs: Mmio<OperationalRegisters>,
    rt_regs: Mmio<RuntimeRegisters>,
    // ...
}

// 変更後
struct Controller {
    regs: XhcRegisters,  // 統合
    device_context_base_array: Mutex<DeviceContextBaseAddressArray>,
    primary_event_ring: Mutex<EventRing>,
    command_ring: Mutex<CommandRing>,
}
```

各メソッドのアクセスも変更：
- `self.cap_regs` → `self.regs.cap_regs`
- `self.op_regs` → `self.regs.op_regs`
- `self.rt_regs` → `self.regs.rt_regs`

---

## async fn run() でのPORTSC表示

```rust
async fn run(xhc: Controller) -> Result<()> {
    // レジスタ情報を表示
    info!(
        "xhci: cap_regs.MaxSlots = {}",
        xhc.regs.cap_regs.as_ref().num_of_device_slots()
    );
    info!(
        "xhci: op_regs.USBSTS = {}",
        xhc.regs.op_regs.as_ref().usbsts()
    );
    info!(
        "xhci: rt_regs.MFINDEX = {}",
        xhc.regs.rt_regs.as_ref().mfindex()
    );

    // PORTSC値を表示
    info!("PORTSC values for port {:?}", xhc.regs.portsc.port_range());
    for port in xhc.regs.portsc.port_range() {
        if let Some(e) = xhc.regs.portsc.get(port) {
            info!("  {port:3}: {:#010X}", e.value());
        }
    }

    // イベントポーリングループ
    let xhc = Rc::new(xhc);
    // ...
}
```

---

## 期待される出力

```
[INFO] Xhci found at: /pci/bus/0x00/device/0x03/function/0x0
[INFO] xhci: page_size = 4096
[INFO] xhci: original num_scratchpad_bufs = 0
[INFO] Starting xHC...
[INFO] xHC started running!
[INFO] xhci: cap_regs.MaxSlots = 64
[INFO] xhci: op_regs.USBSTS = 1
[INFO] xhci: rt_regs.MFINDEX = 0
[INFO] PORTSC values for port 1..9
[INFO]   1: 0x00000000
[INFO]   2: 0x00000000
[INFO]   3: 0x00000000
...
```

デバイスが接続されていない場合、PORTSC値は`0x00000000`になる。

---

## この段階でできること

- USBポートの状態を読み取れる
- ポート数を正しく検出できる
- レジスタ群を統合して管理しやすくなった

## 次のステップ

- PORTSC値の解析（CCS, PED, PP等のビットフィールド）
- ポート接続検出（CSCビットの監視）
- ポートリセット処理
