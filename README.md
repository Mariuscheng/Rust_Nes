# 🎮 Rust_Nes — NES 模擬器

以 Rust 實作的 Nintendo Entertainment System (NES / FC) 模擬器，具備完整的 CPU、PPU、APU 模擬與即時音訊輸出。

[![verify](https://github.com/Mariuscheng/Rust_Nes/actions/workflows/verify.yml/badge.svg)](https://github.com/Mariuscheng/Rust_Nes/actions/workflows/verify.yml)
[![Rust](https://img.shields.io/badge/rust-edition%202024-orange.svg)](https://www.rust-lang.org/)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

## ✨ 功能特色

- **6502 CPU 核心** — 支援全部 56 個官方指令與非官方指令（含週期精確 page-cross 懲罰）
- **PPU 圖形處理** — Nametable 渲染、Attribute Table、CHR-ROM/RAM、硬體精靈（8×8 / 8×16）
- **APU 音頻模擬** — 5 聲道：Pulse 1/2、Triangle、Noise、DMC，含 Frame Counter、Envelope、Sweep
- **即時音訊輸出** — 透過 rodio 以 44100Hz 即時播放，含高通濾波器與低延遲佇列控制
- **穩定畫面節奏** — 以 NES 實際的 60.0988 Hz 更新，靠音訊佇列深度回饋微調 frame 週期（睡眠＋短暫 spin），不再用音訊阻塞限速；避免 frame 間隔在 2 ms 與 20 ms 之間跳動的卡頓
- **輸入支援** — 完整 NES 手柄模擬（鍵盤映射）
- **Save / Load State** — 即時存檔與讀檔（F5 存檔 / F7 讀檔）
- **Reset** — 硬體重置（F2）：CPU 走 7 個 cycle 的重置序列、PPU/APU 暫存器歸零，但工作 RAM、VRAM、調色盤、OAM 與磁碟/卡匣的 PRG RAM 全部保留（暖開機偵測、多合卡選單、遊戲內重置都靠這個）
- **Battery Save** — 卡匣電池存檔（NROM/MMC1/MMC3/MMC5/FME-7）：啟動時讀取與 ROM 同名的 `.sav`，每 30 秒自動存檔、關閉模擬器時再寫回
- **Headless 模式** — 支援無視窗執行以進行自動化測試
- **Mapper 支援** — NROM(0)、MMC1(1)、UxROM(2)、CNROM(3)、MMC3(4)、MMC5(5)、AxROM(7)、VRC6(24/26)、FME-7(69)

## 🎮 按鍵映射

| NES 按鍵 | 鍵盤按鍵 |
|----------|----------|
| A        | Z        |
| B        | X        |
| Start    | Enter    |
| Select   | Right Shift |
| ↑ ↓ ← →  | 方向鍵    |

其他熱鍵：**F5** 存檔、**F7** 讀檔、**F2** 硬體重置（與主機面板的 reset 鍵相同）。

## 🚀 快速開始

### 環境需求

- **Rust 1.88+**（stable，`edition = "2024"`）
- Windows / macOS / Linux

### 安裝與執行

```bash
# 複製專案
git clone https://github.com/Mariuscheng/Rust_Nes.git
cd Rust_Nes

# 編譯（Release 模式以獲得最佳效能）
cargo build --release

# 執行（預設載入 rom.nes）
cargo run --release

# 指定 ROM 檔案
cargo run --release -- "path/to/rom.nes"

# Headless 模式（無視窗，執行 N 幀後退出）
HEADLESS_FRAMES=180 cargo run --release -- "path/to/rom.nes"
```

## 📁 專案結構

```
src/
├── main.rs           # 進入點（視窗 / headless），唯一的平台層
├── nes.rs            # NesConsole：組裝各元件、step/step_frame、存讀檔
├── state.rs          # 存檔格式的 Writer/Reader（唯一的序列化 codec）
├── bus.rs            # 主匯流排、記憶體映射、DMA、NMI/IRQ 線
├── cpu.rs            # CPU 模組入口
│   ├── core.rs       # 6502 核心（暫存器、指令分派、cycle 預算）
│   ├── decoder.rs    # 指令解碼表（opcode → instruction + addressing mode）
│   ├── addressing.rs # 定址模式（含 page-cross 偵測）
│   ├── interrupt.rs  # NMI/IRQ/BRK 入口與 NMI hijack
│   └── ops/          # 指令實作（ALU、分支、堆疊、非官方指令…）
├── ppu.rs            # PPU 核心（暫存器、VRAM、OAM、調色盤）
│   ├── frame.rs      # 逐 dot 渲染管線
│   ├── registers.rs  # PPU 暫存器位元定義
│   └── sprite.rs     # OAM 評估與精靈初始化
├── apu.rs            # APU 入口（frame counter、混音）
│   └── pulse/triangle/noise/dmc/sweep/envelope/constants.rs
├── audio.rs          # 即時音訊輸出（rodio）
├── rom.rs            # iNES 解析 + mapper 分派
│   └── mappers/      # 9 種 mapper 實作
├── cartridge.rs      # Cartridge 黑盒封裝
├── ram.rs            # 內部工作 RAM（2KB，含鏡像）
└── joypad.rs         # NES 手柄（strobe latch + 序列讀出）
```

本機測試 harness 與設計筆記**不包含在 repo 內**（`testing/` 在 `.gitignore` 裡，只存在
本機）；`docs/` 只發布 [`docs/architecture.md`](docs/architecture.md) 這份架構與寫作
慣例指南，其餘研究筆記留在本機。上面 `src/` 的清單就是本 repo 的全部程式碼。本機的
`scripts/`（`verify.ps1` 驗證腳本）與 `tools/`（opcode 覆蓋檢查）同樣不上傳。

## 🧪 執行測試

所有測試（單元測試、存檔 round-trip、blargg ROM 回歸、畫面 golden）都放在本機的
`testing/`，由 Cargo feature `local_harness` 控制。因為 `testing/` 不發布，clone 下來的
repo 直接跑 `cargo test` 會是 0 tests（這是刻意設計：`src/` 不含測試程式碼，CI 只驗
編譯與 lint）。要跑測試需要本機的 `testing/` 與該 feature：

```bash
# 55 個測試（51 harness：interrupt poll、schedule table、battery save、reset、mirroring、
# bus/PPU/APU/joypad/mapper、sprite 評估；4 公開 API：存檔與 mapper round-trip）
cargo test --release --features local_harness

# ROM 回歸 + 畫面 golden（63 個測試，實際啟動 134 片 ROM，約 35 秒）
cargo test --release --features local_harness -- --ignored
```

`testing/` 內的檔案（`mod.rs`、`*_tests.rs`、`rom_tests.rs`、`frame_tests.rs`、
`golden_frames.txt`）透過 `src/main.rs` 與 `src/` 各模組的 `#[path]` 掛進來，只在
`local_harness` feature 開啟時編譯；單元測試因此仍能存取模組私有欄位。`src/main.rs`
只有一行測試模組宣告，產品路徑完全不含測試程式碼。

ROM 回歸的每條路徑都指向 `nes-test-roms-master/`，而且**缺檔會讓測試失敗**（不再印
`skipping` 後回傳成功）。每次執行以 `RAN|` 開頭的行就是實際開機的 ROM 數，可用
`--nocapture` 數出來。`diag_sweep_dir` 是 opt-in 的覆蓋度掃描：沒設 `SWEEP_DIR` 時它
只印一行提示就結束，不會拖慢回歸 suite。

目前本機 harness 全綠（51 個單元測試 + 63 個 ROM/golden 測試，0 失敗）：

| 套件 | 結果 |
|------|------|
| 指令：`instr_test-v5`（16 子項 + 2 大全）、`instr_test-v3`（15 + 2）、`nes_instr_test`（11）、`instr_misc`（4 + 大全） | 全數通過 |
| 時序：`instr_timing`（含 branch）、`branch_timing_tests`（3）、`cpu_timing_test6` | 全數通過 |
| 中斷：`cpu_interrupts_v2`（5 子項 + 大全） | 6/6 |
| CPU 匯流排：`cpu_dummy_writes`（2）、`cpu_dummy_reads`、`cpu_exec_space`（2） | 全數通過 |
| PPU：`ppu_vbl_nmi`（10 + 大全）、`vbl_nmi_timing`（7）、`blargg_ppu_tests_2005`（5）、`sprite_hit_tests_2005`（11）、`sprite_overflow_tests`（5）、`ppu_open_bus`、`oam_read` | 全數通過 |
| Mapper：`mmc3_irq_tests`（5）、`mmc3_test`（5） | 15/18（3 片 revision A 由設計互斥） |
| APU：`apu_test`（8 + 大全）、`blargg_apu_2005.07.30`（11）、`apu_mixer`（4） | 全數通過 |
| 重置：`apu_reset`（6）、`cpu_reset`（2） | 8/8（harness 收到 `$81` 後按下 reset） |
| 手把：`read_joy3`（`count_errors`、`count_errors_fast`、`thorough_test`） | 3/3 |
| 畫面 golden（Mario、1942、1943） | 8 個 frame hash 全部一致 |

畫面 hash 回歸的重新生成方式：

```bash
# PowerShell
$env:REGEN_GOLDEN=1; cargo test --release --features local_harness -- --ignored golden_frames
# bash
REGEN_GOLDEN=1 cargo test --release --features local_harness -- --ignored golden_frames
```

## 🏗️ 架構說明

模組分層、資料流與 Rust 寫作慣例的完整說明見
[`docs/architecture.md`](docs/architecture.md)。以下只列各元件的重點。

### CPU (6502)
- 暫存器：A, X, Y, PC, SP, P (status)
- 支援所有官方 opcode 與非官方 opcode（SLO, RLA, SRE, RRA, SAX, LAX, DCP, ISC, ANC, ALR, ARR, AXS, KIL）
- 週期精確 page-cross 偵測（用於 DMC DMA 同步）

### PPU (Picture Processing Unit)
- 256×240 軟體渲染
- 4 個 Nametable + Attribute Table
- 64 個硬體精靈（透過 OAM）
- 支援垂直/水平鏡像
- 完整 NMI 中斷

### APU (Audio Processing Unit)
- Pulse 1/2：可程式化 duty、sweep、envelope
- Triangle：線性計數器
- Noise：可程式化 LFSR
- DMC：delta modulation 取樣播放
- Frame Counter：4-step / 5-step 模式
- 整數取樣演算法（無浮點誤差）
- 高通濾波器（HPF, 20Hz cutoff）

## 🗺️ 支援的 Mapper

| Mapper | 狀態 |
|--------|------|
| NROM (0) | ✅ |
| MMC1 (1) | ✅ |
| UxROM (2) | ✅ |
| CNROM (3) | ✅ |
| MMC3 (4) | ✅ IRQ 已實作；CHR bank 在 mapper 寫入後立即同步，且只重抄變動的 1 KiB block（支援 mid-frame 切換） |
| MMC5 (5) | ⚠️ 部分：PRG/CHR 是線性映射；`$5120-$512B` CHR bank、ExRAM、scanline IRQ 尚未實作 |
| AxROM (7) | ✅ |
| VRC6 (24/26) | ✅ |
| FME-7 (69) | ✅ |

CHR 同步的取捨：mapper 寫入後立即同步到 PPU 的 8 KiB CHR 副本，且只重抄變動的
1 KiB block，所以 mid-frame 切 bank 會在下一次 tile fetch 生效（Kirby's Adventure
即以此驗證）。

## ⚠️ 已知限制

- MMC5 僅部分實作（見上方 mapper 表）：`$5120-$512B` CHR bank、ExRAM 與 scanline IRQ
  尚未完成。
- MMC3 的 A12 與 IRQ counter 已照硬體實作（PPU 逐 dot 的位址匯流排、`$2006`/`$2007`
  存取、三次 M2 cycle 的低通濾波、revision B 的 reload 語意），但 **revision A 與
  revision B 對「counter 歸零時機」的定義互斥**，所以 `5.MMC3_rev_A`、`6-MMC6`、
  `6-MMC3_alt` 這三片一定會失敗；本核心選擇 revision B 並通過另外 15 片。
- DMC DMA 會在 CPU 的下一個讀取週期 halt 並重複該次讀取（`$2007`/`$4015`/`$4016`
  的副作用會再發生一次），所以 `read_joy3` 的 DMC 衝突計數與硬體一致；但 DMA 的
  cycle 相位與 OAM/DMC 仲裁仍未 sub-cycle 精確，`sprdma_and_dmc_dma`、
  `dma_2007_read`、`dma_4016_read`、`double_2007_read` 尚未對上硬體輸出。
- Reset 已實作（`NesConsole::reset`）：CPU 7-cycle 重置序列、PPU/APU 暫存器歸零、APU 重播最後一次 `$4017` 值。mapper 端只有 MMC1（shift register / control）與 MMC3（bank/IRQ 暫存器）會重設，其餘 mapper 假設遊戲在 reset handler 內自己重寫 bank；重置也不清 VRAM/OAM/調色盤/RAM。
- 沒有 vsync：minifb 0.24 不提供，畫面由 DWM 以螢幕更新率合成。NES 是 60.0988 Hz、60 Hz 螢幕是 60.000 Hz，所以每約 10 秒會有一格被重複顯示（所有以音訊為時鐘、未做動態取樣率補償的模擬器都一樣）；拖動視窗時主迴圈被 Windows 的 modal message loop 擋住，音訊會短暫斷掉，放開後節奏會自動補回。
- 只支援 NTSC；`pal_apu_tests`、`nes15-PAL` 這類 PAL 時序的 ROM 不適用。
- 測試程式碼全部放在本機 `testing/`（不發布），clone 下來 `cargo test` 會是 0 tests。
- `docs/` 只發布架構指南，其餘設計筆記與除錯腳本留在本機。

## 📝 License

本專案以 MIT License 授權，全文見 [LICENSE](LICENSE)。Copyright (c) 2026 Cheng Marius.

## 🙏 參考資源

- [NESdev Wiki](https://www.nesdev.org/wiki/Nesdev_Wiki)
- [6502 Instruction Reference](https://www.masswerk.at/6502/6502_instruction_set.html)
- [NES APU Reference](https://www.nesdev.org/wiki/APU)