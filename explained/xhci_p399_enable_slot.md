# p399 - Enable Slot Command（スロット割り当て）

## 前回（p398）からの追加

p398ではポート初期化処理を`init_port()`に分離した。今回はxHCにコマンドを送信してUSBデバイス用の**スロット**を割り当てる。

---

## スロットとは？

xHCは最大255台のUSBデバイスを管理できる。各デバイスには**スロット番号（1〜255）**が割り当てられる。

```
┌─────────────────────────────────────────────────────────────┐
│                    xHC (ホストコントローラ)                  │
│                                                             │
│  スロット1 → USBキーボード                                   │
│  スロット2 → USBマウス                                       │
│  スロット3 → USBメモリ                                       │
│  ...                                                        │
└─────────────────────────────────────────────────────────────┘
```

Enable Slot Commandを送ると、xHCが空いているスロット番号を返す。

---

## 全体の流れ

```
┌─────────────────────────────────────────────────────────────┐
│ 1. Enable Slot Command を Command Ring に書き込む            │
└─────────────────────────────────────────────────────────────┘
                           ↓
┌─────────────────────────────────────────────────────────────┐
│ 2. Doorbell[0] を鳴らして xHC に通知                         │
└─────────────────────────────────────────────────────────────┘
                           ↓
┌─────────────────────────────────────────────────────────────┐
│ 3. xHC がコマンドを処理                                      │
│    → 空きスロットを探して割り当て                            │
│    → Command Completion Event を Event Ring に書き込む       │
└─────────────────────────────────────────────────────────────┘
                           ↓
┌─────────────────────────────────────────────────────────────┐
│ 4. OS が Event Ring をポーリングしてイベントを取得           │
│    → slot_id フィールドから割り当てられたスロット番号を取得  │
└─────────────────────────────────────────────────────────────┘
```

---

## Doorbell レジスタ

### 役割
xHCに「処理すべきコマンド/転送がある」と通知するためのレジスタ。

```rust
// [xhci] 4.7 Doorbells
// index 0: ホストコントローラ用（コマンド通知）
// index 1-255: デバイス用（転送通知）
pub struct Doorbell {
    ptr: Mutex<*mut u32>,
}

impl Doorbell {
    // [xhci] 5.6 Doorbell Registers
    // bit 0..8: DB Target
    // bit 16..32: DB Task ID
    pub fn notify(&self, target: u8, task: u16) {
        let value = (target as u32) | (task as u32) << 16;
        unsafe { write_volatile(*self.ptr.lock(), value) }
    }
}
```

### ドアベルの配置
```
dboff (Doorbell Offset) からの配置:
┌─────────┬─────────┬─────────┬─────────┐
│ DB[0]   │ DB[1]   │ DB[2]   │ ...     │
│ (xHC)   │ (Slot1) │ (Slot2) │         │
└─────────┴─────────┴─────────┴─────────┘
    ↑
  コマンド通知用
```

---

## CommandRing::push() - コマンド書き込み

```rust
pub fn push(&mut self, mut src: GenericTrbEntry) -> Result<u64> {
    let ring = unsafe { self.ring.get_unchecked_mut() };

    // リングが満杯かチェック
    if ring.current().cycle_state() != self.cycle_state_ours {
        return Err("Command Ring is Full");
    }

    // サイクルビットをセット
    src.set_cycle_state(self.cycle_state_ours);

    // 現在位置のポインタを保存（イベント待機用）
    let dst_ptr = ring.current_ptr();

    // TRBを書き込んでインデックスを進める
    ring.write_current(src);
    ring.advance_index(!self.cycle_state_ours)?;

    // Link TRBに到達したらスキップしてサイクル反転
    if ring.current().trb_type() == TrbType::Link as u32 {
        ring.advance_index(!self.cycle_state_ours)?;
        self.cycle_state_ours = !self.cycle_state_ours;
    }

    // ポインタを返す（イベント照合用）
    Ok(dst_ptr as u64)
}
```

### なぜポインタを返す？
Command Completion Eventには、どのコマンドが完了したかを示す**コマンドTRBのアドレス**が含まれる。このアドレスと照合してイベントを待機する。

---

## EventFuture - 非同期イベント待機

```rust
impl EventFuture {
    fn new_for_trb(event_ring: &Mutex<EventRing>, trb_addr: u64) -> Self {
        Self::new(
            event_ring,
            EventWaitCond {
                trb_addr: Some(trb_addr),  // このアドレスのイベントを待つ
                ..Default::default()
            },
        )
    }
}

impl Future for EventFuture {
    type Output = Result<GenericTrbEntry>;

    fn poll(self: Pin<&mut Self>, _: &mut Context) -> Poll<Result<GenericTrbEntry>> {
        let mut_self = unsafe { self.get_unchecked_mut() };

        // イベントが届いていればReady、なければPending
        if let Some(trb) = mut_self.wait_on.trbs.lock().pop_front() {
            Poll::Ready(Ok(trb))
        } else {
            Poll::Pending
        }
    }
}
```

### イベント待機の仕組み
```
1. EventFuture作成時にEventWaitInfoをwait_listに登録
2. ポーリングループがイベントを検出
3. wait_list内の条件にマッチするwaiterを探す
4. マッチしたらwaiterのtrbs queueにイベントを追加
5. 次のpoll()でイベントを取得してPoll::Ready
```

---

## Controller::send_command() - コマンド送信

```rust
async fn send_command(&self, cmd: GenericTrbEntry) -> Result<GenericTrbEntry> {
    // 1. コマンドをリングに書き込み
    let cmd_ptr = self.command_ring.lock().push(cmd)?;

    // 2. ドアベルを鳴らしてxHCに通知
    self.notify_xhc();

    // 3. イベント完了を非同期で待機
    EventFuture::new_for_trb(&self.primary_event_ring, cmd_ptr).await
}

fn notify_xhc(&self) {
    self.regs.doorbell_regs[0].notify(0, 0);
}
```

---

## init_port() の変更

```rust
async fn init_port(xhc: Rc<Controller>, port: usize) -> Result<u8> {
    let portsc = xhc.regs.portsc.get(port).ok_or("invalid portsc")?;

    // ポートリセット
    info!("xhci: resetting port {port}");
    portsc.reset_port().await;
    info!("xhci: port {port} has been reset");

    // ポート有効化確認
    portsc.is_enabled().then_some(()).ok_or("port is not enabled")?;
    info!("xhci: port {port} is enabled");

    // Enable Slot Command 送信
    let slot = xhc
        .send_command(GenericTrbEntry::cmd_enable_slot())
        .await?
        .slot_id();

    Ok(slot)
}
```

---

## データフロー全体像

```
┌──────────────────────────────────────────────────────────────────┐
│ init_port()                                                       │
│                                                                   │
│   1. reset_port()                                                 │
│   2. is_enabled() 確認                                            │
│   3. send_command(Enable Slot) ─────────────────────┐             │
│                                                     │             │
└─────────────────────────────────────────────────────│─────────────┘
                                                      │
                                                      ▼
┌──────────────────────────────────────────────────────────────────┐
│ send_command()                                                    │
│                                                                   │
│   command_ring.push() ──→ Command Ring に TRB 書き込み            │
│   notify_xhc() ─────────→ Doorbell[0] を鳴らす                    │
│   EventFuture::await ───→ イベント待機                            │
│                                                                   │
└──────────────────────────────────────────────────────────────────┘
                      │                    ↑
                      ▼                    │
┌─────────────────────────────┐   ┌────────────────────────────────┐
│ xHC (ハードウェア)           │   │ ポーリングループ               │
│                             │   │                                │
│ Command Ring読み取り         │   │ Event Ring から pop()          │
│ Enable Slot 処理            │   │ wait_list とマッチング          │
│ Event Ring に結果書き込み   │──→│ EventWaitInfo.resolve()        │
│                             │   │                                │
└─────────────────────────────┘   └────────────────────────────────┘
```

---

## 期待される出力

```
[INFO] xhci: port 5 is connected
[INFO] xhci: resetting port 5
[INFO] xhci: port 5 has been reset
[INFO] xhci: port 5 is enabled
[INFO] slot 2 is assigned for port 5
```

---

## この段階でできること

- xHCへのコマンド送信
- コマンド完了イベントの非同期待機
- USBデバイスへのスロット割り当て

## 次のステップ

- Address Device Command（デバイスアドレス割り当て）
- Input Context / Output Context の設定
- Control Transfer（デバイス情報取得）
