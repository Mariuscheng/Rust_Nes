# Cycle-exact 重構：讓中斷在正確的 cycle 被取用

> 狀態：**P2 完成、P1/P3/P4 待做**（見 §4）
> 目標：`3-nmi_and_irq`、`4-irq_and_dma`、`5-branch_delays_irq` 通過；
> `cpu_interrupts` 5/5、`ppu_vbl_nmi` 10/10、`apu_test` 8/8；
> 既有 12 個 golden 檢查點與 22 個單元測試維持不變。

---

## 0. 目前卡在哪（最新實測）

| ROM | 結果 | 卡點 |
|---|---|---|
| `4-irq_and_dma` | ✅ Passed | — |
| `5-branch_delays_irq` | ✅ Passed | — |
| `ppu_vbl_nmi` | 6/10 | 測項 7 `nmi_on_timing`（`$2000` 開啟 NMI 的時機） |
| `cpu_interrupts` | 1/5 | 測項 2 `nmi_and_brk`（最後一列差 1 個 CPU cycle） |
| `apu_test` | 6/8 | 測項 7（DMC timing） |
| `3-nmi_and_irq` | Failed | NMI/IRQ 優先權 |
| `registers` | 卡住 | 需要 soft reset（與本重構無關） |

## 1. 現況模型

- `CPU::step()` 執行整條指令；每個 bus 存取呼叫 `bus.tick()`，一次推進 1 CPU
  cycle（3 PPU dots + 1 APU cycle）。
- handler 跑完後，用 `total_cycles - used` 在**指令尾端補 tick**（padding）。
- `nes.rs` 在指令邊界決定是否取中斷。
- 為了讓幾個測試過去，另外有三個補丁 `skip_irq` / `irq_inhibit_counter` /
  `irq_allow_counter`（**P2 已移除**，見 §3）。

## 2. 要精確的三件事

中斷時序只由三個量決定，缺一不可：

1. **每個存取的 cycle 位置**：現在 123/256 個 opcode 有尾端 padding。若 instruction
   的內部 cycle 其實發生在最後一次存取**之前**（PHA、PLA、JSR、RTS、RTI、RMW 的
   dummy write、寫入指令的 dummy read…），那個存取就早了一～兩個 cycle。
2. **取樣邊界**：中斷在倒數第二個 cycle 的 φ2 取樣；只要 (1) 正確，邊界就正確。
3. **I flag 的可見時機**：CLI/SEI 在最後一個 cycle 才改 I，所以**同一條指令**的
   取樣看不到新值（這就是 CLI 會延遲一條指令的原因）；PLP/RTI 在更早的 cycle
   載入 P，poll 看得到新值。

## 3. 已完成：P2（明確的 poll 語意 + NMI 邊緣鎖存）

- 刪掉三個補丁，改成 `CPU::poll_iflag`：指令開始時取樣 I，CLI/SEI 不動它（poll 看到
  舊值）、PLP/RTI 更新它、BRK/IRQ/NMI 設為 1。
- 刪掉 `skip_irq`，改成 `CPU::poll_delay`：taken 且未跨頁的分支 poll 點早一個 cycle
  （`nmi_line_before_tick(delay)`），跨頁分支維持正常。
- NMI 改成硬體模型：PPU 的 `nmi_interrupt` 是**電平**，bus 在 cycle 邊界偵測上升緣並
  鎖存（`nmi_edge_seen`），poll 使用「上一個 cycle 邊界」的值（`nmi_edge_seen_prev`），
  所以最後一個 cycle 才出現的邊緣要等下一個指令。
- `$2002` 讀取若在**同一個 CPU cycle** 內清掉旗標，會取消尚未提交的邊緣 → 這就是
  `06-suppression` 的 3-dot 窗口。
- 結果：`ppu_vbl_nmi` 測項 6 通過（之前卡在這裡很久），測項 5 的 timing 表維持原值，
  `instr_timing` / `all_instrs` / 12 個 golden 全部不變。

## 4. 階段

| 階段 | 內容 | 驗證 |
|---|---|---|
| **P0** | 診斷：列出 padding opcode、抓各 ROM 的失敗訊息 | ✅ |
| **P1** | 存取位置：補齊 stack/JMP/RTS/RTI/BRK/RMW/寫入指令的內部 cycle 與 dummy access；`used == total` 變成 assertion | `instr_timing`、`all_instrs`、golden |
| **P2** | 中斷語意：poll_iflag / poll_delay / NMI 邊緣鎖存 | ✅ `ppu_vbl_nmi` 6、`cpu_interrupts` 1 |
| **P3** | DMA：poll 點移到 DMA 之前、513/514 的奇偶規則反了 | ✅ `4-irq_and_dma`、`5-branch_delays_irq` |
| **P4** | APU frame IRQ：wall-clock countdown（29831）、3-cycle 斷言窗口、奇偶 jitter | ✅ `apu_test` 1–6 |
| **P5** | 剩下：`$2000` 開 NMI 的時機、NMI/BRK 同拍、NMI/IRQ 優先權 | `ppu_vbl_nmi` 7、`cpu_interrupts` 2、`3-nmi_and_irq` |

### P3/P4 的兩個關鍵發現

- **DMA 的 513/514 規則原本是反的**：`extra = cpu_cycle_count & 1` 應為 `^ 1`。
  反轉後 `4-irq_and_dma` 由 Failed 變 Passed；代價是 `Mario.nes` frame 600 有
  **2 個像素**改變（non-black 57680 → 57678），因為 OAM DMA 的長度影響 sprite-0
  split 的 scanline 位置。golden 已重新生成並在 commit 中註明。
- **frame IRQ 不能用 sequencer 的 step 門檻**：第一段序列有 `shift = 3` 的相位偏移，
  但 IRQ 仍固定在寫入後 29831 個 CPU clock。改成獨立 wall-clock countdown 後，
  測項 4（jitter）與測項 6（flag 寬度）才同時成立。

## 5. 各測項的正確答案（從 blargg ROM 原始碼取得）

這些表格是測試 ROM 自己印出來的預期值，直接當規格用：

- **ppu_vbl_nmi 06-suppression**：讀 `$2002` 的時機（相對 VBL flag 可讀點 F）
  `F-1 → " - -"`（旗標完全沒設）、`F, F+1 → "V -"`（讀到旗標但 NMI 被取消）、
  `F+2 以後 → "V N"`。
- **cpu_interrupts 2-nmi_and_brk**：10 列
  `27 36 | 26 36 | 26 36 | 36 00 ×5 | 27 36 | 27 36`。NMI 在 BRK 開始前 → 兩者都跑；
  NMI 在 BRK 序列中（向量抓取前）→ 用 BRK 已推入的 frame，NMI 看到的 P 帶 B=1、
  IRQ handler 不執行；NMI 在 IRQ handler 的第一個 SEC 之後 → 兩者都跑。
- **cpu_interrupts 3-nmi_and_irq**：12 列，前 10 列 IRQ 欄都是 `00` —— NMI 先被取用，
  它的 handler 讀 `$4015` 清掉 level-triggered frame IRQ，所以 IRQ 永遠不執行。
  最後 2 列 IRQ 已開始序列，NMI 等到 handler 第一條指令後才進去。
- **cpu_interrupts 4-irq_and_dma**：IRQ 回到的指令 offset 依序 0,1,1,2,2,4,4,7,7,7(…)
  ——`sta $4014` 之後整個 513/514 cycle 的 DMA 期間 offset 停在 7/8，只有 +527 到 9。
  → DMA 不會吞掉 pending IRQ，只是延後 handler；DMA 期間才到的 IRQ 要等下一個
  指令結束才認。

  **實測（本專案現況）**：列出的 offset 是 `0,0,1,1,2,2,4,4,7,7,7,7,8,8,…`，整體比
  預期慢一個 clock，而且最後停在 `8 +526`（少了 `9 +527`）。因為每一列的 IRQ 都是
  APU frame IRQ，這個「慢一個 clock」和下面 apu_test 的 29830/29831 是**同一個根因**：
  先修好 frame IRQ 的相位，再回來看 DMA 這一項。已驗證：把 poll 改成「DMA 前取樣」
  對這一項沒有影響（尚未 commit）。
- **cpu_interrupts 5-branch_delays_irq**：taken 且未跨頁的分支**不會**在最後一個 cycle
  poll IRQ，所以分支目標的那條指令會先執行；跨頁分支、JMP、LDA 沒有這個現象。
- **apu_test 6-irq_flag_timing**：寫 `$4017` 後 frame IRQ 旗標在 29831/29832/29833
  三個 CPU cycle 都算「set」；讀 `$4015` 清掉後，同窗口後續的 set 會再把它拉起來，
  所以 29832 讀完、29836 再讀仍為 set；29833 讀完則不會再 set。

  **已試過、會撞牆的做法**：把 IRQ 事件從 29830 移到 29831 並加上 3-cycle 窗口，
  測項 6 的 check #2 就過了，但測項 4 `4-jitter` 立刻變成 "Frame irq is set too late"
  ——jitter 測項把絕對時間釘在 29830。兩者要同時成立，frame IRQ 必須**依 `$4017`
  寫入的 cycle 奇偶**落在不同絕對 cycle（±1），所以下一步要先讀
  `apu_test/source/4-jitter.s` 與 `common/sync_apu.s`，把「寫入相位 → IRQ 絕對
  cycle」的對應表建出來，而不是再調一個全域常數。

## 6. 風險

- **P1 會動到所有 opcode 的執行路徑**：任何一個 handler 少 tick 或多 tick，都會讓
  `instr_timing`/golden 立刻抓到，所以是可回復的。
- **P4 的 dot 精度**：CPU 只能觀察 cycle 邊界，所以「dot 精度」指的是 PPU 在 cycle
  邊界回報的線路狀態要對。現在有 §5 的預期表可以逐項對照。

