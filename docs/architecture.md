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
└─ DMC DMA 的 4-cycle stall（累積在 stolen cycles，在指令或中斷序列之後支付）
```

DMC DMA 本身在下一個 **讀取** 週期發生：`MainBus::read` 先 tick，若 APU 已經要求
sample byte，就重複該次讀取（1 或 2 次，依 cycle 奇偶）再抓 sample。重複的讀取有
完整副作用，所以 `$2007` 會多推進位址、`$4016` 會多移一位——這正是
`read_joy3` 與 `dmc_dma_during_read4` 在測的行為。

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

同一個 tick 也把 scanline counter 的時鐘來源交給卡匣：PPU 每個 dot 依實際 fetch 排程更新位址匯流排的
A12（BG pattern fetch 用 `$2000` bit 4、sprite pattern fetch 用 bit 3，NT/attribute
讀取為 0），CPU 的 `$2006`/`$2007` 存取則把 `v` 放到匯流排上；PPU 再套用三次 M2
cycle（9 dots）的低通濾波，把「每條 scanline 八次 sprite pattern fetch」收斂成一次
clock。`MainBus` 每個 CPU cycle 把累積的上升緣交給 `Mapper::tick_mapper_irq`，MMC3
以 revision B 的語意計數並把 IRQ 線閂住到 `$E000`/`$E001` 應答
（blargg `mmc3_irq_tests`、`mmc3_test`）。

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
Nametable mirroring 也由 mapper 決定：MMC1/MMC3/VRC6 可在執行中改變，`MainBus`
在 mapper 暫存器寫入後同步 `ppu.mirroring`（MMC1 control bits 0-1 也包含 one-screen
lower/upper）。

Battery save 由 `Cartridge` 統一管理：mapper 只透過 `battery_ram()` /
`battery_ram_mut()` 暴露自己的 PRG RAM（NROM/MMC1/MMC3/MMC5/FME-7），`Cartridge::from_file`
在開機時把 `<rom>.sav` 讀進那塊 RAM，平台層結束模擬時呼叫
`NesConsole::save_battery()` 寫回同名檔案。mapper 不需要知道檔案路徑，也不各自
處理存檔格式。

### 2.5 Reset 與暖開機

`NesConsole::reset()` 模擬的是硬體 `/RESET` 腳位，不是「重新載入 ROM」：

- **CPU**：`CPU::reset` 是完整的 7-cycle 序列——2 個 dummy read（PC、PC+1）、3 個
  堆疊位址的 *read*（`$0100|S`、`|S-1`、`|S-2`，只讀不寫）、S 減 3、設 I，最後從
  `$FFFC`/`$FFFD` 取 vector。A/X/Y、十進位旗標與堆疊內容都保留，遊戲因此能分辨冷
  開機與暖開機（`cpu_reset/registers` 期望 reset 後 A=$34 X=$56 Y=$78 P=$FF S=$0F）。
- **PPU**：`PPU::reset` 清 `ctrl`/`mask`/`status`/`v`/`t`/`x`/`w`/`oam_addr` 與 NMI
  線；VRAM、調色盤、OAM、CHR 與 frame/scanline 計數器不動（PPU 沒有被重啟，只被
  停止渲染）。
- **APU**：`APU::reset` 等於寫 `$4015`=`$00`——關閉所有通道、清 length counter、清
  frame/DMC IRQ；只寫暫存器（duty/period/halt/sweep/DMC 位址）保留，所以 halt 旗標
  還在的 triangle 會繼續發聲（`apu_reset` 的測試重點）。frame counter 由
  `APU::rearm_frame_counter` 在 reset 序列之後重播最後一次 `$4017` 值，並預跑 9 個
  cycle，和 power-up 一致（`apu_reset/4017_timing`）。
- **Mapper**：`Mapper::reset` 預設 no-op；只有 MMC1（shift register + control=$0C）
  與 MMC3（bank/IRQ 暫存器）覆寫。MMC1 的 control 決定 reset vector 落在哪個 bank，
  MMC3 的 IRQ 必須在遊戲重新初始化前關掉。
- **保留**：2 KB 工作 RAM、PRG RAM（含電池存檔）、VRAM/調色盤/OAM、已排入的音訊
  sample。重置不重讀 `.sav`，也不清 `last_ppu_frame` 以外的 frame 基準。

harness 端：ROM 在 `$6000` 寫 `0x81` 表示「請按 reset」，`run_until_result` 等 12 個
frame（>100 ms）後呼叫 `NesConsole::reset()`；reset 後 30 個 frame 內忽略殘留的
`$81`，避免同一次請求被按兩次。

### 2.6 平台層的畫面節奏（`main.rs`）

NES 的 frame 不是 60 Hz：89342 PPU dots ÷ 3 = 29780.67 CPU cycles，在 1.7897725 MHz
下是 16.639 ms（60.0988 Hz）。主迴圈一次 `step_frame()` 就是一個 NES frame，所以必須
自己限速：

- **音訊是時鐘**：`Sink::len()` 是「已排入的 per-frame 音訊 buffer 數」，目標 3 個
  （約 33 ms 緩衝＋正在播的那個）。每 frame 依佇列深度把 frame 週期加減 40 µs
  （±0.24 %，夾在 16.2–17.1 ms），吸收音訊裝置實際取樣率與 44100 Hz 的誤差。這
  不會改變遊戲速度，只有裝置時鐘差多少就補多少。
- **不要用音訊阻塞限速**：舊版 `AudioOut::append` 是 `while sink.len() > 2 { sleep(1ms) }`。
  `Sleep(1)` 在 Windows 是 ~1–16 ms 的粗粒度，而 `len` 只會在整個 16.6 ms buffer
  播完時才掉一格，所以 frame 間隔變成 19–22 ms 與 2 ms 交錯（實測 p50 = 19.4 ms）。
  現在 `append` 永不睡；佇列超過 6 個 buffer 才丟掉該 frame 的樣本（緊急閥）。
- **等待方式**：`wait_until()` 先 `sleep(remain - 2ms)` 再 spin 最後 2 ms；Windows 上
  另外呼叫 `timeBeginPeriod(1)`（`winmm`）把 timer 粒度降到 1 ms。純 sleep 的誤差
  （~0.9–1.7 ms）比可見的抖動還大。
- **落後就重同步**：若 `next_frame` 已經過期（拖視窗、斷點），不要把 deadline 往前
  追（會連播好幾格），直接重設成 `now + 1 ms`。
- 量測（Mario，20 秒）：p50 = 16.62 ms、p99 = 18.44 ms、最大 20.45 ms，音訊佇列
  穩定在 2–4 個 buffer，CPU 約 29 % 單核。修正前 p50 = 19.41 ms、p90 = 21.09 ms，
  且每幾格就出現 1.9 ms 的暴衝。
- minifb 0.24 沒有 vsync，present 只是 `InvalidateRect` + message loop；60.0 Hz 螢幕
  與 60.0988 Hz 的差異會讓 DWM 每 ~10 秒重複一格，這是更新率差，不是節奏 bug。

### 2.7 存檔格式（save state）

存檔是單一 binary blob，欄位全部經過 `src/state.rs` 的 `Writer`/`Reader`：呼叫端只
讀寫欄位，越界由 `Option` 回報——截斷的檔案不會 panic，也不會留下「載入一半」的狀態。

| 區段 | 內容 |
|------|------|
| `version`（1 byte） | 目前 `0x04` |
| CPU | `a, x, y, pc, sp, p` |
| PPU | `v/t/x/w`、`scanline/cycle/frame`、`odd_frame`、`read_buffer`、`oam_addr`、OAM 256 B、VRAM 2 KB、palette 32 B、CHR 8 KB |
| PPU 暫存器 | `ctrl, status, mask`（`0x04` 起） |
| RAM | 2 KB（`bus.rs::ram_data`） |
| mapper | `u32` 長度 + 各 mapper 的 payload |
| APU | `u32` 長度 + payload（109 bytes） |

版本歷史與相容處理：

| 版本 | 差異 | 載入行為 |
|------|------|----------|
| `0x01` | 只有 CPU + PPU | 讀完 PPU 就返回 |
| `0x02` | 多了 RAM 與 8192 bytes 已廢棄的 WRAM | `reader.skip(8192)` 跳過 |
| `0x03` | 移除廢棄 WRAM，加入 mapper / APU 區段 | 正常 |
| `0x04` | PPU 區段尾端加入 `ctrl/status/mask` | `version < 0x04` 不覆寫暫存器 |

`0x04` 不是可有可無的欄位：存檔不含 `PPUMASK` 時，重開模擬器再按 F7 會得到整幀
全黑——渲染從頭就是關的，要等遊戲自己重寫 `$2001` 才恢復。這個 bug 只有把狀態載入
**全新開機**的主機才測得出來（`testing/state_tests.rs::loaded_state_resumes_identically_on_a_fresh_console`）；
載回原本那台會通過，因為暫存器還在。

改格式的四條規矩：

1. 欄位順序或長度一改就 bump 版本，並保留舊版載入分支；舊存檔必須還能讀。
2. 同步更新 `testing/state_tests.rs::state_format_is_pinned` 的 (長度, FNV-1a)。它是唯一擋住
   「writer 與 reader 一起改歪」的檢查——往返測試對稱，但可能已經不是同一個格式。
3. mapper payload 沒有自己的版本號，動它就等於動外層格式。
4. 新欄位若只寫不讀，`save_load_save_is_byte_identical` 會立刻失敗。

## 3. Rust 寫作慣例

### 3.1 模組與可見性

- 一個檔案一個概念。模組入口用 `foo.rs` + `foo/`（現代 Rust 配置），不用 `mod.rs`
  ——`testing/mod.rs` 是本機測試的例外。
- 預設私有；只有跨模組需要的東西才 `pub`。crate 內共用優先用 `pub(crate)` 或
  `pub(super)`。
- 測試預設放 `testing/`：`src/main.rs` 用
  `#[cfg(all(test, feature = "local_harness"))] #[path = "../testing/mod.rs"] mod tests;`
  掛入 frame / interrupt-poll / schedule 測試；`src/nes.rs` 另外用 `#[path]` 掛
  `rom_tests.rs`，因為 ROM 回歸需要直接讀 `NesConsole` 的私有 bus 狀態（`$6000`
  PRG RAM、PPU OAM）。這樣產品 API 不需要任何測試專用 accessor。各模組再用同樣的
  `#[path]` 掛自己的 `*_tests.rs`，因此仍能存取私有欄位。
- `src/` 不含測試：存檔 round-trip、格式 pin、九個 mapper 的測試也都在 `testing/`。
  CI 的 checkout 沒有這個目錄，所以 CI 只驗編譯與 lint，行為驗證只能在本機跑。
- 平台 I/O 只允許出現在 `main.rs` 與 `audio.rs`；`nes.rs` 以下不碰視窗、鍵盤、
  音效裝置或檔案對話框。
- 只給 harness 用的診斷資料（例如 PPU 的 `split_hit`、`row_x`）用
  `#[cfg(all(test, feature = "local_harness"))]` 標記，發布版本完全不存在。

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
| 存檔 | `testing/state_tests.rs` | round-trip、格式 pin、九個 mapper 的對稱性。 |
| ROM 回歸 | `testing/rom_tests.rs` | 每個 blargg 子測項一個 `#[test]`，assert pass/status；路徑一律指向 `nes-test-roms-master/`，**缺檔視為失敗**（早期的「找不到就跳過」讓 15 個測試空跑了好幾個月）。 |
| 畫面回歸 | `testing/frame_tests.rs` | 對 `golden_frames.txt` 比 FNV-1a hash，抓渲染回歸。 |

```bash
cargo test --release --features local_harness                    # 55 個測試（全部在本機 harness）
cargo test --release --features local_harness -- --ignored      # 63 個測試，實際開機 134 片 ROM
```

修 bug 的順序永遠是：**先讓測試失敗 → 最小修正 → 全 suite + golden 綠**。`testing/`
是本機私有（不發布），所以 clone 下來的 repo `cargo test` 會是 0 tests——CI 只驗
編譯與 lint，不含任何行為測試。

## 6. 名詞與時間單位

- 1 CPU cycle = 3 PPU dots；1 frame = 89342 PPU dots（odd frame 少 1 dot）。
- 1 scanline = 341 dots；visible 240 lines + post-render 1 + vblank 20 + pre-render 1。
- Frame counter（4-step）：約 29830–29832 CPU cycles，依寫入 `$4017` 的奇偶 cycle 而異。
- Power-up 時 frame counter 先預跑 9 個 cycle，等效於第一個 instruction 前 9–12 個
  cycle 寫入 `$4017`=`$00`（`APU::power_on`，blargg `09.reset_timing`）。
- Reset 時 frame counter 重播最後一次寫入 `$4017` 的值，同樣預跑 9 個 cycle；
  `apu_reset/4017_timing` 量到的延遲是 9（該測試接受 6–12）。
- CPU reset 固定 7 個 cycle（2 dummy read + 3 stack read + 2 vector read），
  `cpu_reset/registers` 檢查 S 減 3、I 設起、堆疊內容不變。
- 寫入 `$4017` 後第一個序列的 half-frame clock 落在 14915/29831/37283（`+2` shift）。
  同一個 cycle 的 `$4015` 讀取要看 clock 前的長度計數；同一個 cycle 的長度 reload
  在計數器非 0 時要被忽略（`length_before_clock`，blargg `10`/`11`）。
- DMC bit rate 表在 `apu/constants.rs`；sample fetch stall 固定 4 cycles。
