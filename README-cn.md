# rust_gomoku

`rust_gomoku` 是 Python reference 项目 `pygomoku` classic 主线的 Rust 重构，并支持可选的连珠禁手规则。当前主线目标是同时守住两种规则的正确性，在可验证的前提下继续降低搜索耗时并提高棋力。

## 界面截图

Windows x64 程序和已签名的 Android ARM64 APK / AAB 可从[最新 Release](https://github.com/chubbyk-uu/rust-gomoku/releases/latest)下载。

### Windows / 桌面 GUI

Windows 程序启动后打开本地网页界面。可与引擎对弈或进行双人对局，在右侧选择无禁手或连珠规则、切换专业模式（NNUE `Mix9svq` 每步限时 `500 ms`–`5000 ms`，默认 `1000 ms`）或休闲模式（四档经典固定深度），并可开关多线程搜索。

![桌面 GUI：连珠人机对局，右侧提供规则、休闲/专业 NNUE 限时与多线程设置](docs/screenshots/desktop-gui.png)

### Android

手机界面集中展示棋盘、当前回合，以及悔棋和新局操作。内置打包 `Mix9svq` NNUE 权重，默认启用专业限时模式（`500 ms` / `1000 ms` / `2500 ms` / `5000 ms`，默认 `1000 ms`，统一 `tt_bits = 20` / 32 MiB TT），同时提供休闲四档难度（入门、普通、高级、大师）以及多线程搜索开关。

<p>
  <img src="docs/screenshots/android-portrait.png" alt="Android 竖屏界面：连珠专业 1s 对局进行中" width="320">
  <img src="docs/screenshots/android-difficulty.png" alt="Android 新局设置面板：支持休闲/专业 NNUE 模式与四档限时分段切换" width="320">
</p>

横屏时，操作区排列在棋盘旁边：

![Android 横屏对局界面](docs/screenshots/android-landscape.png)

截图说明：桌面图截自 WSL 中实际运行的 GUI，与 Windows 版本使用同一网页。Android 图片使用应用内置的 HTML/CSS/JavaScript 和 Rust 控制器产生的真实对局状态，在浏览器中预览截取，不含 Android 系统栏，非手机实机截图。

## 当前状态

已完成：

- 15x15 无禁手与连珠规则状态机、zobrist、配置、pattern、eval cache、movegen、ordering、TT、alpha-beta root search。
- 连珠黑方长连、四四、递归真三三禁手及五连优先语义；默认规则仍为无禁手。
- 规则感知的 root VCF（NNUE 模式下自动启用 16 层深搜 + 64K Zobrist VCF TT）与 `df-pn` VCT 求解器（`src/threats/dfpn.rs`），配备四方向增量 `Pattern4` 位棋盘/LUT 战术加速（连珠密集 VCT6 提速 `5.5x`–`8.6x`）。
- 纯 Rust 增量 `Mix9svq` NNUE 评估（兼容 Rapfi CC0 `.bin.lz4` 权重），支持 Value + Policy 候选排序、x86_64 AVX2 / VEX AVX-VNNI SIMD、64B 对齐 4-way TT（缓存 `static_eval`）、对齐 Rapfi 的 `policyReduction` / `futilityMoveCount` / `PVS+LMR`、`O(1)` 必胜快判与单防点直通、叶子 QVCF 及异步 Lazy SMP 多线程 NNUE 搜索。
- 经典模式多线程 ABDADA 搜索（`--threads N` / `INFO thread_num N`），支持 `1/64` 层细粒度 TT 深度编码与按单/多线程模式隔离的双平面共享置换表。
- Gomocup stdin/stdout 引擎入口、本地 Web GUI（`gomoku_gui`）以及内置打包 `Mix9svq` 权重的已签名 Android ARM64 APK / AAB（两端共享 Rust `GameController`）。
- Rust/reference 差分脚手架（经典 `13/13` 基准检查全绿）、双引擎对战裁判、连珠 oracle/压测工具和同局面 benchmark。
- SlowRenju、Rapfi 等外部参考引擎按需从本地 checkout 使用；仓库不提交轻量对手或完整 Python reference。

可选诊断/实验：

- `overlap_vct_alphabeta`：VCF miss 后重叠 VCT 与 alphabeta 的实验开关。
- `root_profile`：root candidate 计时诊断。
- TT generation 观测：Gomocup trace 可输出跨手 TT best-move hint 新旧比例，不影响 TT cutoff、replacement 或排序策略。

GUI 入口仅在单线程（`--threads 1`）手动对局时默认开启 `overlap_vct_alphabeta`；多线程模式下保持关闭以避免额外 TT 快照与核心争抢。Gomocup、diff、case probe 和库默认均保持关闭。
历史性能实验和止损结论记录在 `docs/perf-log.md`，README 不重复维护失败方案清单。
Android 架构、手机界面、JNI 边界、分阶段验证门槛与发版记录见 `docs/android-app-design.md`。

## 默认参数

主要默认值集中在 `src/config.rs`。

| 参数 | 默认值 |
|---|---:|
| 固定搜索深度 | `8` |
| 固定 root width | `40` |
| 时间控制最大深度 | `25` |
| 时间控制最大 width | `40` |
| `root_vcf_depth` | `8`（经典）；NNUE 模式下未显式覆盖时自动加深至 `16`（配 64K Zobrist VCF TT） |
| `opponent_vcf_depth` | `7` |
| `vct_verify_opponent_vcf_depth` | `4` |
| `vcf_multi_reply` | `true` |
| `root_vct_depth` | `6` |
| `vct_use_dfpn` | `true`（默认 `df-pn` VCT，上限 `(root_vct_depth * 3).clamp(2, 22)` ply；设 `0` 可回退旧版 IDDFS VCT） |
| `vct_strict_and_memo_key` | `true`（仅旧版 IDDFS VCT 生效） |
| `use_nnue` | 库/Gomocup 默认关闭（传 `--nnue-dir` / `INFO nnue_dir` / `RUST_GOMOKU_NNUE_DIR` 时开启）；桌面 GUI 与 Android 专业模式在权重可用时默认开启 |
| TT bucket bits | 桌面 GUI 与 Android 统一默认 `20`（32 MiB）；库/Gomocup 默认 1–3 线程或深度 `< 5` 为 `20`、4+ 线程且深度 `>= 5` 为 `21`，或由 `INFO max_memory` 自动计算 |
| `threads` | 库/Gomocup 默认 `1`；桌面 GUI 与 Android 在 `>= 4` 核默认 `4`，`< 4` 核默认 `min(2, cores)`（支持 `--threads N` / `set_threads` 回退单线程） |
| `compute_vcf` / `compute_vct` | VCF 开启；VCT 在经典与单线程 NNUE 开启，多线程 NNUE（`threads > 1` 且未开 overlap）默认跳过串行 root df-pn VCT（可用 `INFO compute_vct` 显式覆盖） |
| `overlap_vct_alphabeta` | 关闭；GUI 仅在 `threads == 1` 时开启 |
| `nonroot_vcf` | 关闭 |
| `static_board` | 开启 |
| `dynamic_board_margin` | `4` |

桌面 GUI 和 Android 应用支持两种模式（两端统一使用 `tt_bits = 20` / 32 MiB 置换表）：

1. **专业模式（默认）**——NNUE（`Mix9svq`）限时搜索，提供四档每步时间上限（`500 ms`、`1000 ms`、`2500 ms`、`5000 ms`，默认 `1000 ms`），在 `w40` 下迭代加深至最多 `d25` 并开启 VCF（单线程开启 root VCT，多线程 NNUE 默认跳过串行 root df-pn VCT）。桌面端若未找到 NNUE 权重则自动回退经典评估。

2. **休闲模式**——提供四档共享经典评估固定深度难度：

| 难度 | 搜索参数 | VCF / VCT |
|---|---:|---:|
| 入门 (`beginner`) | `d2 / w10` | 关闭 |
| 普通 (`normal`，休闲默认) | `d4 / w20` | 关闭 |
| 高级 (`senior`) | `d6 / w30` | 开启 VCF，关闭 VCT |
| 大师 (`master`) | `d8 / w40` | 开启 VCF，开启 VCT（`root_vct_depth=6`） |

与 Python reference 严格差分或复现实验时，通常显式使用 `depth=6,width=20,root_vct_depth=4`。

连珠模式实现轮流落子和黑方禁手；RIF、Yamaguchi、Soosorv、Swap2 等连珠开局协议尚未实现。

## 坐标约定

外部坐标统一为 `(x, y) = (列, 行)`，与 Gomocup、GUI 和 case JSON 保持一致。内部棋盘矩阵按 Rust 数组习惯存为 `grid[row][col]`，`Move` 编码为 `row * BOARD_SIZE + col`。代码里优先使用 `xy_to_move` / `move_to_xy`；在矩阵语境下可使用别名 `rc_to_move` / `move_to_rc` 避免误读。

## Reference 路径

完整 Python reference 默认放在本机外部目录：

```bash
~/python_ws/pygomoku
```

也可以显式指定：

```bash
export PYGOMOKU_REF_ROOT=~/python_ws/pygomoku
```

脚本查找顺序是 `--ref-root`、`PYGOMOKU_REF_ROOT`、`~/python_ws/pygomoku`。

## 常用命令

构建和测试：

```bash
cargo build --release
cargo test --quiet
python3 scripts/run_diff.py --jobs 10
```

Android debug/release 验证与构建：

```bash
cd android
./gradlew test lint assembleDebug
./gradlew test lint assembleRelease bundleRelease
```

Release 签名默认读取 `~/.android/rust-gomoku-release.properties`；本地签名文件格式和覆盖环境变量详见 `docs/android-app-design.md`。

Gomocup smoke：

```bash
printf 'START 15\nBEGIN\nEND\n' | cargo run --quiet --bin gomocup_engine -- --depth 2 --width 8
printf 'START 15\nINFO rule 4\nBEGIN\nEND\n' | cargo run --quiet --bin gomocup_engine -- --depth 2 --width 8
```

启动 GUI：

```bash
cargo run --release --bin gomoku_gui
```

GUI 会自动用默认浏览器打开 `http://127.0.0.1:18080`；如需关闭自动打开行为，可加 `--no-open-browser`；可用 `--nnue-dir PATH` 指定权重目录（默认自动探测同级 `nnue/`、当前目录 `nnue/`、`RUST_GOMOKU_NNUE_DIR` 或本地开发目录），或加 `--no-nnue` 强制使用经典评估。GUI 支持人机对战（执黑/执白）和双人对弈（两人在同一棋盘轮流落子、不启动引擎）两种对战模式，无禁手或连珠规则、专业（NNUE 每步限时 `500 ms`–`5000 ms`，默认 `1000 ms`）和休闲（四档经典固定深度）两种模式、多线程搜索开关、悔棋、重新开局、异步思考、手数显示、胜负弹窗和状态面板；快捷键 `U` 悔棋，`R` 重新开局。双人模式下会隐藏仅人机模式有意义的控件（执棋、模式、休闲难度/专业限时、多线程）。空棋盘时切换对战模式、规则或休闲/专业模式会立即生效；落子后（`move_count > 0`）切换会显示待生效提示，并在点击开局按钮开始新局时生效。连珠黑方回合会用红叉标出禁手点，点击禁手点会被拒绝且不会落子（两种模式都如此）。休闲难度、专业限时和多线程开关可在对局中途引擎空闲时随时调整，不重置当前棋局，只影响下一次引擎思考。

Gomocup engine：

```bash
cargo run --release --bin gomocup_engine
target/release/gomocup_engine --depth 8 --width 40
target/release/gomocup_engine --threads 4 --depth 8 --width 40
target/release/gomocup_engine --depth 6 --width 20 --root-profile
target/release/gomocup_engine --tt-bits 22
```

常用 `INFO`：

- `INFO rule 0|4` 或 `INFO rule freestyle|renju`（仅空棋盘时允许）
- `INFO timeout_turn N`
- `INFO time_left N`
- `INFO max_memory N`（字节；未显式指定 `tt_bits` 时按内存上限自动计算 TT 容量）
- `INFO max_node N`
- `INFO thread_num N`（ABDADA 多线程搜索；`1` 保持单线程确定性）
- `INFO compute_vcf 0|1`
- `INFO root_vcf_depth N`
- `INFO opponent_vcf_depth N`
- `INFO vct_verify_opponent_vcf_depth N`
- `INFO vcf_multi_reply 0|1`
- `INFO compute_vct 0|1`
- `INFO root_vct_depth N`
- `INFO vct_use_dfpn 0|1`
- `INFO use_nnue 0|1`
- `INFO nnue_dir PATH`（指向 Rapfi CC0 `mix9svq*.bin.lz4` 权重目录；亦支持 CLI `--nnue-dir PATH` 或环境变量 `RUST_GOMOKU_NNUE_DIR`）
- `INFO vct_strict_and_memo_key 0|1`
- `INFO nonroot_vcf 0|1`
- `INFO overlap_vct_alphabeta 0|1`
- `INFO tt_bits N`
- `INFO static 0|1`
- `INFO dynamic_board_margin N`
- `INFO root_profile 0|1`

VCF、VCT 与 alpha-beta 在一手棋内共用截止时间和累计节点预算。同时给出
`timeout_turn` 与 `time_left` 时取较小值；限时搜索下 root VCT 子预算上限为整手预算的 30%，确保未算杀时留出 alpha-beta 搜索时间。搜索采用协作式中止，完成当前棋盘
操作、选择合法后备手及协议初始化仍可能产生额外耗时。限额搜索报告的节点数
包含战术节点，无限额搜索沿用 alpha-beta 节点数。

运行 `scripts/run_diff.py --profile all --check-baselines` 可同时验证 TT 小数
深度及整手预算修正后 Rust 和 Python 各自的预期值。默认差分仍报告所有不同。
用 `scripts/bench_match_cases.py` 测 Renju 前缀时可指定 `--rule renju`。

## 差分和对战

单 case 差分：

```bash
cargo run --quiet --bin diff_probe -- --case cases/diff/root_center_11.json
cargo run --quiet --bin diff_probe -- --case cases/diff/root_center_11_d6_w5.json --root-profile
```

Rust 默认参数对 Python reference 9 开局双边 18 局：

```bash
cargo build --release --bin gomocup_engine
python3 scripts/run_engine_match.py \
  --opening-set 9 \
  --jobs 18 \
  --output /tmp/rust_vs_reference_9_openings.json
```

通用 engine 对战：

将 `BASELINE_DIR` 指向保存基线版 `gomocup_engine` 和 `case_probe` 发布构建的目录。

```bash
cargo build --release --bin gomocup_engine --bin case_probe
BASELINE_DIR=/path/to/baseline/bin
python3 scripts/run_gomocup_match.py \
  --case-file cases/match/smoke_quick.jsonl \
  --engine-a-side both \
  --engine-a-command 'target/release/gomocup_engine --depth 6 --width 20' \
  --engine-b-command "\"$BASELINE_DIR/gomocup_engine\" --depth 6 --width 20" \
  --output /tmp/engine_version_match.json
```

同局面一手 benchmark：

```bash
python3 scripts/bench_match_cases.py \
  --case-file cases/match/standard.jsonl \
  --baseline-binary "$BASELINE_DIR/case_probe" \
  --candidate-binary target/release/case_probe \
  --jobs 1 \
  --output /tmp/engine_version_bench.json
```

`run_gomocup_match.py` 用于真实对战和棋力评估；`bench_match_cases.py` 用于同一批前缀局面的一手搜索对照。判断优化是否真的提速，应先看同局面 benchmark，再看对战胜率和长尾耗时。

两侧使用相同引擎和选项时，对战脚本会标记为自对弈冒烟，benchmark 会标记为重复性检查。

连珠对战使用 `--rule renju`。对战脚本会给双方设置 Gomocup 规则 `4`，
并使用规则感知裁判拒绝黑方禁手：

```bash
cargo build --release --bin gomocup_engine --bin renju_referee
python3 scripts/run_gomocup_match.py \
  --rule renju \
  --case-file cases/renju/strength_100_prefixes.jsonl \
  --engine-a-side both \
  --engine-a-command 'target/release/gomocup_engine --depth 8 --width 40' \
  --engine-b-command "\"$BASELINE_DIR/gomocup_engine\" --depth 8 --width 40"
```

完整连珠设计、验证证据、SlowRenju 对照约定和剩余工作见
`docs/renju-forbidden-design.md`。

如需观察真实跨手 TT，可给 `run_gomocup_match.py` 加 `--reuse-engine-state`。默认对战脚本每手 `RESTART`，更适合公平复现；`--reuse-engine-state` 会保留 engine/searcher 状态，并把每手 `MESSAGE tt_generation current=... old=...` 汇总到 JSON 的 `tt_generation` 字段。

## 目录

```text
src/                 Rust engine、NNUE、Gomocup、GUI、diff/case probe
android/             Android ARM64 应用（Kotlin/WebView）与 JNI rust_bridge crate
cases/diff/          root 差分 case
cases/match/         对战和 benchmark 前缀局面
cases/renju/         禁手、战术、候选诊断和棋力前缀 case
data/static/         从 reference 提取的静态矩阵
docs/                设计文档、性能日志与截图
scripts/             差分、对战、benchmark、case 抽取脚本
tests/               Rust 自动测试
```

## 当前重点

1. 守住无禁手与连珠的规则正确性和经典回归门槛（禁手 oracle/压测与 `13/13` Python reference 基准差分）。
2. 基于已锁定的 `Mix9svq` NNUE 基线（同权重 32 MiB TT、连珠 `1s/move` 对 Rapfi 1T `35.5%` / 4T `34.0%`），继续推进搜索效率、防守候选生成、Aspiration Window 与多线程扩展。
3. 为 `Mix9svq` 补充 ARM64 NEON SIMD 内核，进一步提升 Android 端搜索深度与响应速度。
4. 所有性能与棋力实验统一在 `docs/perf-log.md` 同时记录正确性、耗时、nodes 与公平对战结果。

## 许可证

本项目使用 GNU General Public License v3.0 或后续版本授权。详见 [LICENSE](LICENSE)。
