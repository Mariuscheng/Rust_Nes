# Dot-offset 事件模型（設計草案）

> 狀態：**草案，尚未實作**
> 目的：解決 `ppu_vbl_nmi` 7、`cpu_interrupts` 2、`3-nmi_and_irq` 這三個剩下 1–2 個
> CPU cycle 的測項。它們卡在「PPU 事件只有 cycle 精度」這一點上。

---

## 1. 問題

目前 `MainBus::tick()` 一次推進 3 個 PPU dot，然後把 `ppu.nmi_interrupt` 取樣成
`nmi_line`。所以一個發生在 cycle 內第 0、1、2 個 dot 的事件，對 CPU 而言完全一樣。

但 blargg 的測試正是用這一點在分辨：`$2002` 在 VBL set 之後第 0/1/2 個 dot 讀取，
硬體給出「取消 NMI / 取消 NMI / 不取消」三種結果。我們的模型只能給出一種，於是
不管把窗口調成 0–4 dots 都對不上（Round 14 已實測）。

## 2. 需要的資料

每一個 CPU cycle 內，PPU 要回報「這個 cycle 發生了什麼、在第幾個 dot」：

| 事件 | 為什麼需要 |
|---|---|
| `VblSet { dot }` | `$2002` 讀取能不能取消 NMI、能不能讀到旗標 |
| `VblClear { dot }` | NMI 線下降的時機 |
| `NmiEnable { dot }` | `$2000` 開啟 NMI 的 quirk（`nmi_on_timing`） |
| `NmiDisable { dot }` | NMI 線下降 |
| `FrameIrq { dot }` | APU frame IRQ 的 3-cycle 窗口起點 |
| `MapperIrq { dot }` | MMC3 A12（目前已是 cycle 精度，可先不動） |

## 3. 設計

### 3.1 PPU 端

`frame.rs::tick()` 已經是逐 dot 迴圈，所以在事件發生處記錄即可：

```rust
pub struct DotEvent { pub kind: DotEventKind, pub dot: u8 }
pub events: Vec<DotEvent>,   // 每個 CPU cycle 清空
```

`DotEventKind` 建議用 `enum`（不是 bitflag），因為同一 cycle 可能同時有
`VblSet` 與 `NmiEnable`。

### 3.2 Bus 端

`MainBus::tick()` 結尾把 PPU 的 `events` 取走，放進一個小環形緩衝（保留最近 2 個
cycle），並提供：

```rust
/// 最近一個 cycle 內、發生在 `before_dot` 之前的事件。
fn events_before_dot(&self, cycle_delay: u8, dot: u8) -> impl Iterator<Item = DotEventKind>;
```

### 3.3 CPU / 中斷端

- `$2002` 讀取（發生在 cycle 邊界，dot = 3）：
  只有當 `VblSet.dot <= 2` **且** 該 cycle 還沒被 poll 過時才取消 NMI。
  也就是把現在的「同 tick 取消」換成「`VblSet.dot` 在讀取之前」。
- NMI 邊緣鎖存：`nmi_edge_seen` 改成記錄 `edge_dot`，poll 時比較
  `edge_cycle < poll_cycle || (edge_cycle == poll_cycle && edge_dot < 3)`。
- `$2000` 開 NMI 的 quirk：改成「`VblSet` 是否發生在這次寫入之前（同 cycle 內比較 dot）」，
  而不是讀 `status.vblank`。

## 4. 分階段落地

| 階段 | 內容 | 驗證 |
|---|---|---|
| **D1** | PPU 記錄事件 + `MainBus` 環形緩衝（只記錄、不改判定） | 所有測試與 golden 不變；加一個 trace 測試把事件印出來 |
| **D2** | `$2002` 取消 NMI 改用 `VblSet.dot` | `ppu_vbl_nmi` 6 仍通過、7 前進 |
| **D3** | NMI 邊緣改用 `edge_dot` | `ppu_vbl_nmi` 7、`cpu_interrupts` 2 |
| **D4** | `$2000` quirk 改用 dot 比較 | `3-nmi_and_irq` |
| **D5** | frame IRQ 的 dot（目前用 wall-clock countdown，若不需改就跳過） | `apu_test` 7–8 |

## 5. 風險

- **golden 可能位移**：D2/D3 會改變 `$2002` 讀取的判定，若某些 ROM 依賴現行（錯誤的）
  行為，畫面會變。每次 golden 變動都必須逐項解釋。
- **`Vec` 配置**：每 cycle 配置小 `Vec` 會拖慢效能（目前 Mario 2.26 ms/frame）。
  應該用固定大小的 `[Option<DotEvent>; 4]` 或 `ArrayVec` 語意的自製結構。
- **回退**：D2–D4 每一階段都要能單獨 revert；先確認前一階段全綠再進下一階段。

## 6. 為什麼值得做

做完 D2–D4 之後，「PPU 事件 → CPU 中斷」這條路徑就是真正的 dot 精度，剩下兩個
測項（`cpu_interrupts` 2 的 BRK/NMI 同拍、`3-nmi_and_irq` 的優先權）也會一併解決
——它們目前的誤差同樣來自「事件只有 cycle 精度」。
