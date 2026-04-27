# rust_gomoku

`rust_gomoku` 是 Python reference 项目 `pygomoku` classic 主线的 Rust 重构。

项目现在按两条线推进：

- `classic/base`：默认路径，保持确定、可回归，继续对齐 `pygomoku` classic 语义。
- `fast`：性能和棋力实验路径，可以不保持固定局面语义完全一致，但对 base 胜率不能低于 `50%`，并且要有实际速度收益。

## 当前状态

已完成并可运行：

- 15x15 自由规则五子棋核心状态机、zobrist、配置、pattern、eval cache、movegen、ordering、TT、alpha-beta root search。
- root VCF 优先、root-only VCT 触发/验证/trace。
- Gomocup stdin/stdout 引擎入口和本地 Web GUI。
- Rust/reference root 差分脚手架和固定开局对战脚本。
- 仓库内保留 `opponent/zhou` 作为轻量对手；完整 Python reference 不随仓库提交。

已移除或不作为默认路径：

- Lazy SMP、root YBWC、root full-window split、aspiration window。
- LUT/indexed/batch-lines 等收益不稳定的局部 eval 优化。

仍保留的实验项：

- `overlap_vct_alphabeta`：默认关闭，只用于 VCF miss 后重叠 VCT 与 alphabeta。
- `tt_bits`：Gomocup 可配置 TT 容量；base 默认仍为 `20`。

## 默认参数

主要搜索默认值在 [src/config.rs](src/config.rs)。TT 默认容量目前保持为 `20` bits。

| 参数 | 默认值 |
|---|---:|
| 固定搜索深度 | `8` |
| 固定 root width | `40` |
| 时间控制最大深度 | `25` |
| 时间控制最大 width | `40` |
| `root_vcf_depth` | `8` |
| `opponent_vcf_depth` | `7` |
| `vct_verify_opponent_vcf_depth` | `4` |
| `root_vct_depth` | `8` |
| TT bucket bits | `20` |
| `compute_vcf` / `compute_vct` | 开启 |
| `overlap_vct_alphabeta` | 关闭 |
| `nonroot_vcf` | 关闭 |
| `static_board` | 开启 |
| `dynamic_board_margin` | `4` |

与 Python reference 严格差分或复现实验时，通常显式使用 `depth=6,width=20,root_vct_depth=4`。

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

## 快速开始

构建：

```bash
cargo build --release
```

测试：

```bash
cargo test --quiet
```

Gomocup smoke：

```bash
printf 'START 15\nBEGIN\nEND\n' | cargo run --quiet --bin gomocup_engine -- --depth 2 --width 8
```

启动 GUI：

```bash
cargo run --release --bin gomoku_gui
```

打开 `http://127.0.0.1:7878`。GUI 支持执黑/执白、悔棋、重新开局、异步思考、手数显示和状态面板；快捷键 `U` 悔棋，`R` 重新开局。

快速试 GUI 可降低参数：

```bash
cargo run --release --bin gomoku_gui -- --depth 6 --width 20
```

## Gomocup 入口

默认运行：

```bash
cargo run --release --bin gomocup_engine
```

常用参数：

```bash
target/release/gomocup_engine --depth 8 --width 40
target/release/gomocup_engine --depth 6 --width 20 --root-profile
target/release/gomocup_engine --profile fast
target/release/gomocup_engine --tt-bits 22
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
- `INFO compute_vct 0|1`
- `INFO root_vct_depth N`
- `INFO nonroot_vcf 0|1`
- `INFO overlap_vct_alphabeta 0|1`
- `INFO tt_bits N`
- `INFO static 0|1`
- `INFO dynamic_board_margin N`
- `INFO root_profile 0|1`

## 差分测试

默认 root 差分：

```bash
python3 scripts/run_diff.py --jobs 10
```

全部 case：

```bash
PYGOMOKU_REF_ROOT=~/python_ws/pygomoku python3 scripts/run_diff.py --profile all --jobs 10
```

单 case：

```bash
cargo run --quiet --bin diff_probe -- --case cases/diff/root_center_11.json
```

带 root profile：

```bash
cargo run --quiet --bin diff_probe -- --case cases/diff/root_center_11_d6_w5.json --root-profile
```

默认差分比较 board、zobrist、root move/score/depth/nodes 和 tactical trace；耗时字段不参与断言。

## 对战脚本

构建 engine：

```bash
cargo build --release --bin gomocup_engine
```

Rust 默认参数对 Python reference 9 开局双边 18 局：

```bash
python3 scripts/run_engine_match.py \
  --opening-set 9 \
  --jobs 18 \
  --output /tmp/rust_vs_reference_9_openings.json
```

常用变体：

```bash
python3 scripts/run_engine_match.py --opening-set 9 --opening-index 4 --rust-side black --jobs 1
python3 scripts/run_engine_match.py --opening-set 9 --jobs 5 --move-timeout-sec 180 --game-timeout-sec 1200
python3 scripts/run_engine_match.py --opening-set 9 --jobs 10 --rust-command "target/release/gomocup_engine --tt-bits 22"
```

通用 `fast` vs `base` 对战：

```bash
python3 scripts/run_gomocup_match.py \
  --opening-set 9 \
  --jobs 18 \
  --output /tmp/fast_vs_base_9_openings.json
```

默认 engine A 是 `base`，命令为 `gomocup_engine --profile base`；engine B 是 `fast`，命令为 `gomocup_engine --profile fast`。核心指标是胜率、avg、median、p95、max、错误率和超时率。

## 目录

```text
src/                 Rust engine、Gomocup、GUI、diff probe
cases/diff/          root 差分 case
data/static/         从 reference 提取的静态矩阵
opponent/zhou/       zhou 基线对手
scripts/             差分、reference 对战、通用 engine 对战、静态数据提取脚本
tests/               Rust 自动测试
```

## 下一步

1. 用通用对战脚本建立 fast 线基准报表。
2. 在 fast 线继续研究 TT 容量、replacement policy、ordering、剪枝和并行方案。
3. 继续扩大 classic/base 的 eval、movegen、VCF/VCT 差分覆盖。
4. 从真实慢手中抽取更多固定回归局面。
