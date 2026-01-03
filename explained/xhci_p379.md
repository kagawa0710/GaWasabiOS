# p379 - xHCI コントローラ起動（Event Ring / Command Ring）

## 前回（p360）からの追加

p360ではレジスタ構造とDCBAAを作成した。今回はxHCを実際に起動するために必要な**リング構造**を追加。

---

## TRB (Transfer Request Block) とは？

xHCとOS間の**通信単位**。16バイトの構造体。

```
┌─────────────────────────────────────────────────┐
│ GenericTrbEntry (16 bytes, aligned to 16)       │
├─────────────────────────────────────────────────┤
│ data: u64       ← データ（アドレス等）           │
│ option: u32     ← オプション                    │
│ control: u32    ← TRBタイプ、サイクルビット等    │
└─────────────────────────────────────────────────┘
```

### TRBの種類（TrbType）

```rust
enum TrbType {
    Link = 6,                    // リングの終端→先頭に戻る
    EnableSlotCommand = 9,       // デバイススロット有効化
    NoOpCommand = 23,            // 何もしない（テスト用）
    CommandCompletionEvent = 33, // コマンド完了通知
    PortStatusChangeEvent = 34,  // ポート状態変化通知
    // ...
}
```

---

## リング構造（Ring）とは？

xHCとOS間の**循環バッファ**による通信メカニズム。

```
TrbRing (4KB aligned, 16個のTRB)
┌─────┬─────┬─────┬─────┬─────┬─────┐
│TRB 0│TRB 1│TRB 2│ ... │TRB14│TRB15│←Link TRB（先頭に戻る）
└─────┴─────┴─────┴─────┴─────┴─────┘
  ↑
  current_index（現在の処理位置）
```

### サイクルビット（Cycle State）

リングが一周したことを検出するためのフラグ。

```
初期状態: cycle_state = 0
┌─────┬─────┬─────┬─────┐
│ C=0 │ C=0 │ C=0 │ C=0 │  ← 全部0
└─────┴─────┴─────┴─────┘

OSがTRBを書き込む時:
┌─────┬─────┬─────┬─────┐
│ C=1 │ C=1 │ C=0 │ C=0 │  ← 1に変えて「新しいデータ」を示す
└─────┴─────┴─────┴─────┘
  ↑     ↑
  処理済み  処理済み

一周したら:
cycle_state = 1（反転）
┌─────┬─────┬─────┬─────┐
│ C=0 │ C=1 │ C=1 │ C=1 │  ← 今度は0が「新しい」
└─────┴─────┴─────┴─────┘
  ↑
  新しいデータ
```

---

## Command Ring（コマンドリング）

**OS → xHC** の一方通行。OSがxHCに命令を送る。

```
OS                              xHC
│                                │
│  ┌────────────────────────┐    │
│  │ Command Ring           │    │
│  │ [EnableSlot][NoOp]...  │───►│ 読み取って実行
│  └────────────────────────┘    │
│                                │
```

### コード

```rust
struct CommandRing {
    ring: IoBox<TrbRing>,      // 実際のリングデータ
    _cycle_state_ours: bool,   // 現在のサイクル状態
}

impl Default for CommandRing {
    fn default() -> Self {
        let mut this = Self { ring: IoBox::new(), ... };
        // 最後のエントリにLink TRBを設定（先頭に戻る）
        let link_trb = GenericTrbEntry::trb_link(this.ring.as_ref());
        this.ring.write(TrbRing::NUM_TRB - 1, link_trb);
        this
    }
}
```

### OperationalRegistersへの登録

```rust
fn set_cmd_ring_ctrl(&mut self, ring: &CommandRing) {
    // CRCR (Command Ring Control Register) にリングのアドレスを設定
    // | 1 はConsumer Ring Cycle State（初期値）
    self.crcr.write(ring.ring_phys_addr() | 1)
}
```

---

## Event Ring（イベントリング）

**xHC → OS** の一方通行。xHCがOSに通知を送る。

```
OS                              xHC
│                                │
│  ┌────────────────────────┐    │
│  │ Event Ring             │◄───│ イベントを書き込む
│  │ [PortChange][CmdDone]  │    │
│  └────────────────────────┘    │
│    ↓                           │
│  イベントを読んで処理           │
```

### Event Ring Segment Table Entry (ERSTE)

Event Ringは「セグメント」単位で管理される。今回は1セグメントのみ。

```
ERSTE (Event Ring Segment Table Entry)
┌───────────────────────────────────────┐
│ ring_segment_base_address: u64        │ ← リングの物理アドレス
│ ring_segment_size: u16                │ ← TRBの数
│ _rsvdz: [u16; 3]                      │ ← 予約（0）
└───────────────────────────────────────┘
         │
         ▼
┌─────┬─────┬─────┬─────┐
│TRB 0│TRB 1│ ... │TRB15│  ← 実際のEvent Ring
└─────┴─────┴─────┴─────┘
```

### コード

```rust
struct EventRing {
    ring: IoBox<TrbRing>,                    // リング本体
    erst: IoBox<EventRingSegmentTableEntry>, // セグメントテーブル
    _cycle_state_ours: bool,                 // サイクル状態
    erdp: Option<*mut u64>,                  // Event Ring Dequeue Pointer
}
```

### RuntimeRegistersへの登録

```rust
fn init_irs(&mut self, index: usize, ring: &mut EventRing) {
    let irs = self.irs.get_mut(index)?;
    irs.erst_size = 1;                    // セグメント数
    irs.erdp = ring.ring_phys_addr();     // 読み取り位置
    irs.erst_base = ring.erst_phys_addr(); // テーブルアドレス
    irs.management = 0;                    // 割り込み設定
}
```

---

## IoBox とは？

ハードウェアがDMAでアクセスするメモリ領域用のラッパー。

```rust
pub struct IoBox<T: Sized> {
    inner: Pin<Box<IoBoxInner<T>>>,
}

impl<T: Sized> IoBox<T> {
    pub fn new() -> Self {
        let inner = Box::pin(IoBoxInner::new(...));
        disable_cache(&this);  // ← CPUキャッシュを無効化！
        this
    }
}
```

### なぜキャッシュを無効化？

```
通常のメモリアクセス:
CPU ──► Cache ──► RAM
          ↑
        ここにデータがあると、RAMと不一致になる可能性

DMA（xHCが直接アクセス）:
xHC ──────────────► RAM
                     ↑
            Cacheを経由しない！

→ CPUのCacheとxHCが見るRAMの内容が違ってしまう
→ キャッシュを無効化して常にRAMを読み書きする
```

---

## 初期化の全体フロー

```rust
impl Controller {
    fn new(cap_regs, op_regs, rt_regs) -> Result<Self> {
        // 1. xHCをリセット
        op_regs.reset_xhc();

        // 2. Scratchpad Buffers を確保
        let scratchpad = ScratchpadBuffers::alloc(...);

        // 3. DCBAA を作成
        let dcbaa = DeviceContextBaseAddressArray::new(scratchpad);

        // 4. Event Ring を作成
        let event_ring = EventRing::new();

        // 5. Command Ring を作成
        let command_ring = CommandRing::default();

        // 6. Event Ring を RuntimeRegisters に登録
        rt_regs.init_irs(0, &mut event_ring);

        // 7. デバイススロット数と DCBAA を設定
        op_regs.set_num_device_slots(num_slots);
        op_regs.set_dcbaa_ptr(&mut dcbaa);

        // 8. Command Ring を設定
        op_regs.set_cmd_ring_ctrl(&command_ring);

        // 9. xHC を起動！
        op_regs.start_xhc();
        // → "xHC started running!"
    }
}
```

---

## メモリレイアウト全体像

```
┌─────────────────────────────────────────────────────────────┐
│ CapabilityRegisters (BAR0+0x00)                             │
│   └─► caplength, rtsoff でオフセット取得                    │
├─────────────────────────────────────────────────────────────┤
│ OperationalRegisters (BAR0+caplength)                       │
│   ├─► CRCR: Command Ring アドレス                           │
│   └─► DCBAAP: DCBAA アドレス                                │
├─────────────────────────────────────────────────────────────┤
│ RuntimeRegisters (BAR0+rtsoff)                              │
│   └─► IRS[0]: Event Ring 設定                               │
└─────────────────────────────────────────────────────────────┘

ヒープ上（IoBox経由）:
┌─────────────────┐     ┌─────────────────┐
│ Command Ring    │     │ Event Ring      │
│ (TrbRing)       │     │ (TrbRing)       │
└─────────────────┘     └─────────────────┘
                              │
                        ┌─────▼─────┐
                        │   ERSTE   │
                        └───────────┘

┌─────────────────────────┐
│ DCBAA                   │
│ [0] → Scratchpad Table  │
│ [1] → Device 1 Context  │
│ ...                     │
└─────────────────────────┘
```

---

## この段階でできること

- xHCを起動して動作状態にする
- Command Ring / Event Ring の準備完了

## 次のステップ

- ポートスキャン（USBデバイス検出）
- Enable Slot コマンド送信
- Address Device コマンド送信
- 実際のUSBデバイスとの通信
