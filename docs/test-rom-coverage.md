# 測試 ROM 覆蓋度盤點（`nes-test-roms-master/`）

這份筆記回答一個問題：**在現有 harness 之外，資料夾裡還有什麼值得測？**

結論先講：

1. ~~現有 harness 有一半以上是假綠燈~~ **已修正**：這份盤點發現 23 個斷言型 ROM
   測試裡只有 8 個真的跑，其餘 15 個因為路徑指向 repo 根目錄而印 `skipping` 後
   回傳成功。現在路徑全部改指 `nes-test-roms-master/`、缺檔視為失敗，一次執行
   實際開機 134 片 ROM、63 個測試全綠（約 33 秒）。
2. 全資料夾 290 個 `.nes` 掃過一輪（`diag_sweep_dir`，reset API 完成後重掃，每片上限
   1500 幀）：103 個 `$6000` 協定 PASS、4 個 FAIL（2 片 `sprdma` 的 DMA 仲裁 + 2 片
   MMC3 revision A）、6 個 TIMEOUT（四片指令大全、`ppu_vbl_nmi` 大全、`oam_stress`，
   需要 3000–9000 幀）、168 個不寫 `$6000`（只印螢幕或不輸出結果）、8 個 mapper
   不支援、1 個標頭壞掉。需要 reset 的 8 片現在由 harness 自己按下 reset，已全部 PASS。
3. 三塊「宣稱支援但時序不對」的部分：~~MMC3 IRQ~~（**已修正，15/18**，其餘 3 片是
   互斥的 revision A）、~~sprite overflow~~（**已修正，5/5**）、**DMA 衝突（7 個）** 仍待處理。
4. 「現在就會過、但沒進 harness」的套件已經全部接進去（§3.1 的套件現在都在
   `cargo test -- --ignored` 裡跑）。

---

## 1. 現況：假綠燈（已修正，保留記錄）

修正前，`testing/rom_tests.rs` 的 23 個斷言型 ROM 測試中只有 8 個真的啟動 ROM：

**有跑**：`cpu_interrupts_v2/rom_singles/2-nmi_and_brk`、`ppu_vbl_nmi/rom_singles/07`、
`apu_test/rom_singles/7-dmc_basics`、`blargg_apu_2005` 的 07/09/10/11、`apu_produces_audio`。

**只印 `skipping ... ROM not found` 就回傳成功**（15 個，現已修正）：

| 測試 | 找不到的路徑 | 檔案實際位置 |
|------|--------------|--------------|
| `instr_test_v5_passes` | `all_instrs.nes` | `instr_test-v5/all_instrs.nes` |
| `instr_test_v5_passes` | `official_only.nes` | `instr_test-v5/official_only.nes` |
| `instr_timing_passes` | `instr_timing.nes` | `instr_timing/instr_timing.nes` |
| `cpu_dummy_writes_oam_passes` | `cpu_dummy_writes_oam.nes` | `cpu_dummy_writes/cpu_dummy_writes_oam.nes` |
| `cpu_interrupts_passes` | `cpu_interrupts.nes` | `cpu_interrupts_v2/cpu_interrupts.nes` |
| `ppu_vbl_nmi_passes` | `ppu_vbl_nmi.nes` | `ppu_vbl_nmi/ppu_vbl_nmi.nes` |
| `apu_test_passes` | `apu_test.nes` | `apu_test/apu_test.nes` |
| `blargg_ppu_2005_passes` | `sprite_ram.nes` 等 5 個 | `blargg_ppu_tests_2005.09.15b/` |
| `sprite_hit_suite_passes` | `01.basics.nes` … 11 個 | `sprite_hit_tests_2005.10.05/` |

`assert_pass` / `assert_zero_page_result` 的 `if !path.is_file() { return; }` 讓「ROM 不存在」
和「測試通過」無法區分，回歸網因此一直是空的。

**已修正**：所有路徑改走 `rom_path()`（指向 `nes-test-roms-master/`），缺檔直接 panic，
每片 ROM 開機會印一行 `RAN|<path>|...` 供計數。現在一次 `--ignored` 執行是

```
test result: ok. 59 passed; 0 failed        # 127 行 RAN|，約 30 秒
```

而且上表那些測試修完之後都會過（見 §3.1），所以這一步是純賺。

---

## 2. 掃描工具

`testing/rom_tests.rs` 新增 `diag_sweep_dir`：遞迴跑 `SWEEP_DIR` 底下所有 `.nes`，用三種
方式判讀結果，一行一個 ROM 印出來。

```powershell
# 全資料夾（400 幀上限，約 4 分鐘）
$env:SWEEP_DIR='nes-test-roms-master'; $env:SWEEP_FRAMES='400'; $env:SWEEP_FILTER=''
cargo test --release --features local_harness diag_sweep_dir -- --ignored --nocapture

# 只掃某一套件
$env:SWEEP_DIR='nes-test-roms-master'; $env:SWEEP_FILTER='mmc3'; $env:SWEEP_FRAMES='1500'
cargo test --release --features local_harness diag_sweep_dir -- --ignored --nocapture
```

`SWEEP_DIR` 是 opt-in 開關：一般 `cargo test -- --ignored` 不會跑掃描（只印一行提示），
才不會讓回歸 suite 每次都多花十幾分鐘。

三種判讀方式：

1. **`$6000` 協定**（大部分 blargg 新測試）：`$6000`=status、`$6001-$6003`=`DE B0 61`、
   `$6004` 起 NUL 結尾文字。status `0x00` = pass、`0x80` = 執行中、`0x81` = 等 reset
   （harness 等 12 幀後呼叫 `NesConsole::reset()`，再讓 ROM 繼續跑）。
2. **零頁結果碼**（2005 年那批）：`$F0`/`$F8` = 1 表示 pass。
3. **螢幕文字**（MMC3 IRQ 測試、vbl_nmi_timing、sprite overflow 等只印在畫面上）：
   把 nametable 的 tile index 當 ASCII 讀出來（`nametable_text()`），選四個 nametable
   裡最像文字的那一頁。輸出長這樣：

   ```
   ROM|mmc3_irq_tests/1.Clocking.nes|4|NO_PROTOCOL|... screen=MMC3 IRQ COUNTER/FAILED #3
   ```

   注意：`dmc_tests`、`ppu_read_buffer` 等用自訂字型（tile index 不是 ASCII）的 ROM
   讀不出來，只能人工看畫面。

輸出格式 `ROM|<路徑>|<mapper>|<判定>|<內容>`，判定有
`PASS` / `FAIL(0xNN)` / `TIMEOUT` / `NO_PROTOCOL` / `NEEDS_RESET`（`$81` 按了 4 次
reset 仍未結束）/ `LOAD_ERR` / `PANIC`。

---

## 3. 全資料夾結果

> 掃描產物：`_research/sweep_after_reset.txt`（最新）、`sweep_after_sprite.txt`、
> `sweep_after_mmc3.txt`、`sweep_fast.txt`、`sweep_deep.txt`、`sweep_blank.txt`、
> `sweep_summary.csv`。

### 3.1 現在就會過的套件（已全部接進 harness）

| 套件 | ROM 數 | 結果 | 測什麼 |
|------|--------|------|--------|
| `instr_test-v5` | 16 singles + 2 suite | **全 PASS** | 官方+非官方指令、定址、堆疊、中斷 |
| `instr_test-v3` | 15 + 2 | **全 PASS** | 上一代版本，覆蓋面略不同 |
| `nes_instr_test` | 11 | **全 PASS** | 早期版本 |
| `instr_misc` | 4 + suite | **全 PASS** | dummy read、branch wrap、abs,X wrap |
| `instr_timing` | 2 + suite | **全 PASS** | 指令週期數（含 branch timing） |
| `cpu_interrupts_v2` | 5 + suite | **全 PASS** | CLI/PLP 延遲、NMI/BRK/IRQ、DMA 對 IRQ 的影響 |
| `cpu_exec_space` | 2 | **全 PASS** | 從 I/O 空間執行程式碼；PPU open bus |
| `cpu_dummy_writes` | 2 | **全 PASS** | RMW 指令的 dummy write（OAM/PPU） |
| `cpu_dummy_reads` | 1 | **PASS** | 單位元組指令的 dummy read |
| `apu_test` | 8 + suite | **全 PASS** | length/IRQ/DMC |
| `apu_mixer` | 4 | **全 PASS** | 各聲道混音非線性（需長期跑，>1000 幀） |
| `blargg_apu_2005.07.30` | 11 | **11 PASS**（`$F0`=1） | 目前 harness 只接 4 個 |
| `ppu_vbl_nmi` | 10 + suite | **全 PASS** | VBL/NMI 逐 dot 時序 |
| `vbl_nmi_timing` | 7 | **全 PASS** | 舊版 VBL/NMI 套件（畫面判讀） |
| `blargg_ppu_tests_2005` | 5 | **全 PASS**（`$F0`=1） | palette RAM、power-up palette、sprite RAM、VRAM |
| `sprite_hit_tests_2005` | 11 | **全 PASS** | sprite 0 hit 對齊/翻轉/邊界/時序 |
| `sprite_overflow_tests` | 5 | **全 PASS** | overflow flag 的時序與 buggy byte 掃描 |
| `oam_read` | 1 | **PASS** | `$2004` 讀回目前位址 |
| `ppu_open_bus` | 1 | **PASS** | PPU open bus/decay |
| `read_joy3` | 4 | 3 PASS（`thorough_test`、`count_errors`×2） | 手把時序 + DMC 搶讀取 |
| `branch_timing_tests` | 3 | **全 PASS** | branch 週期 |
| `apu_reset` | 6 | **全 PASS** | reset 後 `$4015`/`$4017`/IRQ/長度計數器行為 |
| `cpu_reset` | 2 | **全 PASS** | 暖開機偵測：A/X/Y/堆疊保留、RAM 不清 |
| `cpu_timing_test6` | 1 | PASS（官方指令） | 指令時序總表 |
| `blargg_nes_cpu_test5/official` | 1 | PASS | 官方指令 |
| `ppu_vbl_nmi`（完整） | 1 | 10/10 PASS（9000 幀） | — |

`cpu_timing_test6` 與 `blargg_nes_cpu_test5/cpu.nes` 需要按 A/B 才會測非官方指令；
`cpu.nes` 會停在 `06-abs_xy Error 125`，和 README 的「非官方暫時不支援」一致。

### 3.2 現在會失敗的套件（真 bug）

| 套件 | ROM 數 | 結果 | 根因 |
|------|--------|------|------|
| `mmc3_irq_tests` | 6 | **5 PASS**（`5.MMC3_rev_A` 敗） | rev A/B 互斥，核心實作 rev B |
| `mmc3_test` | 6 | **5 PASS**（`6-MMC6` 敗） | 同上 |
| `mmc3_test_2` | 6 | **5 PASS**（`6-MMC3_alt` 敗） | 同上（與 `mmc3_test` 重複，未接入 harness） |
| `sprdma_and_dmc_dma` | 2 | **2/2 FAIL** | OAM/DMC DMA 仲裁還不到 sub-cycle 精確 |
| `dmc_dma_during_read4` | 5 | 2 PASS、1 自檢 Failed、2 資訊性 CRC 不符 | DMC DMA 的請求相位與硬體差幾 cycle |
| `ppu_read_buffer` | 1 | 卡住、不回報 | 待查（`$2007` read buffer） |
| `blargg_nes_cpu_test5/cpu.nes` | 1 | FAIL（Error 125） | 非官方指令（已知限制） |

剩下的失敗訊息很明確：

```
sprdma_and_dmc_dma.nes          T+ Clocks 00 3765 01 3764 02 3765 ...
mmc3_test/6-MMC6.nes            Failed #3  IRQ shouldn't occur when reloading after counter normally reaches 0
```

`6-MMC6` 與 `6-MMC3_alt` 的失敗不是 bug：那兩片測的是 MMC3 **revision A** 的語意
（只有 `$C001` 造成的 reload 才檢查歸零），而 `5-MMC3` 測 revision B（每次歸零都
檢查）。readme 明說「at most only one will pass on a particular emulator」。

### 3.3 需要新能力才測得動

| 套件 | ROM 數 | 缺什麼 |
|------|--------|--------|
| `pal_apu_tests` | 10 | PAL 時序（3 個過、7 個失敗，這是預期） |
| `nes15-1.0.0/nes15-PAL` | 1 | PAL |
| `mmc5test`、`mmc5test_v2`、`exram` | 3 | MMC5 完整度（`$5120-$512B` CHR bank、ExRAM、scanline IRQ） |
| `PaddleTest3`、`vaus-test` | 2 | paddle/VAUS 周邊 |

### 3.4 不建議投入（或只能人工判讀）

- `other/` 下的遊戲與 demo（約 90 個）：用 golden frame / 手動試玩，不是 pass/fail 測試。
- `tvpassfail/tv.nes`：測 composite 訊號的 chroma/luma crosstalk，需要 NTSC 訊號模擬。
- `volume_tests`：要人耳或比對錄音（`recordings/`）。
- `full_palette`、`window5`、`240pee`、`blargg_litewall`、`scanline`、`scrolltest`：
  視覺測試，需要截圖/像素比對，沒有 pass/fail。
- `oam_stress`：吃開機時 PPU/CPU 相位，參考模擬器（`test_roms.xml`）自己也是 fail。
- `dmc_tests`（4）：自訂字型輸出，`$6000` 也沒有，只能人工看畫面。
- mapper 11/22/28/34 的 8 個 ROM：`src/rom.rs:180` 直接 `panic!("不支援 Mapper")`。

> `test_roms.xml` 的 `testresult` 欄位是某個參考模擬器的結果，不是硬體真理
> （例如它把 `oam_read` 記成 fail、MMC3 rev A/B 也記成 fail）。判斷「該不該修」要
> 以 ROM 內附 readme 的「tested on a real NES」為準。

---

## 4. 失敗根因與修法

### 4.1 MMC3 IRQ：A12 是假的（✅ 已修正，15/18）

修正前的 `compute_a12_rising()` 只用「BG pattern table 是不是 `$1000`」推測 A12，並
假設 `257..=320` 的 sprite fetch 一定是 high，所以 `1.Clocking`、`3.A12_clocking`、
`4-scanline_timing`、`5-MMC3` 全部失敗。

修正內容（三個檔案）：

1. **PPU 逐 dot 驅動真正的 A12**（`src/ppu.rs`、`src/ppu/frame.rs`）：依 fetch 排程
   更新（BG pattern = `$2000` bit 4、sprite pattern = bit 3、NT/attribute = 0），
   `$2006` 第二次寫入與 `$2007` 讀寫把 `v` 放上匯流排，而且 `$2007` 的 +1/+32
   增量本身也會改變 A12（`3.A12_clocking` 第 5/6 項就是測這個）。
2. **三次 M2 cycle 的低通濾波**：A12 必須連續 low 9 個 dot 才接受上升緣。這讓
   「每條 scanline 八次 sprite pattern fetch」只算一次 clock，也讓 BG=$1000 時
   scanline 邊界那個 8-dot 間隙不算數（`4-scanline_timing` 第 12 項）。
   實測：$2000=$08 每 frame 241 次、$10 每次 line 兩次但只接受一次。
3. **MMC3 counter 改成 revision B 語意並閂住 IRQ 線**（`src/rom/mappers/mapper4.rs`、
   `src/bus.rs`）：counter 為 0 或 `$C001` 要求 reload 時重載，否則遞減；歸零就
   設 pending，直到 `$E000`/`$E001` 應答才放掉（新增 `Mapper::irq_line`，讓
   acknowledge 在同一個指令內生效）。

結果：`mmc3_irq_tests` 5/6、`mmc3_test` 5/6、`mmc3_test_2` 5/6，其中三片失敗都是
revision A 的 ROM（與 revision B 互斥）。Kirby's Adventure 每 frame 仍有一次 IRQ
（scanline 174），MMC3 遊戲的 mid-frame split 不受影響。

### 4.2 Sprite overflow：只有「超過 8 個」的簡化版（✅ 已修正，5/5）

修正前的評估器在 `cycle == 256` 才把旗標寫入 `$2002`，而且掃到第 64 個 sprite 就
**繞回 0**，也沒有實作硬體那個「八格滿了之後 byte phase 不再歸零」的 bug，所以
`2.Details #9`（繞回造成誤判）、`3.Timing #5`（時序太晚）、`4.Obscure`、`5.Emulator`
全部失敗。

修正內容（`src/ppu/sprite.rs` 改寫 `oam_evaluate`）：

1. **一個 byte 兩個 dot**：65-256 只跑讀取，寫入併進讀取（只有 overflow 時序依賴
   dot）。in-range 的 sprite 花 8 dots，off-screen 的只花 2 dots（跳過）。
2. **旗標在第 9 個 in-range 的 Y 被讀到的那個 dot 就閂住**，不再等 cycle 256。
   實測：9 個 sprite 在 scanline 1 時是 line 0 的 dot 129（blargg `3.Timing` 的
   window 是 1-2 個 CPU cycle）。
3. **buggy byte 掃描**：八格滿了之後，每個 byte 都當 Y 比對，而且 phase 不歸零
   （`addr += 4`、`phase += 1`），所以第 10 個 sprite 的第 2 個 byte、第 11 個的第 3 個
   byte…會被當成 Y（`4.Obscure` 全部 7 項）。
4. **掃到 OAM 尾端就停**，不繞回；`2.Details #9` 就是靠這個修正的。

`testing/sprite_tests.rs` 另外加了 4 個單元測試把 dot 位置釘住（dot 129 / 131 / 239
與「不繞回」），避免日後改評估器時又漂掉。

### 4.3 DMA 衝突：Sprite DMA 與 DMC DMA 撞在一起（部分修正）

這一輪修掉的三個真 bug：

1. **`$4016` 的 strobe 只送給 1P**：硬體上 strobe 是兩支手把共用的；原本寫 `$4016`
   只 latch joypad1，joypad2 的移位暫存器永遠不會重置。連帶 `$4017` 不該去動手把
   （它只屬於 APU frame counter）。
2. **手把讀超過 8 次後回傳 0**：硬體在移位暫存器空了之後資料線浮高、回傳 1。
   `read_joy3` 的 `read_joy_fast` 就是靠這個行為終止迴圈與補償 DMC 搶讀取的。
3. **DMC DMA 的 halt 時機**：以前是「請求當下直接抓 byte、在指令結尾補 4 cycles」，
   現在改成在 CPU 的**下一個讀取週期** halt，並重複該次讀取 1-2 次（依 cycle 奇偶），
   所以 `$2007`、`$4015`、`$4016` 的副作用會再發生——`read_joy3` 的衝突計數從 0 變成
   55/1000（`count_errors`）與 16/1000（`count_errors_fast`），和硬體一樣會偶發衝突，
   而 `read_joy` 的補償仍然讓 `thorough_test` 通過。

還沒對上的部分（`sprdma_and_dmc_dma` 兩片、`dma_2007_read`、`dma_4016_read`、
`double_2007_read`）：這些 ROM 把 DMC 的請求時間校準到 **±1-2 cycle** 的窗格，
要求 DMA 請求剛好落在 `lda $2007`/`lda $4016` 那一個讀取週期上。我們的請求週期
（3424 cycles）正確，但相位與硬體差幾個 cycle，5 次迭代的掃描窗格因此全部錯過；
`dma_4016_read` 現在會跑完並回報 `Failed`（CRC `FBF7C7B1`，硬體是 `F0AB808C`），
不再是無窮迴圈。`sprdma` 的輸出呈現 3765/3764 交替，代表 OAM DMA 與 DMC DMA
同時要求 bus 時的仲裁（誰先、halt 幾個 cycle）還沒建模。

要繼續推進需要一個 sub-cycle 的 DMA engine：DMC 請求相位、halt/dummy/get 的
3-4 cycle 變化、以及 OAM DMA 被 DMC DMA 搶佔後的重對齊。

### 4.4 `ppu_read_buffer`：卡住

157 個子測試的 `$2007` read buffer 測試，4000 幀（約 66 秒，ROM 自稱 20 秒）仍沒有
寫出 `$6000`，畫面停在字元表。值得單獨查：如果它在某個子測試無限迴圈，代表
`$2007` 讀 buffer 的行為和硬體差太多。

---

## 5. 建議順序

1. **P0 — 修 harness 路徑** ✅ 完成：路徑全部改指 `nes-test-roms-master/`，缺檔 fail，
   每片 ROM 印 `RAN|`。現在一次 `--ignored` 是 63 個測試 / 134 片 ROM / 約 33 秒。
2. **P1 — 接進 §3.1 的套件** ✅ 完成：`instr_test-v5`、`instr_test-v3`、
   `nes_instr_test`、`instr_misc`、`instr_timing`、`cpu_interrupts_v2`、
   `cpu_exec_space`、`cpu_dummy_writes`、`cpu_dummy_reads`、`apu_mixer`、
   `blargg_apu_2005`（11 個）、`vbl_nmi_timing`、`sprite_hit`、`blargg_ppu_tests`、
   `ppu_open_bus`、`oam_read`、`read_joy3`、`branch_timing_tests`、`cpu_timing_test6`
   全部進 harness，而且都過。
3. **P2 — MMC3 IRQ** ✅ 完成：PPU 逐 dot 的 A12 + 9-dot 濾波 + revision B 的 counter，
   `mmc3_irq_tests`／`mmc3_test` 各 5/6 通過（3 片 revision A 由設計互斥），已接入
   harness。Kirby's Adventure 每 frame 一次 IRQ（scanline 174）。
4. **P3 — Sprite overflow** ✅ 完成：改寫 `ppu/sprite.rs` 的評估器（2 dots/byte、
   第 9 個 sprite 的 dot 就閂旗標、buggy byte 掃描、不繞回），5/5 通過並接入 harness，
   另外 4 個單元測試釘住 dot 位置。
5. **P4 — DMA 衝突**（部分完成）：修好 `$4016` 共用 strobe、手把空暫存器回 1、
   DMC DMA 在下一個讀取週期 halt 並重複讀取（見 §4.3）；`sprdma_and_dmc_dma` 與 3 片
   `dmc_dma_during_read4` 需要 sub-cycle DMA engine 才能對上硬體窗格。
6. **P5 — 新能力**：~~reset~~ ✅ 完成（`NesConsole::reset`、CPU 7-cycle 序列、PPU/APU
   歸零、APU 重播 `$4017`、MMC1/MMC3 mapper reset、harness 的 `$81` 處理），8 片
   `apu_reset`/`cpu_reset` 全過並接入 harness。剩下 MMC5 完整度 → 3 個；PAL / paddle
   視需求。`ppu_read_buffer` 卡住的原因也還沒查（§4.4）。

---

## 6. 附錄：怎麼重跑這份盤點

```powershell
# 全資料夾（每個 ROM 上限 400 幀，會提早結束）
$env:SWEEP_DIR='nes-test-roms-master'; $env:SWEEP_FRAMES='400'; $env:SWEEP_FILTER=''
cargo test --release --features local_harness diag_sweep_dir -- --ignored --nocapture

# 需要較長預算的套件
$env:SWEEP_DIR='nes-test-roms-master'; $env:SWEEP_FILTER='apu_mixer'; $env:SWEEP_FRAMES='1500'
```

`diag_sweep_dir` 用 `catch_unwind` 包住每個 ROM，mapper panic 只會記成 `PANIC`，
不會中斷整輪；`NO_PROTOCOL` 那行會附上螢幕文字與零頁 `$F0`/`$F8`，方便人工判讀。
