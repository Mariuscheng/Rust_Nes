# C0：相位模型筆記（讀 blargg `common/sync_vbl.s`）

> 目的：在動手 C1 之前，先把「測試 ROM 怎麼做到 1 dot 解析度」釘死。
> 結論：**不是靠 CPU 在 cycle 內移動，而是靠 frame 週期的分數部分。**

---

## 1. 原文關鍵句

`sync_vbl.s` 開頭：

> Synchronizes EXACTLY to VBL, to accuracy of **1/3 CPU clock**
> (1/2 CPU clock if PPU is enabled).

`sync_vbl_delay` 的註解：

> VBL occurs every **29780.67** CPU clocks, therefore each iteration of the loop
> is like delaying **1/3 CPU clock (1 PPU clock)**.

## 2. 機制

- NTSC frame = 89342 PPU dots = **29780.67 CPU clocks**（不是整數）
- `sync_vbl_delay` 每一輪延遲 29781 CPU clocks，比一個 frame 多 0.33 → 每一輪
  相對 VBL 就滑動 1/3 CPU clock = **1 PPU dot**
- 所以「A=0..8 每列差 1 dot」是**跨 frame 累積**出來的，不是 CPU 在單一 cycle 內
  移動

## 3. 對我們的意義

我們的 PPU 每 frame 前進 89342 dots（odd frame 少 1 dot），CPU 每 cycle 前進 3 dots，
所以 `89342 mod 3 = 2` —— **每 frame 相位自然滑動 2 dots（等價 -1）**。這與硬體一致，
也就是說「跨 frame 的 1-dot 滑動」在我們的模型裡是存在的。

因此剩下三個測項的差距**不是**「缺少 1 dot 的解析度」這麼籠統，而是更具體的一件事：
**`$2002` 讀取在 VBL set 前後的「可見性規則」差了一個 dot 級別的邊界**。我們的
`vblank_just_set` 是「整個 tick（3 dots）寬」，硬體是「set dot 之後 1 個 dot」。

## 4. 下一個具體實驗（C1 之前）

在 `sync_vbl_delay` 的路徑上記錄：每一列（row）的 enable 寫入、VBL set、`$2002` 讀取
各自的絕對 dot，確認我們的「跨 frame 滑動」是否真的是每列 1 dot（而不是 3 的倍數）。
若是 1 dot，那 C1 只需要把可見性規則改成 dot 級，不必重寫 CPU 排程。

## 4b. 量測結果（Round 35）

測項 7 各列的實際 dot（VBL set 固定在 82182）：

| 列 | enable | disable |
|---|---|---|
| 02 | 82155 | 82263 |
| 03 | 82156 | 82264 |
| 04 | 82157 | 82265 |
| 05 | 82158 | 82266 |
| 06 | 82159 | 82213 |
| 07 | 82160 | 82214 |
| 08 | 82161 | 82215 |

兩件事：

1. **enable 每列正好差 1 dot**（82155→82161）→ 證實 C0 的「跨 frame 漂移」在我們的模型裡是 1 dot，不是 3 的倍數。
2. disable 有兩種間隔：108 dots（列 02–05）與 54 dots（列 06–08）。
   差了 18 個 CPU cycle，不是 NMI handler 的 11，也不是單純的程式碼路徑差異，
   需要下一輪單獨查（很可能是 sync_vbl 輪詢在不同相位下多跑幾輪）。

## 4c. 界線找到了（Round 36）

把 enable dot 與「有沒有 NMI」對起來（VBL set = 82182）：

| enable dot | 距 set | 我們的 NMI | 預期 |
|---|---|---|---|
| 82155 | 27 | 有 | N |
| 82156 | 26 | 有 | N |
| 82157 | 25 | 有 | N |
| 82158 | 24 | 有 | **-** |
| 82159 | 23 | 無 | - |
| 82160 | 22 | 無 | - |
| 82161 | 21 | 無 | - |

**界線只差 1 個 dot**：我們允許到「距 set 24 dots」，硬體只到 25。

另外，先前看到的 disable 間隔 108/54 dots 的分界**正好落在同一列**：
有 NMI 的列間隔 108、沒有的列間隔 54。所以長短間隔是「NMI 有沒有被取用」的**結果**
（handler 11 cycles + dispatch），不是原因——這一點已釐清。

→ 剩下的唯一差異是：**$2000 寫入（NMI enable）的效果，硬體比我們晚 1 個 PPU dot 才被 VBL set 看到**。
這正是 dot 級可見性問題，屬 C4；但也許可以用「把 $2000 的效果延後 1 dot 生效」的局部做法先驗證。

## 5. 如果第 4 點成立

那 `docs/cycle-stepped-cpu.md` 的 C2–C4 可以縮小成「把 `$2002` 的可見性規則改成
dot 級」，成本遠低於完整 micro-op 排程器。這一點必須先量測再決定。
