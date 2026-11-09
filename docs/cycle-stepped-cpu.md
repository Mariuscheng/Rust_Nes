# Cycle-stepped CPU 重構計畫

> 狀態：**計畫，尚未實作**（使用者已授權）
> 前情：`docs/dot-offset-events.md` 記錄了為什麼「只記 dot、不改 CPU 排程」無法解決
> 剩下三個測項。本文件是真正的下一步。

---

## 0. 先釐清一個常見誤解

「cycle-stepped CPU 是業界難題、很多模擬器修不到」——更精確的說法是：

- 它**不是無解**。Mesen、higan/ares、Nintendulator 都通過 blargg 的
  `ppu_vbl_nmi` 與 `cpu_interrupts` 全套。
- 它**很貴**：要求 CPU、PPU、APU、mapper IRQ 共用一個 master clock，每個 cycle
  只推進一步，所有存取都在自己的 cycle 內發生。多數模擬器停在「指令為單位 +
  事後補 tick」，就是我們現在的架構——這種架構可以通過 `instr_timing`、
  `all_instrs`，但過不了「同一個 cycle 內不同 dot」的競賽。
- 所以這不是「能不能」的問題，是「要不要付這個成本」的問題。

## 1. 目標架構

```
MasterClock (u64, 單位 = CPU cycle)
   │
   ├─ CPU: step_cycle() -> 每個 cycle 執行一個 micro-op（讀/寫/內部）
   ├─ PPU: tick_dot() × 3       （dot 為單位，事件自帶 dot）
   ├─ APU: tick_cycle()
   └─ Mapper: tick_cycle() / A12 邊緣
```

關鍵差異：**CPU 不再「整條指令跑完」，而是每 cycle 一步**；中斷在
`step_cycle()` 內部、於硬體規定的 cycle 取樣，而不是在指令邊界事後補判。

## 2. 分階段（每階段獨立 commit + 全測試驗證）

| 階段 | 內容 | 驗證 |
|---|---|---|
| **C0** | 讀一份參考實作（Mesen / higan 的排程器）確認 CPU/PPU 相位模型，特別是 `$2002` 讀取落在 cycle 內哪個 dot | 產出一頁筆記，不改 code |
| **C1** | 引入 `MasterClock`；`NesConsole::step()` 內部改為「跑 N 個 cycle」，指令仍整條執行但每個存取都經過 clock（行為不變） | 23 單元 + 10 ignored + 12 golden 全綠 |
| **C2** | CPU 改成 micro-op 佇列：`decode` 時產生 `[Op; N]`（存取/內部），`step_cycle()` 一次一個 | `instr_timing`、`all_instrs`、golden 不變 |
| **C3** | 中斷輪詢移進 `step_cycle()`：在硬體指定的 cycle 取樣 NMI/IRQ，並就地決定是否插入 7-cycle 中斷序列 | `cpu_interrupts`、`ppu_vbl_nmi` |
| **C4** | `$2002` 讀取的 dot 精度（用 C1 的 clock + C2 的 cycle 位置） | `ppu_vbl_nmi` 7、`3-nmi_and_irq`、`apu_test` 7–8 |

## 3. 風險與回退

- **C1 是純重構**：行為必須完全不變，golden 變動就是 bug。
- **C2 會改寫 40+ 個 handler 的呼叫介面**，但 handler 內部的邏輯可以保留：把
  `bus.read()` 換成「push micro-op」的 macro，語意不變。
- **C3/C4 才是行為改變**，每次只改一項，準備隨時 revert。
- 每一階段結束都要跑 `cargo test`、`cargo test --release -- --ignored`（含
  12 golden）、以及三個計時 ROM。

## 4. 為什麼值得做（除了那三個測試）

目前的架構讓「一次存取」與「一個 cycle」綁死，所以：
- `$2001` 渲染開關、`$2005/$2006` 的 dot 級效果無法精確
- MMC3 的 A12 時鐘只能跟 cycle 對齊
- APU/frame IRQ 只能用 wall-clock 近似

C1–C4 做完之後，這些都會自然變精確，不只是為了三個 blargg 測試。
