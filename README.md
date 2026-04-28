# rust_gomoku

`rust_gomoku` 是 Python reference 项目 `pygomoku` classic 主线的 Rust 重构。当前主线目标是守住 classic 行为，同时继续降低平均耗时和长尾耗时。

## 当前状态

已完成：

- 15x15 自由规则五子棋状态机、zobrist、配置、pattern、eval cache、movegen、ordering、TT、alpha-beta root search。
- root VCF、root-only VCT 触发/验证/trace。
- Gomocup stdin/stdout 引擎入口和本地 Web GUI。
- Rust/reference 差分、Rust/reference 对战、base/fast 对战和同局面 benchmark 脚手架。
- 非 root 候选排序成本优化：保持原排序 key 不变，减少 `getmi` 和候选复制开销。
- fast profile 默认开启第三版 history/killer ordering：只在静态排序同组内调整安静着法顺序，base 不受影响。
- 仓库内保留 `opponent/zhou` 作为轻量对手；完整 Python reference 不随仓库提交。

不作为默认路径：

- Lazy SMP、root YBWC、root full-window split、aspiration window。
- LUT/indexed/batch-lines 等收益不稳定的局部 eval 优化。

保留但默认关闭：

- `overlap_vct_alphabeta`：VCF miss 后重叠 VCT 与 alphabeta 的实验开关。
- `root_profile`：root candidate 计时诊断。

GUI 入口为了降低手动对局体感等待，默认单独开启 `overlap_vct_alphabeta`；Gomocup、diff、case probe 和库默认仍保持关闭。

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

与 Python reference 严格差分或复现实验时，通常显式使用 `depth=6,width=20,root_vct_depth=4`。

默认 profile 是 `base`，用于守住 classic 语义和 reference 差分。`--profile fast` 当前会默认开启 `fast_history_ordering`；如需对照可用 `--no-fast-history-ordering` 关闭。

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
```

启动 GUI：

```bash
cargo run --release --bin gomoku_gui
```

打开 `http://127.0.0.1:7878`。GUI 支持执黑/执白、悔棋、重新开局、Base/Fast 模式切换、异步思考、手数显示和状态面板；快捷键 `U` 悔棋，`R` 重新开局。Base/Fast 切换只在引擎未思考时允许，不重置当前棋局，只影响下一次引擎思考。

Gomocup engine：

```bash
cargo run --release --bin gomocup_engine
target/release/gomocup_engine --depth 8 --width 40
target/release/gomocup_engine --depth 6 --width 20 --root-profile
target/release/gomocup_engine --tt-bits 22
target/release/gomocup_engine --profile fast
```

常用 `INFO`：

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

## 目录

```text
src/                 Rust engine、Gomocup、GUI、diff/case probe
cases/diff/          root 差分 case
cases/match/         对战和 benchmark 前缀局面
data/static/         从 reference 提取的静态矩阵
opponent/zhou/       zhou 基线对手
scripts/             差分、对战、benchmark、case 抽取脚本
tests/               Rust 自动测试
```

## 近期重点

1. 继续扩大 reference/Rust 差分覆盖。
2. 针对真实慢手优化 VCT miss 和 alphabeta 长尾，优先寻找能稳定压低 p95/max 的方案。
3. 继续扩大 fast profile 的 fast vs base 对战覆盖，确认默认开启的 history/killer ordering 在更大样本下胜率不低于 base。
4. 所有性能实验都要同时报告正确性、耗时、nodes、move/score/trace 是否变化。
