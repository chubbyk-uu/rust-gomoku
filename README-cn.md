# rust_gomoku

`rust_gomoku` 是 Python reference 项目 `pygomoku` classic 主线的 Rust 重构，并支持可选的连珠禁手规则。当前主线目标是同时守住两种规则的正确性，在可验证的前提下继续降低搜索耗时并提高棋力。

## 当前状态

已完成：

- 15x15 无禁手与连珠规则状态机、zobrist、配置、pattern、eval cache、movegen、ordering、TT、alpha-beta root search。
- 连珠黑方长连、四四、递归真三三禁手及五连优先语义；默认规则仍为无禁手。
- 规则感知的 root VCF、root-only VCT 触发/验证/trace。
- Gomocup stdin/stdout 引擎入口和本地 Web GUI。
- Rust/reference 差分、Rust/reference 对战、base/fast 对战和同局面 benchmark 脚手架。
- 连珠 oracle、定向密集压测、一维穷举、候选诊断、性能测试和规则感知对局裁判工具。
- 非 root 候选排序成本优化：保持原排序 key 不变，减少 `getmi` 和候选复制开销。
- fast profile 默认开启第三版 history/killer ordering：只在静态排序同组内调整安静着法顺序，base 不受影响。
- SlowRenju、Rapfi 等外部参考引擎按需从本地 checkout 使用；仓库不再提交轻量对手或完整 Python reference。

可选诊断/实验：

- `overlap_vct_alphabeta`：VCF miss 后重叠 VCT 与 alphabeta 的实验开关。
- `root_profile`：root candidate 计时诊断。
- TT generation 观测：Gomocup trace 可输出跨手 TT best-move hint 新旧比例，不影响 TT cutoff、replacement 或排序策略。

GUI 入口为了降低手动对局体感等待，默认单独开启 `overlap_vct_alphabeta`；Gomocup、diff、case probe 和库默认仍保持关闭。
历史性能实验和止损结论记录在 `docs/perf-log.md`，README 不重复维护失败方案清单。
Android 架构、手机界面、JNI 边界和分阶段验证门槛记录在
`docs/android-app-design.md`。Android 应用已可构建 ARM64 APK，并通过
JNI 使用共享 Rust 控制器。

## 默认参数

主要默认值集中在 `src/config.rs`。

| 参数 | 默认值 |
|---|---:|
| 固定搜索深度 | `8` |
| 固定 root width | `40` |
| 时间控制最大深度 | `25` |
| 时间控制最大 width | `40` |
| `root_vcf_depth` | `8` |
| `opponent_vcf_depth` | `7` |
| `vct_verify_opponent_vcf_depth` | `4` |
| `vcf_multi_reply` | `true` |
| `root_vct_depth` | `6` |
| `vct_strict_and_memo_key` | `true` |
| TT bucket bits | `20` |
| `compute_vcf` / `compute_vct` | 开启 |
| `overlap_vct_alphabeta` | 关闭，GUI 入口单独开启 |
| `fast_history_ordering` | base 关闭，fast 开启 |
| `nonroot_vcf` | 关闭 |
| `static_board` | 开启 |
| `dynamic_board_margin` | `4` |

桌面 GUI 和 Android 应用提供五档共享难度：

| 难度 | 搜索参数 | VCF / VCT |
|---|---:|---:|
| 入门 | `d1 / w10` | 关闭 |
| 初级 | `d2 / w10` | 关闭 |
| 中级 | `d4 / w20` | 关闭 |
| 高级 | `d6 / w30` | 开启 |
| 大师 | `d8 / w40` | 开启 |

中级是桌面 GUI 和 Android 应用的默认难度。难度与 Base/Fast 互相独立：
难度控制深度、宽度和战术搜索，Base/Fast 控制排序行为。

与 Python reference 严格差分或复现实验时，通常显式使用 `depth=6,width=20,root_vct_depth=4`。

默认 profile 是 `base`，用于守住 classic 语义和 reference 差分。`--profile fast` 当前会默认开启 `fast_history_ordering`；如需对照可用 `--no-fast-history-ordering` 关闭。

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

Gomocup smoke：

```bash
printf 'START 15\nBEGIN\nEND\n' | cargo run --quiet --bin gomocup_engine -- --depth 2 --width 8
printf 'START 15\nINFO rule 4\nBEGIN\nEND\n' | cargo run --quiet --bin gomocup_engine -- --depth 2 --width 8
```

启动 GUI：

```bash
cargo run --release --bin gomoku_gui
```

GUI 会自动用默认浏览器打开 `http://127.0.0.1:18080`；如需关闭自动打开行为，可加 `--no-open-browser`。GUI 支持人机对战（执黑/执白）和双人对弈（两人在同一棋盘轮流落子、不启动引擎）两种对战模式，无禁手或连珠规则、五档难度、悔棋、重新开局、Base/Fast 模式切换、异步思考、手数显示、胜负弹窗和状态面板；快捷键 `U` 悔棋，`R` 重新开局。双人模式下会隐藏仅人机模式有意义的控件（执棋、Base/Fast、难度）。规则选择只在新局开始时生效；连珠黑方回合会用红叉标出禁手点，点击禁手点会被拒绝且不会落子（两种模式都如此）。难度和 Base/Fast 只在引擎未思考时允许切换，不重置当前棋局，只影响下一次引擎思考。

Gomocup engine：

```bash
cargo run --release --bin gomocup_engine
target/release/gomocup_engine --depth 8 --width 40
target/release/gomocup_engine --depth 6 --width 20 --root-profile
target/release/gomocup_engine --tt-bits 22
target/release/gomocup_engine --profile fast
```

常用 `INFO`：

- `INFO rule 0|4` 或 `INFO rule freestyle|renju`（仅空棋盘时允许）
- `INFO timeout_turn N`
- `INFO time_left N`
- `INFO max_node N`
- `INFO profile base|fast`
- `INFO compute_vcf 0|1`
- `INFO root_vcf_depth N`
- `INFO opponent_vcf_depth N`
- `INFO vct_verify_opponent_vcf_depth N`
- `INFO vcf_multi_reply 0|1`
- `INFO compute_vct 0|1`
- `INFO root_vct_depth N`
- `INFO vct_strict_and_memo_key 0|1`
- `INFO fast_history_ordering 0|1`
- `INFO nonroot_vcf 0|1`
- `INFO overlap_vct_alphabeta 0|1`
- `INFO tt_bits N`
- `INFO static 0|1`
- `INFO dynamic_board_margin N`
- `INFO root_profile 0|1`

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

```bash
python3 scripts/run_gomocup_match.py \
  --case-file cases/match/smoke_quick.jsonl \
  --jobs 12 \
  --max-moves 80 \
  --move-timeout-sec 90 \
  --game-timeout-sec 600 \
  --output /tmp/base_fast_smoke_quick.json
```

同局面一手 benchmark：

```bash
cargo build --release --bin case_probe
python3 scripts/bench_match_cases.py \
  --case-file cases/match/standard.jsonl \
  --jobs 16 \
  --output /tmp/base_fast_standard_bench.json
```

`run_gomocup_match.py` 用于真实对战和棋力评估；`bench_match_cases.py` 用于同一批前缀局面的一手搜索对照。判断优化是否真的提速，应先看同局面 benchmark，再看对战胜率和长尾耗时。

连珠对战使用 `--rule renju`。对战脚本会给双方设置 Gomocup 规则 `4`，
并使用规则感知裁判拒绝黑方禁手：

```bash
cargo build --release --bin gomocup_engine --bin renju_referee
python3 scripts/run_gomocup_match.py \
  --rule renju \
  --case-file cases/renju/strength_100_prefixes.jsonl \
  --engine-a-side both \
  --engine-a-command 'target/release/gomocup_engine --depth 8 --width 40'
```

完整连珠设计、验证证据、SlowRenju 对照约定和剩余工作见
`docs/renju-forbidden-design.md`。

如需观察真实跨手 TT，可给 `run_gomocup_match.py` 加 `--reuse-engine-state`。默认对战脚本每手 `RESTART`，更适合公平复现；`--reuse-engine-state` 会保留 engine/searcher 状态，并把每手 `MESSAGE tt_generation current=... old=...` 汇总到 JSON 的 `tt_generation` 字段。

## 目录

```text
src/                 Rust engine、Gomocup、GUI、diff/case probe
cases/diff/          root 差分 case
cases/match/         对战和 benchmark 前缀局面
cases/renju/         禁手、战术、候选诊断和棋力前缀 case
data/static/         从 reference 提取的静态矩阵
scripts/             差分、对战、benchmark、case 抽取脚本
tests/               Rust 自动测试
```

## 当前重点

1. 保留已经完成的 100 局固定深度 Rust vs SlowRenju 连珠门槛；后续扩样应增加独立来源对局，而不是继续截取同一批轨迹的相邻前缀。
2. 以当前优化后基线继续降低连珠每节点剩余开销和 VCF/VCT 长尾；性能改动必须通过 oracle、root diff 和棋力门槛。
3. 学习 Rapfi 的首要目标是提高棋力，重点审计候选排序、评估、战术搜索、TT、剪枝/延伸和时间管理；线模式与禁手表也可作为加深搜索的支撑。
4. 保留已经完成的无禁手门槛：100 局 Rust-vs-SlowRenju、当前版本与 Renju 前历史版本逐手等价、classic reference/Rust 差分，以及 fast-vs-base 棋力检查。
5. 所有性能实验都要同时报告正确性、耗时、nodes、move/score/trace 是否变化。

## 许可证

本项目使用 GNU General Public License v3.0 或后续版本授权。详见 [LICENSE](LICENSE)。
