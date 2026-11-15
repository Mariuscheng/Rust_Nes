# Rust_Nes 架構與寫作慣例

這份文件說明模擬器的分層、一個 CPU 指令如何流過各元件，以及本專案遵循的 Rust
寫作慣例。目標是：新功能放在對的檔案、用對的方式寫，而且改動有辦法驗證。

## 1. 分層與模組地圖

程式是單一 binary crate，由上而下分層。**上層可以依賴下層，下層不知道上層存在**。

| 層 | 檔案 | 責任 |
|----|------|------|
| 平台層 | `main.rs` | 命令列、視窗（minifb）、鍵盤、音訊裝置（rodio）、DPI 設定。唯一碰平台 I/O 的地方。 |
| 平台層 | `audio.rs` | rodio 輸出佇列與取樣率轉換。 |
| 主機層 | `nes.rs` | `NesConsole`：組裝 CPU/PPU/APU/匯流排、`step`/`step_frame`、存讀檔。 |
| 匯流排層 | `bus.rs` | `MainBus`：記憶體映射、PPU 暫存器 I/O、OAM DMA、DMC sample fetch、NMI/IRQ 取樣。 |
| 處理器層 | `cpu/` | 6502：解碼、定址、指令實作、中斷入口。 |
| 圖形層 | `ppu/` | PPU：暫存器、VRAM/OAM/調色盤、逐 dot 渲染管線。 |
| 音訊層 | `apu/` | APU：frame counter、五個聲道、混音。 |
| 卡匣層 | `rom/` | iNES 解析、`Mapper` trait、九種 mapper 實作。 |
| 周邊 | `ram.rs`、`joypad.rs`、`cartridge.rs` | 2KB 工作 RAM（含鏡像）、手把移位暫存器、卡匣黑盒封裝。 |
| 常數 | `constants.rs`、`apu/constants.rs` | 中斷向量、畫面尺寸、frame counter/DMC 週期表。 |

`cartridge.rs` 是唯一的「卡匣邊界」：`NesConsole` 只拿到 `Cartridge`，不直接讀
`.nes` 檔；mapper 的差異全部收在 `rom/` 裡。

## 2. 執行模型

### 2.1 一個 CPU 指令的生命週期

```
NesConsole::step()
├─ CPU::step(&mut bus)                     // 執行一個指令，回傳 cycle 數
│   ├─ decoder：opcode → instruction + addressing mode + 基本 cycle
│   ├─ ops/*：依序做 bus.read / bus.write
│   └─ 剩餘的內部 cycle 用 bus.tick() 補齊
├─ OAM DMA（$4014 先排隊，指令結束後執行 513/514 cycles）
├─ 中斷派送（NMI 邊緣優先，其次 IRQ）
└─ DMC DMA 造成的 4-cycle stall（在指令或中斷序列之後支付）
```

`CPU::step` 是 **instruction-stepped**：handler 的 bus 存取順序就是硬體 cycle
順序，宣告的 cycle 數與實際存取數的差額用「no bus access 的 cycle」補齊。這個模型
足以通過目前所有 timing 測試，也讓 handler 讀起來像資料流而不是狀態機。

### 2.2 匯流排：CPU cycle = 3 PPU dots

`MainBus::read`/`write` 會先呼叫 `BusInterface::tick`（`tick_with_dots(3)`），
所以每一次 CPU 匯流排存取都精確推進 3 個 PPU dot 與 1 個 APU cycle：

```
MainBus::tick_with_dots(3)
├─ 記錄 IRQ 線狀態（給 penultimate-cycle 取樣用）
├─ PPU::tick(3)      // 逐 dot：scanline/cycle、VBL、NMI 線、BG/sprite pipeline
├─ APU::tick_one()   // frame counter + 聲道；DMC 要 sample 時向 bus 要 byte
└─ 更新 NMI 線並 latch rising edge
```

PPU 是唯一以 dot 為單位的元件；CPU/APU 都以 3 dots 為一步。**不要**在 PPU 以外
的地方假設 dot 級時間。

### 2.3 中斷模型

- **NMI 是邊緣觸發**：PPU 每次 raise 都設 `nmi_rise`，bus 在每個 CPU cycle 把它
  latch 成 `nmi_edge_seen`；CPU 在指令的 penultimate cycle 取樣，指令結束後才派送。
  這樣 `$2002` 讀取可以取消「同一個 CPU cycle 內」的邊緣，但取消不了已 latch 的。
- **IRQ 是準位觸發**：bus 保存 tick 前的 IRQ 線狀態（`irq_before_tick`），CPU 用
  `poll_delay` 決定要取樣哪一個 cycle；`poll_iflag` 保存「poll 當下看到的 I 旗標」，
  所以 CLI/SEI 會延遲一個指令，PLP/RTI/BRK 不會。
- **BRK 自己 poll NMI**（`cpu/ops/branch_jump.rs`），並在 cycle 5 決定是否被 NMI
  hijack；IRQ 進入序列在 push status 後做同樣的取樣（`cpu/interrupt.rs`），讓
  NMI 可以在前 4 個 cycle 搶走 vector。

時序類的註解一定要寫「依據哪個測試 ROM 的哪一列」，例如
`ppu_vbl_nmi 07-nmi_on_timing`、`cpu_interrupts 3-nmi_and_irq`。

### 2.4 卡匣與 mapper

`rom.rs` 解析 iNES/NES 2.0 標頭，`rom/mappers.rs` 用 `MapperVariant`
enum + `delegate!` 做靜態分派（避免 `Box<dyn Mapper>` 的虛擬呼叫與 lifetime）。
`Mapper` trait 的預設實作就是「硬體上合理的 no-op」，所以新 mapper 只需覆寫有差的
方法。CHR bank 切換透過 `take_chr_dirty_blocks()` 只重抄受影響的 1 KiB block。

## 3. Rust 寫作慣例

### 3.1 模組與可見性

- 一個檔案一個概念；`mod.rs` 只列出子模組與 `pub use`，不放邏輯。
- 預設私有；只有跨模組需要的東西才 `pub`。crate 內共用優先用 `pub(crate)` 或
  `pub(super)`。
- `src/` **不含測試**。測試全部在 `testing/`，由原模組用
  `#[cfg(all(test, feature = "local_harness"))] #[path = "..."] mod tests;` 掛回，
  因此仍能存取私有欄位。
- 平台 I/O 只允許出現在 `main.rs` 與 `audio.rs`；`nes.rs` 以下不碰視窗、鍵盤、
  音效裝置或檔案對話框。
- 測試支援用的 accessor（例如 `NesConsole::ppu()`、`cpu_ram_byte`）用
  `#[cfg(all(test, feature = "local_harness"))]` 標記，不進產品路徑。

### 3.2 命名

- 型別 `CamelCase`、函式/變數 `snake_case`、常數 `SCREAMING_SNAKE_CASE`。
- 硬體縮寫保持大寫：`CPU`、`PPU`、`APU`、`DMA`、`OAM`、`CHR`、`PRG`、`NMI`、
  `IRQ`、`VBL`；暫存器寫成 `$2002` 這種形式。
- 函式用動詞開頭：`read_prg`、`write_prg`、`raise_nmi`、`take_nmi_edge`、
  `step_frame`。boolean 用 `is_`/`has_`/`can_`。
- 時序推進叫 `step`/`tick`；匯流排存取叫 `read`/`write`；「取走並清除」叫
  `take_*`。同一個動詞在不同模組代表同一種語意。

### 3.3 註解與文件

- 註解寫 **為什麼** 與硬體依據，不重述程式碼。
- 每個檔案開頭用 `//!` 說明模組責任；不要用 `// src/xxx.rs` 這種檔名註解。
- 不明顯的常數要說明來源（例如 `29831` 是 NTSC frame counter 的 CPU cycle 數）。
- 暫時性除錯輸出不要進 commit；`println!`/`eprintln!` 只用於產品輸出
  （ROM 載入資訊、headless 摘要）或本機 harness。

### 3.4 狀態與常數

- 硬體常數集中在 `constants.rs` 與 `apu/constants.rs`，不要在函式內散落 magic
  number；只在一處使用的小常數可以留在附近，但要給名字。
- PPU 的 `v`/`t`/`x`/`w`、APU 的 frame counter 等狀態機欄位，命名要與 NESdev
  文件一致，讓對照規格書時不用翻譯。
- 可變狀態盡量縮小作用域：能用區域變數就不要變成欄位；欄位只在「跨 cycle 或跨
  模組需要」時才存在。

### 3.5 錯誤處理

- 檔案/ROM I/O 用 `Result`；`main.rs` 負責把錯誤轉成清楚的 panic 訊息。
- CPU/PPU/APU 熱路徑不 panic；未實作的硬體行為用 no-op 並在文件標註。
- `debug_assert!` 只放不變量（cycle 預算、索引範圍），不要放使用者輸入檢查。

### 3.6 格式化與 lint

- 提交前跑 `cargo fmt`。
- 目標是 `cargo clippy --all-targets -- -D warnings` 與 `cargo build --release`
  零 warning。
- 不要用 `#[allow(dead_code)]` 蓋掉沒人用的程式碼；真的沒用就刪掉（`tools/verify_opcodes.py`
  會檢查 CPU 目錄）。

### 3.7 Commit

- 英文、祈使句、說明「為什麼」而不是「改了什麼」。
- 一次 commit 一個主題；修正時序的 commit 要附上測項名稱與前後對照。

## 4. 常見修改流程

### 新增或修正 CPU 指令

1. `cpu/decoder.rs`：opcode → instruction、addressing mode、基本 cycle。
2. `cpu/ops/`：用 `bus.read`/`bus.write` 的順序表達 cycle；需要額外 cycle 時用
   `bus.tick()` 放在硬體對應的位置，不要全部丟到最後。
3. `testing/schedule_tests.rs`：跑 `regenerate_schedule_table` 更新
   `testing/schedule_table.rs`，再跑 `schedule_table_matches_handlers` 確認。
4. 跑 `cargo test --release --features local_harness` 與 `instr_timing`/`all_instrs`。

### 新增 mapper

1. `rom/mappers/<name>.rs` 實作 `Mapper`（只覆寫有差的方法）。
2. `rom/mappers.rs`：加進 `MapperVariant` 與 `delegate!`。
3. `rom.rs`：iNES mapper id → variant。
4. 加一個 `testing/<name>_tests.rs` 或在 `rom_tests.rs` 掛 ROM 回歸。

### 修 PPU timing

1. 先在 `testing/rom_tests.rs` 用單一 ROM 重現（必要時加 `diag_*` 印出 dot/cycle）。
2. 改 `ppu/frame.rs`（逐 dot 管線）或 `ppu.rs`（暫存器行為），一次只改一個相位。
3. 驗證順序：該 ROM → 整個 blargg PPU suite → golden frames。
4. 畫面預期改變時才 `REGEN_GOLDEN=1`，並在 commit 說明為什麼。

### 修 APU timing

- `apu/mod.rs` 管 frame counter 與混音；各聲道在 `pulse.rs`、`triangle.rs`、
  `noise.rs`、`dmc.rs`。
- DMC 的 sample fetch 由 `MainBus::clock_apu` 提供，4-cycle stall 由 `CPU::step`
  支付；改動時要同時看 `apu_test` 的 DMC 子測項。

## 5. 測試策略

| 層級 | 位置 | 說明 |
|------|------|------|
| 單元測試 | `testing/<module>_tests.rs` | 測私有邏輯：decoder、mapper、APU tick、中斷 poll、schedule table。 |
| ROM 回歸 | `testing/rom_tests.rs` | 每個 blargg 子測項一個 `#[test]`，assert pass/status；需要 `nes-test-roms-master/`。 |
| 畫面回歸 | `testing/frame_tests.rs` | 對 `golden_frames.txt` 比 FNV-1a hash，抓渲染回歸。 |

```bash
cargo test --release --features local_harness                    # 30 個單元測試
cargo test --release --features local_harness -- --ignored      # 45 個 ROM/golden
```

修 bug 的順序永遠是：**先讓測試失敗 → 最小修正 → 全 suite + golden 綠**。`testing/`
是本機私有（不發布），所以 clone 下來的 repo `cargo test` 會是 0 tests。

## 6. 名詞與時間單位

- 1 CPU cycle = 3 PPU dots；1 frame = 89342 PPU dots（odd frame 少 1 dot）。
- 1 scanline = 341 dots；visible 240 lines + post-render 1 + vblank 20 + pre-render 1。
- Frame counter（4-step）：約 29830–29832 CPU cycles，依寫入 `$4017` 的奇偶 cycle 而異。
- DMC bit rate 表在 `apu/constants.rs`；sample fetch stall 固定 4 cycles。
