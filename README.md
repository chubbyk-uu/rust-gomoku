# rust_gomoku

`rust_gomoku` 的目标不是重新设计一个新的五子棋引擎，而是：

**在尽量不破坏语义的前提下，用 Rust 重构 `reference/pygomoku`。**

这里的“语义”包括但不限于：

- 棋盘状态机行为
- `(x, y)` 坐标与扁平 `Move` 编码
- 评估、候选点、搜索、TT、VCF / VCT 的行为
- Gomocup 协议行为
- 默认参数、默认搜索行为
- 固定局面上的回归结果

如果 Rust 写法更优雅，但行为偏离 `reference/pygomoku`，默认视为不符合当前项目目标。

## 当前状态

本仓库仍处于重构早期，但 classic 主链已经从基础状态机推进到搜索和战术层。

### 已完成

- Rust crate 基础骨架
- `constants`
- `types`
- `move` 编码 / 解码
- `board`
- `zobrist`
- `config`
- `patterns`
- `eval/caches`
- `eval/local`
- `eval/global_eval`
- `search/movegen`
- `search/ordering`
- `search/tt`
- `search/alphabeta`
- `search/root`
- `threats/threat_board`
- `vcf`
- `vct`
- `RootSearcher` 的 VCF 优先、VCT 触发、VCT 根着验证和 trace
- Gomocup 协议入口
- 命令行 Gomocup engine 入口
- 与 `opponent/zhou` 的 9 开局黑白双边对战验证
- reference / Rust 双端差分测试脚手架
- 12 个 root 搜索差分 case
- reference 静态数据提取脚本
- 可选 Lazy SMP 实验入口，默认关闭
- 无锁固定桶 TT，为后续并发搜索保留基础设施
- 一批与 reference 对齐的 Rust 自动测试

### 当前已对齐的能力

- `BOARD_SIZE = 15`
- `BLACK = 1`
- `WHITE = -1`
- `EMPTY = 0`
- `(x, y)` 即 `(列, 行)` 语义
- `Board::play / undo / replay`
- `winner / side_to_move / move_history / zobrist_key` 同步更新
- `config` 默认参数切片与默认运行时配置
- `patterns` 的 `ShapeLabel / bucket / line / shape_table`
- `eval` 的局部缓存、增量更新、全局评估分支
- `last5 / next43` 相关评估分支
- `snapshot / restore / value_log / shape_log` 基础契约
- movegen 的候选点覆盖、forcing class 折叠、hostile-three 扩展
- ordering 的 tuple 排序和 root classic selection-sort 语义
- TT 的 probe / store / replacement / alpha-beta window 行为
- TT 已从惰性 `HashMap` 改为无锁固定桶表；默认串行行为继续由差分测试约束
- alphabeta 的终局分数、深度、node limit、deadline、fallback 行为
- root 搜索的空盘中心、动态棋盘窗口、classic fallback RNG、固定开局回归
- root tactical fast path 在 VCF/VCT 返回前不初始化 eval caches，保持与 reference 结构更接近
- threat board 的威胁点、A4/B4/A3 判定、嵌套 `play/undo` 恢复不变量
- VCF 的 begin depth 映射、序列 key、forcing threat 搜索
- VCT 的 trigger、OR/AND 搜索、memo 清理、root 接入、验证与 `vct_ms` trace
- Gomocup `START / RECTSTART / RESTART / BEGIN / TURN / BOARD / DONE / TAKEBACK / INFO / ABOUT / END`
- 命令行入口默认 `--depth 6 --width 20`，对齐 reference GUI / Gomocup engine 正式运行参数
- Rust 默认开启 VCT，`root_vct_depth = 8`；这是基于 Rust 性能的有意偏离，reference Python 主线因性能约束使用较低默认值
- Gomocup `BOARD` 模式对齐 reference 的颜色重建语义：`sfn == opn - 1` 时只调整输入列表顺序，正式重放仍从黑棋开始
- Rust Gomocup engine 在 `d6/w20`、zhou `d5`、9 开局黑白双边共 18 线中，与 reference Gomocup engine 的完整走法序列一致
- `diff_probe` 使用 `serde/serde_json` 读取固定局面 JSON，输出 Rust 侧 board/root/trace 结果
- `diff_probe` 可通过 `--lazy-smp --lazy-smp-workers N` 单独观察 Rust 并行路径
- `scripts/diff_reference.py` 调用 `reference/pygomoku` 输出同结构 JSON
- `scripts/run_diff.py` 可批量运行 `cases/diff/*.json`，当前 12 个 root case 全部通过
- 单线程兼容模式下，差分默认比较 `root.nodes`，用于尽早发现搜索路径漂移
- Lazy SMP 是 experimental 功能，默认关闭；开启后允许 TT 命中、节点数和 best move 变化，不作为 reference 等价路径

### 还没有完成

- GUI 或其它外部集成
- movegen / eval / VCF / VCT 的细粒度双端差分覆盖
- Python fallback 与 Cython 加速路径的系统性交叉验证
- Lazy SMP 的策略重设计；当前朴素 helper 填表没有稳定性能收益

也就是说，**当前已经有单线程 classic 搜索主链、战术层、Gomocup stdin/stdout 入口、一轮 zhou 对战对齐验证、root 搜索层面的最小双端差分脚手架，以及可选 Lazy SMP 实验路径；但还没有覆盖 eval / movegen / VCF / VCT 的完整差分体系，Lazy SMP 也还不是可推荐的主线性能方案。**

## 当前实现原则

项目当前遵循这些原则：

- 以 `reference/pygomoku` 作为唯一主语义基准
- 先保证单线程、确定性、可回归
- Rust 代码从一开始就考虑未来并行架构边界
- 但默认执行路径先不因为并行而改变 reference 结果
- 先做可验证等价实现，再谈性能优化

当前的并行设计立场是：

- `Board` 和 `EvalCaches` 保持可快照、可恢复
- `AlphaBetaSearcher`、`VCFSearcher`、`VCTSearcher` 当前持有各自搜索状态，适合作为线程本地 worker 状态
- root tactical fast path 仍只在主线程执行；辅助线程不跑 VCF/VCT/root trace
- Lazy SMP 只在进入 alphabeta 主链后启动，辅助线程使用线程本地 `Board / EvalCaches / AlphaBetaSearcher`
- Lazy SMP 辅助线程共享无锁 TT，只负责填表；最终返回值仍取主线程 root search 结果
- 默认必须保留稳定的单线程兼容模式，且 `node_limit / time_limit` 下当前不启用 Lazy SMP
- 已放弃 root-split full-window 方案：它会丢掉串行 PV 搜索的 alpha/null-window 剪枝，实测容易增大搜索量并降低棋力
- Lazy SMP 当前仅作为 experimental 开关保留，不作为 reference 等价路径，也不建议默认用于对战；开启后不强制 `nodes` 或 best move 与串行严格一致
- 当前实测显示朴素 Lazy SMP helper 在 `d6/w20` 和 `d8/w30` 下没有稳定提速，主要价值是保留并行架构边界和验证工具

## 目录说明

当前仓库的重要目录：

```text
rust_gomoku/
├── src/
│   ├── board.rs
│   ├── config.rs
│   ├── constants.rs
│   ├── eval/
│   │   ├── caches.rs
│   │   ├── global_eval.rs
│   │   └── local.rs
│   ├── patterns/
│   │   ├── buckets.rs
│   │   ├── line.rs
│   │   ├── shape_table.rs
│   │   └── shapes.rs
│   ├── search/
│   │   ├── alphabeta.rs
│   │   ├── movegen.rs
│   │   ├── ordering.rs
│   │   ├── root.rs
│   │   └── tt.rs
│   ├── threats/
│   │   ├── threat_board.rs
│   │   ├── types.rs
│   │   ├── vcf.rs
│   │   └── vct.rs
│   ├── protocol/
│   │   ├── gomocup.rs
│   │   └── mod.rs
│   ├── bin/
│   │   ├── diff_probe.rs
│   │   └── gomocup_engine.rs
│   ├── types.rs
│   └── zobrist.rs
├── tests/
├── cases/
│   └── diff/
├── data/
│   ├── reference_text/
│   └── static/
├── scripts/
│   ├── compare_diff_outputs.py
│   ├── diff_reference.py
│   ├── extract_static_data.py
│   └── run_diff.py
└── reference/
    └── pygomoku/
```

其中：

- `reference/pygomoku/`：当前语义基准
- `data/reference_text/`：从 reference vendoring 进来的源文本副本
- `data/static/`：从 vendored 文本提取出的纯静态数据文件
- `scripts/extract_static_data.py`：负责重新生成和校验静态数据
- `cases/diff/`：reference / Rust 双端差分固定局面
- `src/bin/diff_probe.rs`：Rust 侧差分探针
- `scripts/diff_reference.py`：Python reference 侧差分探针
- `scripts/run_diff.py`：批量运行差分 case

## 静态数据来源

当前 Rust 不再直接在运行时依赖 `reference/` 里的 Python 源文件，而是改成：

1. vendoring reference 文本到 `data/reference_text/`
2. 用脚本提取必须的静态数据到 `data/static/`
3. Rust 运行时只读取 `data/static/*.txt`

当前已提取的数据：

- `data/static/default_eval_para.txt`
- `data/static/shape_table.txt`

重新生成：

```bash
python3 scripts/extract_static_data.py
```

校验当前生成文件是否仍与 vendored source 一致：

```bash
python3 scripts/extract_static_data.py --check
```

## 开发命令

格式化：

```bash
cargo fmt
```

运行测试：

```bash
cargo test
```

## 测试现状

当前已经建立了 Rust 侧对齐测试，覆盖这些方向：

- `board`
- `zobrist`
- `config`
- `patterns`
- `eval/caches`
- `eval/local`
- `eval/global_eval`
- `search/movegen`
- `search/ordering`
- `search/tt`
- `search/alphabeta`
- `search/root`
- `threats/threat_board`
- `vcf`
- `vct`
- `protocol/gomocup`

这些测试既包含基础行为，也包含 handpicked 精确值回归和固定局面搜索回归。

当前验证命令：

```bash
python3 scripts/extract_static_data.py --check
cargo test
python3 scripts/run_diff.py --profile all --jobs 10
```

当前全量测试通过规模：177 个 Rust 测试通过。

Gomocup CLI smoke：

```bash
printf 'START 15\nBEGIN\nEND\n' | cargo run --quiet --bin gomocup_engine -- --depth 2 --width 8
```

Lazy SMP CLI smoke：

```bash
printf 'START 15\nBEGIN\nEND\n' | cargo run --quiet --bin gomocup_engine -- --depth 2 --width 8 --lazy-smp --lazy-smp-workers 2
```

Lazy SMP 单局探针：

```bash
cargo run --quiet --bin diff_probe -- --case cases/diff/root_deep_opening_10_4_d8_w15.json --lazy-smp --lazy-smp-workers 4
```

Lazy SMP 当前结论：

- 这是 experimental 功能，默认关闭。
- 它保留了并行搜索的工程边界：线程本地 `Board / EvalCaches / AlphaBetaSearcher`、共享无锁 TT、stop signal、CLI / protocol 开关和探针参数。
- 朴素 helper 填表策略当前没有稳定性能收益，不建议作为默认对战模式。
- `d8/w30`、同开局 `(7,7)`、Rust 执黑对 zhou 的单局 smoke 中，Lazy2 与串行棋谱一致，但 Rust 平均耗时略慢。
- 更早的 Lazy8 / 多开局 smoke 出现过走法漂移，说明共享 TT 会改变主线程 move ordering；这是 experimental 模式允许的行为，但不能用于 reference 等价验证。
- 后续如果继续推进并行，应先重设计 helper 写入策略或使用更保守的后台分析模式，而不是继续增加 worker 数。

Gomocup / zhou 对战验证：

- 参数：Rust/reference `depth=6, width=20`，zhou `depth=5`
- 说明：这轮历史对战是在当时显式对齐 reference runtime 的条件下完成；现在 Rust 默认 `root_vct_depth` 已提升到 8，后续对战验证应同时记录 VCT depth
- 开局：reference 的 9 个固定开局
- 颜色：黑白双边，共 18 线
- 结果：Rust 18 胜，reference 18 胜，18/18 完整走法序列一致
- 输出样例位置：`/tmp/rust_gomoku_fixed_vs_zhou_9_black.json`、`/tmp/rust_gomoku_fixed_vs_zhou_9_white.json`

Reference / Rust 差分脚手架：

```bash
cargo run --quiet --bin diff_probe -- --case cases/diff/root_center_11.json > /tmp/rust_diff.json
python3 scripts/diff_reference.py --case cases/diff/root_center_11.json > /tmp/ref_diff.json
python3 scripts/compare_diff_outputs.py --rust /tmp/rust_diff.json --reference /tmp/ref_diff.json
```

批量运行全部差分 case：

```bash
python3 scripts/run_diff.py --profile all
```

日常快速差分默认只跑 fast case，可以并行执行：

```bash
python3 scripts/run_diff.py --jobs 10
```

只跑 slow case：

```bash
python3 scripts/run_diff.py --profile slow --jobs 10
```

当前已有 12 个 root 搜索差分 case，其中 11 个 fast、1 个 slow：

- `root_center_11.json`：中盘固定局面，`d6/w20`
- `root_center_11_d6_w5.json` / `root_center_11_d6_w12.json`：同一局面的 root_width 裁剪边界
- `root_deep_opening_10_4_d8_w15.json`：depth 8 深层 alphabeta / TT 路径，标记为 slow
- `root_node_limit_5.json`：带 `node_limit=200` 的 root 搜索路径
- `root_node_limit_5_d6_w20_nodes_20.json` / `root_node_limit_5_d6_w20_nodes_1000.json`：节点超限后的 best-move 选择边界
- `root_vcf_hit_6.json`：VCF fast path 命中场景
- `root_vct_hit_8.json`：VCT trigger / 命中场景
- `root_white_first_3.json`：白棋先手 side_to_move 场景
- `root_zhou_black_5_10_4_prefix_14_d6_w20.json` / `root_zhou_white_8_7_7_prefix_16_d6_w20.json`：从 zhou 18 线对战抽样的多步重建局面

默认比较字段包括 board 状态、`zobrist_key`、root move/score/depth/nodes 和 tactical trace。`root.nodes` 当前在单线程兼容模式下作为强断言，用来捕捉候选顺序、TT、剪枝或提前停止语义的细微漂移；但它不是最终对弈语义字段，后续并行模式或优化模式不应默认强锁 `nodes`。`vct_ms` 等耗时字段始终不应纳入确定性回归断言。

## 当前限制

需要明确：

- 当前已有基础 Gomocup 入口，并通过一轮 zhou 9 开局黑白双边对战对齐验证
- 当前重点仍然是“语义迁移”，不是性能冲刺
- 某些实现虽然已经有增量路径，但整体仍未进入最终优化阶段
- Rust 默认 `root_vct_depth = 8` 是明确的性能型偏离；需要与 reference 做严格差分时，应在 case 或协议 `INFO root_vct_depth` 中显式设成 reference 对应值
- 已有最小 reference / Rust 差分脚手架，但覆盖范围还只到 root 搜索层面
- `RootTrace.vct_ms` 是耗时调试字段，不应纳入确定性回归断言
- `root.nodes` 只应在单线程兼容模式下强断言；并行或优化模式应改为比较 move/score/depth/trace 等语义字段
- root tactical fast path 当前仍应保持 VCF 优先于 VCT，VCT 只在 trigger 命中后运行
- 多线程只完成架构预留，没有实现执行路径

## 下一步

下一步不建议继续大面积扩搜索算法，应先把差分测试覆盖面补到能支撑后续优化和并行。

### 1. Root 搜索后续补强

1. 继续从 18 线 zhou 对战关键 ply 抽样，补充更多接近真实对局的 root 差分局面。
2. 检查 `RootTrace` 是否还需要补充协议或日志层会用到的字段，但不要把耗时字段纳入确定性回归断言。
3. 继续核对 root search 与 `reference/pygomoku/pygomoku/search/root.py` 的结构差异，只保留有明确理由的差异。
4. 明确区分单线程兼容断言字段和未来并行模式断言字段，避免 `nodes` 阻塞无语义变化的并行优化。

### 2. Gomocup / zhou 对战验证沉淀

1. 把本次手工命令整理成脚本或 README 中的可复制命令。
2. 后续优先比较 Rust/reference 的完整 move sequence，而不是只看胜负。
3. 继续保留输出到 `/tmp` 或专门 ignored 目录，避免把对战 JSON 误提交。
4. 若后续开启并行搜索，必须重复跑这 18 线，确认默认单线程兼容路径不漂移。

### 3. 协议补强

1. 与 reference `tests/test_protocol.py` 继续逐条比对，补缺少的边界用例。
2. 检查 `ABOUT` 文本是否需要完全对齐 reference，还是保留 Rust engine 名称。
3. 确认 `TAKEBACK` 是否保持 reference 的“忽略参数，只撤一步”语义。
4. 根据 zhou smoke 结果补 transcript 回归。

### 4. Reference / Rust 差分测试脚手架

1. 扩展 probe 输出，从 root search 逐步覆盖 movegen、VCF、VCT。
2. 再补 eval 细粒度字段，避免一开始把输出结构做得过大。
3. 为不同层级拆分默认比较字段，例如 root 兼容模式比较 `nodes`，并行模式不比较 `nodes`。
4. 对发现的差异明确记录：Rust 对齐 Python fallback、Cython 路径，还是 reference 当前测试主线。
5. 耗时字段始终不作为强断言。

### 5. 并行架构第一版

1. 保留单线程兼容路径作为默认 baseline。
2. 新增 root candidate task 抽象，不改变候选顺序和最终归并规则。
3. 每个 worker 使用线程本地 `Board`、`EvalCaches`、`AlphaBetaSearcher`、`VCFSearcher`、`VCTSearcher`。
4. 第一版不共享 TT，只做确定性 root-split。
5. 加重复运行稳定性测试：同一局面、同一深度、多次运行返回同一 move、score、depth。
6. 后续再评估共享 TT、取消 token、统计聚合和时间控制。

## 一句话总结

当前仓库已经完成了 **基础状态机 + 配置 + pattern + eval + 单线程 classic 搜索 + VCF/VCT 战术路径 + Gomocup 协议入口** 的 Rust 化，并建立了对应回归测试、一轮 zhou 对战对齐验证，以及 root 搜索层面的 reference / Rust 差分脚手架；下一阶段重点是 **扩展差分覆盖和落地确定性并行架构**。
