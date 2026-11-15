# 🎮 Rust_Nes — NES 模擬器

以 Rust 實作的 Nintendo Entertainment System (NES / FC) 模擬器，具備完整的 CPU、PPU、APU 模擬與即時音訊輸出。

[![Rust](https://img.shields.io/badge/rust-edition%202024-orange.svg)](https://www.rust-lang.org/)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

## ✨ 功能特色

- **6502 CPU 核心** — 支援全部 56 個官方指令與非官方指令（含週期精確 page-cross 懲罰）
- **PPU 圖形處理** — Nametable 渲染、Attribute Table、CHR-ROM/RAM、硬體精靈（8×8 / 8×16）
- **APU 音頻模擬** — 5 聲道：Pulse 1/2、Triangle、Noise、DMC，含 Frame Counter、Envelope、Sweep
- **即時音訊輸出** — 透過 rodio 以 44100Hz 即時播放，含高通濾波器與低延遲佇列控制
- **輸入支援** — 完整 NES 手柄模擬（鍵盤映射）
- **Save / Load State** — 即時存檔與讀檔（F5 存檔 / F7 讀檔）
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
慣例指南，其餘研究筆記留在本機。上面 `src/` 的清單就是本 repo 的全部程式碼；
`tools/` 另外放了一個檢查 decoder 是否覆蓋全部 256 個 opcode 的小工具。

## 🧪 執行測試

所有測試（單元測試、blargg ROM 回歸、畫面 golden）都放在本機的 `testing/`，由 Cargo
feature `local_harness` 控制。因為 `testing/` 不發布，clone 下來的 repo 直接跑
`cargo test` 會是 0 tests（這是刻意設計：`src/` 不含測試程式碼）。要跑測試需要本機的
`testing/` 與該 feature：

```bash
# 單元測試（30 個：interrupt poll、schedule table、bus/PPU/APU/joypad/mapper）
cargo test --release --features local_harness

# ROM 回歸 + 畫面 golden（45 個）
cargo test --release --features local_harness -- --ignored
```

`testing/` 內的檔案（`*_tests.rs`、`rom_tests.rs`、`frame_tests.rs`、`rom_output.rs`、
`golden_frames.txt`）透過 `src/` 各模組與 `src/main.rs` 的 `#[path]` 掛進來，只在
`local_harness` feature 開啟時編譯；單元測試因此仍能存取模組私有欄位。同一個 feature
也會把 blargg 的 `$6000` 測試結果印到 stdout，方便手動跑單一 ROM：

```bash
# bash
HEADLESS_FRAMES=600 cargo run --release --features local_harness -- "test.nes"

# PowerShell
$env:HEADLESS_FRAMES=600; cargo run --release --features local_harness -- "test.nes"
```

目前本機 harness 全綠（30 個單元測試 + 45 個 ROM/golden 測試，0 失敗）：

| 套件 | 結果 |
|------|------|
| `all_instrs`、`official_only`、`instr_timing`、`cpu_dummy_writes_oam` | 通過 |
| `cpu_interrupts`（含 `2-nmi_and_brk`、`3-nmi_and_irq`） | 5/5 |
| `ppu_vbl_nmi` | 10/10 |
| `apu_test` | 8/8 |
| `sprite_hit_tests_2005.10.05` | 11/11 |
| `blargg_ppu_tests_2005.09.15b`（sprite_ram、power_up_palette、palette_ram、vbl_clear_time、vram_access） | 5/5 |
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
- 測試程式碼全部放在本機 `testing/`（不發布），clone 下來 `cargo test` 會是 0 tests。
- `docs/` 只發布架構指南，其餘設計筆記與除錯腳本留在本機。

## 📝 License

MIT License

## 🙏 參考資源

- [NESdev Wiki](https://www.nesdev.org/wiki/Nesdev_Wiki)
- [6502 Instruction Reference](https://www.masswerk.at/6502/6502_instruction_set.html)
- [NES APU Reference](https://www.nesdev.org/wiki/APU)