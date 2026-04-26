# rust_gomoku

`rust_gomoku` 是对 Python reference 项目 `pygomoku` classic 主线的 Rust 重构。

当前目标不是重新发明一个五子棋引擎，而是在尽量不改变语义的前提下，把 reference 的棋盘状态机、评估、搜索、VCF/VCT、Gomocup 协议和固定局面回归迁移到 Rust。若 Rust 写法更“优雅”但行为偏离 reference，默认视为回归。

## 当前状态

主线已经完成并可运行：

- 15x15 自由规则五子棋核心状态机：`Board / Move / Side / Zobrist`
- pattern、局部 eval cache、全局评估、候选点生成、ordering
- classic alpha-beta root search、TT、node/time limit、fallback RNG
- root VCF 优先、root-only VCT 触发、验证与 trace
- Gomocup stdin/stdout 引擎入口
- 本地 Web GUI 人机对战入口
- Rust/reference 双端 root 差分脚手架
- Rust/reference 固定开局 Gomocup 对战脚本
- 仓库内保留 `opponent/zhou` 作为轻量基线对手

当前主线是确定性串行搜索。Lazy SMP、root YBWC 等并行实验已经从主线移除，因为实测没有稳定收益且会改变搜索路径或棋力。后续并行应在独立分支重新设计，不在当前串行基线上补丁式接入。

## 默认参数

默认参数集中在 [src/config.rs](src/config.rs)。

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
| `compute_vcf` / `compute_vct` | 开启 |
| `nonroot_vcf` | 关闭 |
| `static_board` | 开启 |
| `dynamic_board_margin` | `4` |

说明：

- Rust 默认运行参数有意高于 reference 的常用对战参数。
- 需要与 Python reference 严格对齐时，通常显式设置 `depth=6,width=20,root_vct_depth=4`。
- `static_board=true` 只影响 root allowed window；正常 alphabeta 候选仍来自已有棋子附近的 covered moves。

## Reference 路径

完整 Python reference 不提交进本仓库。本机默认约定路径：

```bash
~/python_ws/pygomoku
```

也可以显式指定：

```bash
export PYGOMOKU_REF_ROOT=~/python_ws/pygomoku
```

差分脚本会按以下顺序查找 reference：

1. 命令行 `--ref-root`
2. 环境变量 `PYGOMOKU_REF_ROOT`
3. 本机约定路径 `~/python_ws/pygomoku`

## 快速开始

构建：

```bash
cargo build --release
```

运行测试：

```bash
cargo test --quiet
```

运行 Gomocup engine smoke：

```bash
printf 'START 15\nBEGIN\nEND\n' | cargo run --quiet --bin gomocup_engine -- --depth 2 --width 8
```

启动 GUI：

```bash
cargo run --release --bin gomoku_gui
```

然后打开：

```text
http://127.0.0.1:7878
```

GUI 支持执黑/执白、重新开局、悔棋、异步引擎思考、棋子手数显示、最后一手红色标记和参数/状态面板。快捷键：`U` 悔棋，`R` 重新开局。

如果只是快速试 GUI，可降低参数：

```bash
cargo run --release --bin gomoku_gui -- --depth 6 --width 20
```

## Gomocup 入口

命令行入口：

```bash
cargo run --release --bin gomocup_engine
```

可选参数：

```bash
target/release/gomocup_engine --depth 8 --width 40
target/release/gomocup_engine --depth 6 --width 20 --root-profile
```

已实现的主要协议命令：

- `START`
- `RECTSTART`
- `RESTART`
- `BEGIN`
- `TURN`
- `BOARD` / `DONE`
- `TAKEBACK`
- `INFO`
- `ABOUT`
- `END`

常用 `INFO` 项：

- `INFO timeout_turn N`
- `INFO time_left N`
- `INFO max_node N`
- `INFO compute_vcf 0|1`
- `INFO root_vcf_depth N`
- `INFO opponent_vcf_depth N`
- `INFO vct_verify_opponent_vcf_depth N`
- `INFO nonroot_vcf 0|1`
- `INFO compute_vct 0|1`
- `INFO root_vct_depth N`
- `INFO static 0|1`
- `INFO dynamic_board_margin N`
- `INFO root_profile 0|1`

`root_profile` 只用于慢手定位。开启后，Gomocup 会在最终坐标前输出 root depth 和候选点耗时统计；该字段不参与确定性回归。

## 差分测试

运行 fast root 差分：

```bash
python3 scripts/run_diff.py --jobs 10
```

运行全部 case：

```bash
PYGOMOKU_REF_ROOT=~/python_ws/pygomoku python3 scripts/run_diff.py --profile all --jobs 10
```

单独运行一个 Rust probe：

```bash
cargo run --quiet --bin diff_probe -- --case cases/diff/root_center_11.json
```

带 root profile：

```bash
cargo run --quiet --bin diff_probe -- --case cases/diff/root_center_11_d6_w5.json --root-profile
```

当前 `cases/diff/` 主要覆盖 root search、节点限制、root width、VCF/VCT fast path、白棋先手和对战抽样局面。默认比较 board 状态、zobrist、root move/score/depth/nodes 和 tactical trace。耗时字段不参与断言。

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

脚本默认设置：

- Rust：当前 engine 默认参数
- Python reference：`python -m pygomoku.gomocup_engine --depth 6 --width 20`
- Reference 额外 `INFO root_vct_depth 4`
- 每手通过 `BOARD` 全量同步局面
- 默认单手超时 `120s`
- 默认单局超时 `900s`

常用变体：

```bash
python3 scripts/run_engine_match.py --opening-set 9 --opening-index 4 --rust-side black --jobs 1
python3 scripts/run_engine_match.py --opening-set 9 --jobs 5 --move-timeout-sec 180 --game-timeout-sec 1200
```

最近一次默认串行 9 开局黑白双边 18 局结果：Rust `17 胜 / 1 负`，唯一败局是 `[4,4]` 开局 Rust 执白。

## 目录结构

```text
src/
├── board.rs
├── config.rs
├── constants.rs
├── eval/
├── patterns/
├── protocol/
├── search/
├── threats/
├── bin/
│   ├── diff_probe.rs
│   ├── gomocup_engine.rs
│   └── gomoku_gui.rs
├── types.rs
└── zobrist.rs

cases/diff/          root 差分 case
data/static/         从 reference 提取的静态矩阵
opponent/zhou/       zhou 基线对手
scripts/             差分、对战、静态数据提取脚本
tests/               Rust 自动测试
```

## 工程边界

必须保持的语义：

- 坐标统一使用 `(x, y)`，即 `(列, 行)`
- `BLACK = 1`，`WHITE = -1`，`EMPTY = 0`
- 搜索主流程通过 `play / undo` 修改局面
- 默认串行路径必须确定、可回归
- 修改 eval、movegen、TT、alphabeta、VCF/VCT 前应补或更新差分/回归

当前不在主线做的事：

- 不默认启用并行搜索
- 不保留 Lazy SMP/YBWC 开关
- 不为提升速度改变候选排序或搜索语义
- 不把 `opponent/zhou` 当作主语义来源

## 下一步

优先级较高：

1. 扩展 movegen / eval / VCF / VCT 的双端差分覆盖。
2. 从真实对战慢手中抽取更多固定局面 case。
3. 继续做串行热点 profiling 和低风险数据结构优化。
4. 补协议边界用例，继续对齐 reference。
5. 若重启并行，必须新建分支，并先设计验证标准。

## 当前结论

当前主线已经具备可运行的 Rust classic engine、Gomocup 入口、本地 GUI、reference 差分和固定开局对战验证。后续重点不是继续堆功能，而是扩大可验证覆盖面，在串行语义稳定的基础上做性能优化。
