# p360 - xHCI コントローラ初期化

## xHCIとは？

**xHCI (eXtensible Host Controller Interface)** = USB 3.0のホストコントローラ仕様

```
┌─────────────┐      ┌─────────────┐      ┌─────────────┐
│  USBデバイス │◄────►│   xHCI      │◄────►│    CPU      │
│(キーボード等)│      │ コントローラ │      │  (OS)       │
└─────────────┘      └─────────────┘      └─────────────┘
                      ↑
                    PCIeバス経由で接続
```

OSはxHCIコントローラを通じてUSBデバイスと通信する。

---

## レジスタ群（ハードウェアが持つ設定値）

xHCはPCIデバイスとして認識され、BAR0から各レジスタ領域にアクセスする。

```
BAR0 (PCIから取得したアドレス)
│
├─► CapabilityRegisters (0x00～)  ← xHCの能力を示す（読み取り専用）
│     - MaxSlots: 最大デバイス接続数（例: 64）
│     - MaxPorts: USBポート数
│     - caplength: このレジスタ領域のサイズ
│     - rtsoff: RuntimeRegistersへのオフセット
│
├─► OperationalRegisters (caplength～)  ← xHCの操作用
│     - USBCMD: コマンド（開始/停止/リセット）
│     - USBSTS: ステータス（動作中か停止中か）
│     - DCBAAP: DCBAAのアドレスを設定
│
└─► RuntimeRegisters (rtsoff～)  ← 割り込み・イベント管理
      - MFINDEX: マイクロフレームインデックス
      - IRS[]: Interrupter Register Sets（イベント通知用）
```

### コードとの対応

```rust
// Capability Registers - xHCの能力情報
#[repr(C)]
struct CapabilityRegisters {
    caplength: Volatile<u8>,      // OperationalRegistersへのオフセット
    hcsparams1: Volatile<u32>,    // MaxSlots, MaxPorts等
    hcsparams2: Volatile<u32>,    // Scratchpadバッファ数等
    rtsoff: Volatile<u32>,        // RuntimeRegistersへのオフセット
    // ...
}

// Operational Registers - xHCの操作
#[repr(C)]
struct OperationalRegisters {
    usbcmd: Volatile<u32>,        // Run/Stop, Reset等
    usbsts: Volatile<u32>,        // HCHalted等のステータス
    dcbaap: Volatile<*mut ...>,   // DCBAAのアドレス
    // ...
}
```

---

## Scratchpad Buffers

xHCが**内部処理用に使う作業メモリ**。OSが確保してxHCに渡す。

```
  table (ポインタ配列)          実際のバッファ
┌──────────────┐           ┌─────────────┐
│ ptr[0] ──────┼──────────►│ 4KB buffer  │
├──────────────┤           ├─────────────┤
│ ptr[1] ──────┼──────────►│ 4KB buffer  │
├──────────────┤           └─────────────┘
│ ...          │
└──────────────┘
```

### コードとの対応

```rust
struct ScratchpadBuffers {
    table: Pin<Box<[*const u8]>>,  // ポインタのテーブル
    _bufs: Vec<Pin<Box<[u8]>>>,    // 実際のバッファ（解放されないよう保持）
}

impl ScratchpadBuffers {
    fn alloc(cap_regs, op_regs) -> Result<Self> {
        let page_size = op_regs.page_size()?;           // xHCが要求するページサイズ
        let num = cap_regs.num_scratchpad_bufs();       // 必要なバッファ数

        // num個のpage_sizeバッファを確保
        for _ in 0..num {
            let buf = ALLOCATOR.alloc_with_options(...);
            // ...
        }
    }
}
```

---

## Device Context Base Address Array (DCBAA)

USBデバイスごとの状態を管理するテーブル。

```
DCBAA (Device Context Base Address Array)
┌────────────────────────────────┐
│ [0] scratchpad_table_ptr ──────┼──► Scratchpadテーブル
├────────────────────────────────┤
│ [1] context[0] ────────────────┼──► デバイス1 (Slot 1) の状態
├────────────────────────────────┤
│ [2] context[1] ────────────────┼──► デバイス2 (Slot 2) の状態
├────────────────────────────────┤
│ ...                            │    (最大255デバイス)
└────────────────────────────────┘
```

### コードとの対応

```rust
// 生のメモリレイアウト（xHCが直接参照）
#[repr(C, align(64))]
struct RawDeviceContextBaseAddressArray {
    scratchpad_table_ptr: *const *const u8,  // Slot 0 = Scratchpad
    context: [u64; 255],                      // Slot 1-255 = デバイス
}

// 管理用ラッパー
struct DeviceContextBaseAddressArray {
    _inner: Pin<Box<RawDeviceContextBaseAddressArray>>,
    _context: [Option<Pin<Box<OutputContext>>>; 255],
    _scratchpad_buffers: ScratchpadBuffers,  // 解放されないよう保持
}
```

---

## デバイスコンテキスト構造

各USBデバイスの状態を表す。

```
OutputContext (4KB aligned)
├── DeviceContext (0x400 bytes)
│   ├── SlotContext (32 bytes)      ← デバイス全体の状態
│   └── EndpointContext[31] (各32 bytes)  ← 各エンドポイントの状態
```

### コードとの対応

```rust
#[repr(C, align(32))]
struct EndpointContext {
    data: [u32; 2],
    tr_dequeue_ptr: Volatile<u64>,  // Transfer Ringのアドレス
    // ...
}

#[repr(C, align(32))]
struct DeviceContext {
    slot_ctx: [u32; 8],                    // Slot Context
    ep_ctx: [EndpointContext; 2 * 15 + 1], // Endpoint Contexts
}

#[repr(C, align(4096))]
struct OutputContext {
    device_ctx: DeviceContext,
}
```

---

## 初期化の流れ

```
attach()
   │
   ├─► PCIからBAR0取得
   │     └─► pci.try_bar0_mem64(bdf)
   │
   ├─► setup_xhc_registers()
   │     └─► 3つのレジスタ領域をMmioでマッピング
   │
   ├─► ScratchpadBuffers::alloc()
   │     └─► xHC用の作業メモリを確保
   │
   ├─► DeviceContextBaseAddressArray::new()
   │     └─► Scratchpadポインタを設定
   │
   ├─► Controller::new()
   │     └─► 全部まとめて保持
   │
   └─► spawn_global(run())
          └─► 非同期タスクでレジスタ値を表示
```

---

## なぜPin<Box<...>>を使う？

xHCは**物理メモリアドレス**でデータにアクセスする。
Rustがメモリを移動させると、xHCが古いアドレスを参照してクラッシュ！

```rust
Pin<Box<T>>  // 「このデータは絶対に移動しない」という約束
```

---

## この段階でできること

- xHCを認識してレジスタ値を読む
- Scratchpad Buffers を確保
- DCBAA を作成

## まだできないこと（次のステップ）

- xHCのリセット・起動
- Command Ring / Event Ring の設定
- USBデバイスの検出・通信

---

## 参考

- xHCI仕様書: https://www.intel.com/content/www/us/en/products/docs/io/universal-serial-bus/extensible-host-controler-interface-usb-xhci.html
- OSDev Wiki: https://wiki.osdev.org/XHCI
