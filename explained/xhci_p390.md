# p390 - xHCI イベントポーリング

## 前回（p379）からの追加

p379ではxHCを起動した。今回はイベントを**ポーリング**して処理する仕組みを追加。

---

## イベントポーリングとは？

xHCが発生させるイベント（USB接続、コマンド完了等）を継続的にチェックして処理する仕組み。

```
┌─────────────────────────────────────────────────────────────┐
│                    xHC (ハードウェア)                        │
│                                                             │
│  何かイベント発生 → Event Ringに書き込む                     │
└─────────────────────────────────────────────────────────────┘
                           ↓
┌─────────────────────────────────────────────────────────────┐
│                    Event Ring                               │
│  ┌─────┬─────┬─────┬─────┐                                  │
│  │TRB 0│TRB 1│TRB 2│ ... │  ← xHCがイベントTRBを書き込む     │
│  └─────┴─────┴─────┴─────┘                                  │
│    ↑                                                        │
│    current_index (OSが読み取り位置を追跡)                    │
└─────────────────────────────────────────────────────────────┘
                           ↓
┌─────────────────────────────────────────────────────────────┐
│                    OS (ポーリングループ)                     │
│                                                             │
│  loop {                                                     │
│      event_ring.poll().await?;  // イベントをチェック        │
│      yield_execution().await;    // 他のタスクに譲る         │
│  }                                                          │
└─────────────────────────────────────────────────────────────┘
```

---

## attach() の変更

```rust
pub fn attach(pci: &Pci, bdf: BusDeviceFunction) -> Result<()> {
    // ... 省略 ...

    let xhc = Controller::new(cap_regs, op_regs, rt_regs)?;

    // ControllerをRcでラップ（複数箇所から参照可能に）
    let xhc = Rc::new(xhc);
    {
        let xhc = xhc.clone();
        spawn_global(async move {
            loop {
                // イベントをポーリング
                xhc.primary_event_ring.lock().poll().await?;
                // 他のタスクに実行を譲る
                yield_execution().await;
            }
        })
    }

    Ok(())
}
```

### なぜRc<Controller>？

- 今後、ポーリングループ以外からもControllerを参照する必要がある
- `Rc`（Reference Counted）で複数箇所からの共有参照を可能に
- `clone()`で参照カウントを増やして別タスクに渡す

---

## EventRing::pop() - イベント取り出し

```rust
fn pop(&mut self) -> Result<Option<GenericTrbEntry>> {
    // 新しいイベントがあるかチェック
    if !self.has_next_event() {
        return Ok(None);
    }

    // 現在のTRBを取得
    let e = self.ring.as_ref().current();
    let eptr = self.ring.as_ref().current_ptr() as u64;

    // インデックスを進める
    unsafe { self.ring.get_unchecked_mut() }
        .advance_index_notoggle(self.cycle_state_ours)?;

    // ERDP（Event Ring Dequeue Pointer）を更新
    // → xHCに「ここまで読んだよ」と伝える
    unsafe {
        let erdp = self.erdp.expect("erdp is not set");
        write_volatile(erdp, eptr | (*erdp & 0b1111));
    }

    // リングが一周したらサイクルビットを反転
    if self.ring.as_ref().current_index() == 0 {
        self.cycle_state_ours = !self.cycle_state_ours;
    }

    Ok(Some(e))
}
```

### has_next_event() の仕組み

```rust
fn has_next_event(&self) -> bool {
    self.ring.as_ref().current().cycle_state() == self.cycle_state_ours
}
```

サイクルビットでイベントの有無を判定：
- xHCがイベントを書き込む時、サイクルビットを**OS側の値と同じ**にする
- OSがイベントを読んだ後、インデックスが0に戻ったらサイクルビットを反転
- これにより、古いイベントと新しいイベントを区別できる

---

## EventRing::poll() - イベント処理

```rust
async fn poll(&mut self) -> Result<()> {
    if let Some(e) = self.pop()? {
        let mut consumed = false;

        // 登録されたwaiterを順にチェック
        for w in &self.wait_list {
            if let Some(w) = w.upgrade() {
                let w: &EventWaitInfo = w.as_ref();
                if w.matches(&e) {
                    w.resolve(&e)?;
                    consumed = true;
                }
            }
        }

        // どのwaiterも処理しなかったらログ出力
        if !consumed {
            info!("unhandled event: {e:?}");
        }

        // 無効になったwaiterを削除
        // ...
    }
    Ok(())
}
```

---

## イベント待機の仕組み

### EventWaitCond - 待機条件

```rust
struct EventWaitCond {
    trb_type: Option<TrbType>,  // 特定のイベントタイプのみ
    trb_addr: Option<u64>,       // 特定のTRBアドレスに対するイベント
    slot: Option<u8>,            // 特定のスロット（デバイス）
}
```

### EventWaitInfo - 待機情報

```rust
struct EventWaitInfo {
    cond: EventWaitCond,                    // 待機条件
    trbs: Mutex<VecDeque<GenericTrbEntry>>, // マッチしたイベントを格納
}

impl EventWaitInfo {
    fn matches(&self, trb: &GenericTrbEntry) -> bool {
        // 条件にマッチするかチェック
        if let Some(trb_type) = self.cond.trb_type {
            if trb.trb_type() != trb_type as u32 {
                return false;
            }
        }
        // ... slot, trb_addr も同様にチェック
        true
    }

    fn resolve(&self, trb: &GenericTrbEntry) -> Result<()> {
        // マッチしたイベントをキューに追加
        self.trbs.under_locked(&|trbs| {
            trbs.push_back(trb.clone());
            Ok(())
        })
    }
}
```

### wait_list - waiterの管理

```rust
struct EventRing {
    // ...
    wait_list: VecDeque<Weak<EventWaitInfo>>,  // 弱参照のリスト
}
```

- `Weak<T>`: 強参照（Rc）がなくなったら自動的に無効になる
- これにより、waiterが不要になったら自動的にクリーンアップされる

---

## GenericTrbEntry の追加メソッド

```rust
impl GenericTrbEntry {
    fn data(&self) -> u64 {
        self.data.read()
    }
    fn slot_id(&self) -> u8 {
        self.control.read_bits(24, 8).try_into().unwrap()
    }
    fn trb_type(&self) -> u32 {
        self.control.read_bits(10, 6)
    }
    fn cycle_state(&self) -> bool {
        self.control.read_bits(0, 1) != 0
    }
}
```

### TRBのcontrolフィールド構造

```
control (32bit)
┌────────┬────────┬────────┬────────┐
│ 31..24 │ 23..16 │ 15..10 │  9..0  │
│ SlotID │   -    │TrbType │ Flags  │
└────────┴────────┴────────┴────────┘
                            ↑
                            bit 0 = Cycle State
```

---

## データフロー全体像

```
                  ┌──────────────────┐
                  │ USBデバイス接続   │
                  │ コマンド完了     │
                  │ 転送完了         │
                  └────────┬─────────┘
                           │ xHCがイベント生成
                           ▼
┌─────────────────────────────────────────────────────────────┐
│ Event Ring                                                  │
│ ┌─────┬─────┬─────┬─────┐                                   │
│ │ PSC │ CC  │  ?  │  ?  │  PSC=PortStatusChange            │
│ └─────┴─────┴─────┴─────┘  CC=CommandCompletion             │
└─────────────────────────────────────────────────────────────┘
                           │ poll()
                           ▼
┌─────────────────────────────────────────────────────────────┐
│ wait_list                                                   │
│ ┌──────────────────┐  ┌──────────────────┐                  │
│ │ EventWaitInfo    │  │ EventWaitInfo    │                  │
│ │ cond: CC, slot=1 │  │ cond: Transfer   │                  │
│ │ trbs: [...]      │  │ trbs: [...]      │                  │
│ └──────────────────┘  └──────────────────┘                  │
│         │                     │                             │
│      マッチ!               マッチしない                      │
│         │                                                   │
│         ▼                                                   │
│      resolve() → trbs.push_back()                           │
└─────────────────────────────────────────────────────────────┘
                           │ マッチしない場合
                           ▼
                  info!("unhandled event: ...")
```

---

## EventFuture（将来の拡張用）

```rust
#[derive(Clone)]
struct EventFuture {
    wait_on: Rc<EventWaitInfo>,
    _pinned: PhantomPinned,
}
```

この段階では定義のみ。次のステップで`Future`トレイトを実装して、
`await`でイベントを待機できるようになる。

---

## この段階でできること

- イベントの継続的なポーリング
- 未処理イベントのログ出力
- イベント待機のインフラ準備

## 次のステップ

- PORTSC（ポートステータス）レジスタの読み取り
- ポート状態変化イベント（PortStatusChangeEvent）の処理
- USBデバイスの検出と初期化
