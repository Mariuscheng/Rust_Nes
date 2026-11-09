# Step 1：把「時間」收斂成單一時鐘（per-access ticking）

> 狀態：**1a、1b 已完成**（見 §0 進度），1c–1e 待做。
> 目標：讓每一次 CPU 匯流排存取都推進 1 個 CPU cycle（3 PPU dots + 1 APU cycle），
> 取代現在「整條指令跑完再補 tick」的模型。
> 這一階段**不做**完整 cycle-stepped CPU state machine；它先消除「暫存器存取相位」
> 與「APU 批次推進」這兩類誤差，再用量測決定是否需要更進一步。

---

## 0. 進度

| 階段 | 狀態 | 內容 / 觀測 |
|---|---|---|
| **1a** | ✅ `a61257e` | `MainBus` 去 `RefCell`、`read`/`write` 改 `&mut`、新增 `tick()`。行為不變：golden 10/10 一致。 |
| **1b** | ✅ 本次 | `read`/`write` 內部 tick（**先 tick 再存取**）、`CPU::step` 用 `cpu_cycles()` 記帳並補足剩餘 cycle、移除 `nes.rs` 補 tick 迴圈與 `ppu_ticks_consumed`、`nmi()`/`irq()`/`op_brk()` 改由自身存取 tick。 |
| 1c | ⬜ | APU lockstep + DMC request |
| 1d | ⬜ | DMA 移到指令後執行 |
| 1e | ⬜ | 中斷取樣精確化（移除 `skip_irq` 等近似） |

**1b 觀測**

- golden：只有 `Mario.nes` frame 600 改變，**6 個像素**（y=30、x=102..111，狀態列
  `×00` 字型頂端一列），非黑像素數不變（57680）。其餘 9 個檢查點完全一致。
  原因為 sprite-0 split 的 scanline 邊界位移一格 —— 正是「暫存器存取相位」改變的預期結果。
- ROM 套件：`all_instrs` / `official_only` / `instr_timing` 仍通過；
  `vbl_set_time` / `jitter` / `nmi_and_brk` 仍為已知失敗（1c–1e 才處理 APU 與中斷取樣）。
- 效能：Mario headless 2.34 ms/frame（1a 為 2.26 ms），差異在雜訊範圍。
- debug build 跑 `instr_timing`：沒有觸發 cycle 記帳的 `debug_assert`。

---

## 1. 現況

`NesConsole::step()`（`src/nes.rs:103`）目前的順序是：

```
cpu.step(&mut bus)                       // 整條指令（含所有 $2000-$2007 存取）
  ├─ bus.read / bus.write                // 只做記憶體動作，不推進時間
  └─ 少數 op 例外：irq()/op_brk() 手動 bus.advance_ppu_ticks(...)

bus.advance_ppu_cycle() × 剩餘 cycles     // 事後補 tick，NMI 只在倒數第二個 cycle 取樣
NMI / IRQ 判斷                            // 只在指令邊界
bus.step_apu(整條指令的 cycles)           // APU 批次推進
```

**後果**

- 指令內所有 PPU/APU 暫存器存取都發生在該指令時間窗的「起點」，相位誤差 0–7 cycles。
- `$2002` 讀 VBlank 的競賽、`$4017` 寫入與 frame IRQ 的相對位置都會差。
- APU frame IRQ / DMC IRQ 的斷言時間以指令為單位量化。
- `$4014` DMA 在寫入的當下就搬完 OAM 並記帳，PPU 事後才補。

已驗證的失敗（`cargo test --release -- --ignored`）：
`ppu_vbl_nmi` test 2/10（`vbl_set_time`）、`apu_test` test 4/8（jitter）、
`cpu_interrupts` test 2/5（`nmi_and_brk`）。

---

## 2. 目標架構

```
MainBus::read(addr)   → tick(1) → 解碼 → 回傳值
MainBus::write(addr)  → tick(1) → 解碼 → 寫入
MainBus::tick()       → PPU tick(3 dots)、APU tick(1 cycle)、drain mapper IRQ

CPU::step()           → 執行指令（存取自動 tick）
                      → 結尾補 (宣告 cycles − 已用 cycles) 次 bus.tick()
```

時間只有一個來源：`MainBus::tick()`。CPU、PPU、APU、mapper 都不知道「現在是第幾條指令」。

### 介面變更

```rust
pub trait BusInterface {
    fn read(&mut self, addr: u16) -> u8;          // 由 &self → &mut self，內部 tick
    fn write(&mut self, addr: u16, data: u8);      // 內部 tick
    fn tick(&mut self);                            // 內部 cycle（無匯流排動作）
    fn set_cpu_pc(&mut self, pc: u16);
    fn cpu_cycles(&self) -> u64;                   // 供 step() 計算已消耗 cycle

    // 移除：
    // fn advance_ppu_cycle(&mut self) -> bool;
    // fn advance_ppu_ticks(&mut self, ticks: u16) -> bool;
    // fn take_immediate_stolen_cycles(&mut self) -> u16;
}
```

`MainBus` 的欄位由 `RefCell<PPU>` / `RefCell<APU>` / `RefCell<Joypad>` 改為直接持有
（`read`/`write` 已是 `&mut self`，不再需要 interior mutability）：

```rust
pub struct MainBus {
    ram: Ram,
    mapper: MapperVariant,
    ppu: PPU,          // 原本 RefCell<PPU>
    apu: APU,          // 原本 RefCell<APU>
    joypad1: Joypad,   // 原本 RefCell<Joypad>
    joypad2: Joypad,
    cpu_cycles: u64,
    open_bus_latch: u8,
    nmi_edge: bool,        // PPU vblank 上升緣（跨 cycle 保留）
    dma_request: Option<u8>,
}
```

連帶影響：
- `ImmediateMemoryBusRef`（`src/bus/bus.rs:34`）整段刪除 —— APU 不再需要讀 bus。
- `NesConsole::set_button` / `drain_audio_samples` 由 `&self` 改 `&mut self`；
  `poll_input` 改收 `&mut NesConsole`。
- 測試用 `DummyBus`（`src/timing.rs`）改成新介面。

### CPU 端的記帳

```rust
pub fn step(&mut self, bus: &mut dyn BusInterface) -> u16 {
    let start = bus.cpu_cycles();
    // ... 執行指令（所有 read/write 都各自 tick 了一 cycle）...
    let total = base_cycles + extra_cycles;
    let used = (bus.cpu_cycles() - start) as u16;
    debug_assert!(used <= total, "instruction used more cycles than declared");
    for _ in used..total {
        bus.tick();
    }
    total
}
```

`CPU::ppu_ticks_consumed` 欄位刪除；`cpu::irq()` 與 `op_brk()` 內手動的
`advance_ppu_ticks(18)` / `tick_ppu_cycle()` 全部移除，改成單純的 push / vector read
（這些存取本身就會 tick）。

### APU

```rust
impl APU {
    // 原本 step(cpu_cycles, bus) -> stolen，改成：
    fn tick_one(&mut self);                 // 1 CPU cycle，不含 bus
    fn take_sample_request(&mut self) -> Option<u16>;  // DMC 要讀的位址
    fn fill_sample(&mut self, byte: u8);
}
```

`MainBus::tick()`：
```
ppu.tick(3)
apu.tick_one()
if let Some(addr) = apu.take_sample_request() { let b = self.peek(addr); apu.fill_sample(b); }
drain a12 → mapper.tick_mapper_irq()
```

DMC 的 4-cycle steal 改成「`MainBus::tick()` 連跑 4 次但不執行 CPU」——
由 `NesConsole::step()` 在每條指令後檢查 `bus.take_stall_cycles()` 並補 tick。

### DMA（$4014）

寫入 `$4014` 時只設定 `dma_request`。指令結束後由 `NesConsole::step()` 呼叫
`bus.run_dma()`：513/514 cycles 全部以 `tick()` 推進、逐 byte 搬 OAM，
期間 CPU 不執行。這比現在「寫入當下搬完 + 事後記帳」更接近硬體，也讓 DMA 期間的
NMI/IRQ 自然被計入。

---

## 3. 改動範圍（依檔案）

| 檔案 | 改動 | 規模 |
|---|---|---|
| `src/bus/bus.rs` | trait 改 `&mut`、去 `RefCell`、新增 `tick()`、DMA 佇列 | 大（重寫核心 ~200 行） |
| `src/nes.rs` | `step()` 縮成 CPU + 中斷派發；刪除 4 段重複 NMI 區塊與補 tick 迴圈 | 大（−100 行） |
| `src/cpu/core.rs` | cycle 記帳、刪 `ppu_ticks_consumed`、`irq()` 簡化 | 中 |
| `src/cpu/addressing.rs` | `&dyn BusInterface` → `&mut dyn`；`read_operand` 同步 | 中（26 個呼叫點） |
| `src/cpu/ops/*.rs` | 簽名 `&dyn` → `&mut dyn`（其餘不動） | 機械性（42 處） |
| `src/cpu/ops/branch_jump.rs` | 刪 `tick_ppu_cycle()` 與 BRK 手動 tick | 中 |
| `src/apu/mod.rs` | `step` → `tick_one`、DMC request、`write_register` 去掉 bus 參數 | 中 |
| `src/apu/dmc.rs` | `fill_sample_buffer(bus)` → request/fill 兩段 | 小 |
| `src/ppu/*` | 幾乎不動（只新增 `nmi_line()` 查詢） | 小 |
| `src/timing.rs` | `DummyBus` 對齊新介面 | 小 |
| `src/main.rs` | `&mut` 對齊 | 小 |

總計約 **600–800 行淨變動**，其中一半是機械性簽名修改。

---

## 4. 分階段落地（每階段都要綠）

| 階段 | 內容 | 驗證 |
|---|---|---|
| **1a** | `MainBus` 去 `RefCell`、`read/write` 改 `&mut`、新增 `tick()`；`nes.rs` 暫時仍用迴圈叫人 tick | `cargo test` 全綠、golden 不變 |
| **1b** | `read/write` 內部自動 tick；CPU 用 `cpu_cycles()` 記帳；刪 `nes.rs` 補 tick 迴圈 | golden 可能變（預期）、blargg 觀察 |
| **1c** | APU 改 lockstep、DMC request、刪 `step_apu` | `apu_test`、聲音手測 |
| **1d** | DMA 改指令後執行、刪 `dma_stolen_cycles` | `1943`、`Mario` golden |
| **1e** | 刪 `op_brk`/`irq` 手動 tick、NMI 邊緣在 tick 時鎖存 | `cpu_interrupts`、`ppu_vbl_nmi` |

每個階段一個 commit，出問題可以單獨 revert。

---

## 5. 設計決策點（實作前要定案）

1. **tick 與存取的先後**：`read()` 是「先 tick 再讀」還是「先讀再 tick」？
   建議先 tick 再讀（存取發生在該 cycle 結束），再用 `ppu_vbl_nmi` 校準；
   若差一個 dot，改成先讀再 tick 即可。
2. **NMI 取樣點**：Phase 1 維持「指令結束才派發」，但邊緣在正確 cycle 鎖存；
   若 `cpu_interrupts` 仍不過，再在每次 `tick()` 後讓 CPU 檢查 latch（成本低）。
3. **DMA 期間的 OAM 寫入**：一次搬完或每 cycle 搬一個 byte（後者較準，成本相同）。
4. **APU sample 讀取時機**：DMC 要求 sample 的當下讀，還是下一個 cycle 讀（差 1 cycle）。

---

## 6. 風險

| 風險 | 對策 |
|---|---|
| 某條指令的存取次數 > 宣告 cycles → underflow | `debug_assert` + `all_instrs`（涵蓋全部 opcode） |
| 移除 `RefCell` 後 borrow 衝突 | 這是目的：讓衝突在編譯期出現，而不是 runtime panic |
| 畫面回歸 | `golden_frames.txt`（Mario 600 幀、1942、1943、MMC3 測試 ROM） |
| APU 時序改動導致聲音變化 | 保留 1c 前後的手動試聽；frame counter 單元測試仍在 |
| 效能下降 | 每 cycle 一次 `tick()`；但省掉大量 `RefCell` borrow 與補 tick 迴圈，預期持平或更快 |

---

## 7. 完成標準

- `cargo test`：19 個單元測試 + golden 全綠。
- `cargo test --release -- --ignored`：`all_instrs` / `official_only` / `instr_timing` 維持綠；
  `vbl_set_time`、`jitter`、`nmi_and_brk` 至少一項轉綠（未轉綠則進 Step C）。
- `Mario`、`1942`、`1943` 的 golden hash 只允許「有解釋的」變動。
- 手動試玩 5 分鐘：畫面、輸入、聲音正常。

---

## 8. 不在這一階段

- 完整 cycle-stepped CPU（每 cycle 一個狀態）——只有在 Phase 1 量測不足時才做。
- MMC3 A12 filter 與 IRQ 精確化。
- CHR 改為經 mapper 讀取（獨立工作，約 1 天）。
