# NMI 時序設計整理（`ppu_vbl_nmi` 05 / 06）

> 狀態：設計稿。目前 `ppu_vbl_nmi` 通過 test 2–5，卡在 test 6 `06-suppression`。
> 這份文件把「PPU 何時拉高 NMI 線 / CPU 何時取樣 / 何時派發」拆成三條時間軸，
> 用兩張測試表當同時約束，並記錄已經排除的方案。

---

## 1. 約束：兩張表必須同時成立

測試以 `sync_vbl` 對齊到 1/3 CPU clock，再用 `sync_vbl_delay` 每列延後 1 個 PPU dot
（frame 長度 29780.67 CPU clocks，每 frame 相位漂移 1/3 clock，因此列與列之間可以
做到 1 dot 的解析度）。

**05-nmi_timing**（NMI 發生在第幾個 instruction 之後）：

```
row:  00 01 02 03 04 05 06 07 08 09
want:  4  4  4  3  3  3  3  3  3  2
```

**06-suppression**（第一欄：$2002 讀到的 VBL；第二欄：是否發生 NMI）：

```
row:  00 01 02 03 04 05 06 07 08 09
want: -N -N -N -N -- V- V- VN VN VN
```

兩者的張力：05 要求 NMI 盡量早（目前用「旗標設定的同一個 dot 就斷言」才過），
06 的 row 04–06 要求 NMI 晚到「同一條指令裡的 $2002 讀取可以把它取消」。

---

## 2. 目前的時間軸

| 事件 | 目前實作 |
|---|---|
| VBL 旗標設定 | scanline 241, dot 1（`ppu/frame.rs`） |
| 抑制窗口 | `vblank_just_set`：只在「最後處理的那個 dot」為真 |
| NMI 線 | `nmi_interrupt`；旗標設定時**立即**拉高（已移除 1-dot 延遲） |
| NMI 線清除 | `$2002` 讀取、抑制、停用 NMI、pre-render 開始 |
| CPU 取樣 | `nmi_before_tick`＝最後一個 tick 之前的線狀態（penultimate cycle 尾端） |
| 派發 | 指令結束後（`NesConsole::step`） |
| 抑制讀取 | 清旗標與 `nmi_interrupt`；若 NMI 已派發則無法挽回 |

**已觀測到的現象**

- 立即斷言 → 05 通過。
- 斷言延後 1 dot → 05 的轉折列（03、09）各晚一個 instruction。
- CPU 端「取消 latch」→ 06 的表格完全不變（NMI 已在更早的指令邊界派發）。
- 全域把存取提前 1 dot（`read/write` 改成 2 dots → access → 1 dot）：
  `ppu_vbl_nmi` 退到 test 5、`apu_test` 退到 test 4 —— 全域位移會破壞 APU 與 05 的
  既有校準，因此**排除**。

---

## 3. 提議：把「線」與「取樣」拆開

核心想法：**NMI 線的斷言時間**與**CPU 的取樣時間**是兩個獨立參數，目前兩者都綁在
CPU cycle 邊界上，才會互相打架。

1. **線**：VBL 旗標在 dot D 設定，NMI 線在 **D+1** 拉高（符合硬體的 1 clock 延遲）。
2. **取樣**：CPU 在 penultimate cycle 的**最後一個 dot** 取樣，而不是 cycle 邊界。
   bus 每個 dot 記錄一次線狀態（3 個 slot 的 ring），CPU 取對應 slot。
3. **抑制**：讀取落在 dot D 時，旗標還沒拉線（D+1），所以可以取消；
   同時清掉 CPU 的 `nmi_latch`。這一點只有在「線延後」成立時才會生效。
4. **派發**：維持在指令結束後，不再改動。

為什麼可能同時滿足：

- 05 量的是「NMI 在哪个 instruction 後被取用」。把取樣點放在 penultimate cycle 的
  最後一個 dot（比現在的 cycle 邊界晚 1 dot），可以補回「線延後 1 dot」造成的晚一
  個 instruction。
- 06 量的是「旗標/線是否已經可見」。線延後 1 dot 讓 row 04–06 的抑制讀取趕得上，
  而 row 07+ 仍然是 NMI 已發生。

這是一個假設，需要實測驗證；文件不宣稱它一定成立。

---

## 4. 實作步驟

1. `MainBus`：新增 `nmi_history: [bool; 3]`，在 `advance_dots` 每個 dot 後寫入。
2. `CPU::step`：penultimate cycle 的線狀態改從 history 取「該 cycle 的最後一個 dot」。
   （目前的 `nmi_before_tick` 是 cycle 邊界值，直接對應 history 的前一個 cycle 尾端，
   需要確認索引。）
3. `PPU`：`nmi_interrupt` 改成「旗標設定後 1 dot」才 true；抑制時取消這個 pending。
4. 驗證：
   - `ppu_vbl_nmi` 05/06 兩張表逐列比對（用 `$6000` 文字 + 截圖）。
   - `apu_test`、golden frames、unit tests 不得退步。

若第 2 步需要更細的「每 dot 取樣」而 CPU 無法提供，才考慮把 `read/write` 拆成
per-dot 存取（成本較高，先不做）。

---

## 5. 風險與回退

| 風險 | 對策 |
|---|---|
| 05 或 06 其中一邊退步 | 每次只改一項，兩張表都跑；退步就回退 |
| 破壞 02–05 | 全套 `cargo test --release -- --ignored` + golden frames 把關 |
| 需要 per-dot 存取才能收斂 | 先停在 6/10，把 06 標成 known limitation，轉做其他相容性工作 |

---

## 6. 已排除的方案（避免重做）

- **CPU 端取消 latch**：NMI 已在更早的指令邊界派發，清 latch 來不及（表格不變）。
- **全域存取提前 1 dot**：`apu_test` 與 `ppu_vbl_nmi` 同時退步。
- **單純把斷言延後 1 dot**：05 的轉折列晚一個 instruction。
- **把 VBL 清除改回 cycle 1**：在現行取樣點下 03 會退步（現用 cycle 2 校準）。

---

## 7. 實作後的實驗結果（已回退）

照第 3 節實作（NMI 線延後 1 dot、`nmi_history` 每 dot 記錄、取樣 index 可調），
再用 `NMI_SAMPLE_DOTS ∈ {0,1,2}` 掃描：

| 取樣 index | `ppu_vbl_nmi` 結果 |
|---|---|
| 0（最後一個 dot） | test 5 `nmi_timing` 失敗 |
| 1 | test 5 失敗 |
| **2（最後 cycle 的第一個 dot）** | **test 5 通過**，進到 test 6 |

結論（部分成立）：

- 第 3 節的假設對 **05** 有效：線延後 1 dot 可以由「取樣點往後挪 1 dot」補回來。
- 但 **06 的表格與修改前完全相同**（`04 -N`、`05/06 VN`），逐 byte 一致。
  原因：這些列的 NMI 在**更早的指令邊界**就已經派發，無論在讀取指令內怎麼取樣都來不及取消。
  要修 06，需要把「是否派發 NMI」的決定延後一個指令（讓抑制讀取有機會撤回），
  這會直接影響已通過的 05，風險高。

由於淨結果與修改前相同、而模型多了 per-dot history 與額外延遲，實驗已回退
（`ppu_vbl_nmi` 維持 6/10）。若之後要再挑戰 06，建議從「延後一個指令再決定 NMI」
著手，而不是再調取樣點。