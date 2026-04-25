# rust_gomoku

`rust_gomoku` 的目标不是重新设计一个新的五子棋引擎，而是：

**在尽量不破坏语义的前提下，用 Rust 重构 Python reference `pygomoku`。**

这里的“语义”包括但不限于：

- 棋盘状态机行为
- `(x, y)` 坐标与扁平 `Move` 编码
- 评估、候选点、搜索、TT、VCF / VCT 的行为
- Gomocup 协议行为
- 默认参数、默认搜索行为
- 固定局面上的回归结果

如果 Rust 写法更优雅，但行为偏离 Python reference `pygomoku`，默认视为不符合当前项目目标。

## 当前状态

本仓库的 classic 主链已经从基础状态机推进到搜索、战术层、Gomocup 入口和本地 GUI。
当前重点已经从“能否跑通主链”转为“扩大差分覆盖、守住语义一致性，并继续做可验证的串行热点优化”。

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
- 本地 Web GUI 人机对战入口
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
- 命令行入口默认固定搜索为 `depth=8, width=40`；带时间控制且未显式传 CLI 深宽时，`SearchLimits` 最大上限为 `depth=25, width=40`
- 固定默认搜索限制由 `SearchLimits::fixed_from_config` 生成，时间控制上限由 `SearchLimits::timed_from_config` 生成，源头都在 `src/config.rs`
- Rust 默认开启 VCF/VCT：`root_vcf_depth = 8`、`opponent_vcf_depth = 7`、`vct_verify_opponent_vcf_depth = 4`、`root_vct_depth = 8`
- 运行时开关集中在 `RuntimeOptions`：`compute_vcf`、`nonroot_vcf`、`compute_vct`、`static_board`、`lazy_smp` 等都从 config 读取，协议层只负责更新 config
- 动态棋盘搜索窗口默认由已落子包络线外扩 `dynamic_board_margin = 4` 后再补成正方形；当前 `static_board = true`，默认不启用动态窗口，可通过 Gomocup `INFO static 0` / `INFO dynamic_board_margin N` 调整
- 正常候选生成不依赖动态窗口；`covered_moves` 使用 reference 对齐的固定 32 偏移覆盖表，包含距离 1、距离 2 的邻域和距离 3 的八方向点
- Gomocup `BOARD` 模式对齐 reference 的颜色重建语义：`sfn == opn - 1` 时只调整输入列表顺序，正式重放仍从黑棋开始
- Rust Gomocup engine 在 `d6/w20`、zhou `d5`、9 开局黑白双边共 18 线中，与 reference Gomocup engine 的完整走法序列一致
- `diff_probe` 使用 `serde/serde_json` 读取固定局面 JSON，输出 Rust 侧 board/root/trace 结果
- `diff_probe` 可通过 `--lazy-smp --lazy-smp-workers N` 单独观察 Rust 并行路径
- `scripts/diff_reference.py` 调用外部 Python reference 输出同结构 JSON
- `scripts/run_diff.py` 可批量运行 `cases/diff/*.json`，当前 12 个 root case 全部通过
- 单线程兼容模式下，差分默认比较 `root.nodes`，用于尽早发现搜索路径漂移
- Lazy SMP 是 experimental 功能，默认关闭；开启后允许 TT 命中、节点数和 best move 变化，不作为 reference 等价路径
- 串行主线已完成一轮热点优化：move ordering 的 `getmi` 预计算、movegen/eval 临时表固定数组化、局部 shape 读取避免整线复制、局部 shape 读取直接按 line index 扫描；固定 slow probe 在保持 `nodes` 不变的前提下，最新单次 release probe 约 `0.63s`
- `patterns::line` 保留完整 line 提取路径作为 reference 形态和对照 oracle；热路径 `compute_direction_shape` 使用局部读取路径，并用内部测试逐点对比新旧结果

### 还没有完成

- movegen / eval / VCF / VCT 的细粒度双端差分覆盖
- Python fallback 与 Cython 加速路径的系统性交叉验证
- Lazy SMP 的策略重设计；当前朴素 helper 填表没有稳定性能收益
- GUI 仍是轻量本地 Web UI，不是 reference pygame GUI 的逐行为复刻；后续可继续补局面保存、更多参数面板和引擎日志面板

也就是说，**当前已经有单线程 classic 搜索主链、战术层、Gomocup stdin/stdout 入口、本地 Web GUI、一轮 zhou 对战对齐验证、root 搜索层面的最小双端差分脚手架，以及可选 Lazy SMP 实验路径；但还没有覆盖 eval / movegen / VCF / VCT 的完整差分体系，Lazy SMP 也还不是可推荐的主线性能方案。**

## 当前实现原则

项目当前遵循这些原则：

- 以外部 Python reference `pygomoku` 作为唯一主语义基准
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
- 当前实测显示朴素 Lazy SMP helper 在 `d6/w20` 和早期 `d8/w30` smoke 下没有稳定提速，主要价值是保留并行架构边界和验证工具

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
│   │   ├── gomocup_engine.rs
│   │   └── gomoku_gui.rs
│   ├── types.rs
│   └── zobrist.rs
├── tests/
├── cases/
│   └── diff/
├── opponent/
│   └── zhou/
├── data/
│   ├── reference_text/
│   └── static/
├── scripts/
│   ├── compare_diff_outputs.py
│   ├── diff_reference.py
│   ├── extract_static_data.py
│   ├── run_engine_match.py
│   └── run_diff.py
```

其中：

- Python reference 不随本仓库提交；默认可通过 `PYGOMOKU_REF_ROOT=/path/to/pygomoku` 指定，本机约定路径为 `~/python_ws/pygomoku`
- `opponent/zhou/`：保留在本仓库内的 zhou 基线对手
- `data/reference_text/`：从 reference vendoring 进来的源文本副本
- `data/static/`：从 vendored 文本提取出的纯静态数据文件
- `scripts/extract_static_data.py`：负责重新生成和校验静态数据
- `cases/diff/`：reference / Rust 双端差分固定局面
- `src/bin/diff_probe.rs`：Rust 侧差分探针
- `src/bin/gomocup_engine.rs`：Gomocup stdin/stdout 协议入口
- `src/bin/gomoku_gui.rs`：本地 Web GUI 人机对战入口
- `scripts/diff_reference.py`：Python reference 侧差分探针
- `scripts/run_diff.py`：批量运行差分 case
- `scripts/run_engine_match.py`：Rust / Python reference 的固定开局 Gomocup 对战脚本

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
PYGOMOKU_REF_ROOT=~/python_ws/pygomoku python3 scripts/run_diff.py --profile all --jobs 10
```

当前全量测试通过规模：188 个 Rust 测试通过。

Gomocup CLI smoke：

```bash
printf 'START 15\nBEGIN\nEND\n' | cargo run --quiet --bin gomocup_engine -- --depth 2 --width 8
```

本地 Web GUI：

```bash
cargo run --release --bin gomoku_gui
```

启动后打开 `http://127.0.0.1:7878`。GUI 支持选择执黑/执白、点击执黑/执白重新开局、悔棋、异步引擎思考、棋盘轮询刷新、棋子手数显示、最后一手红色标记和参数/状态信息面板。快捷键：`U` 悔棋，`R` 按当前执棋方重新开局。默认使用当前配置 `depth=8,width=40,root_vct_depth=8`；如果需要更快的交互 smoke，可用：

```bash
cargo run --release --bin gomoku_gui -- --depth 6 --width 20
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
- 早期 `d8/w30`、同开局 `(7,7)`、Rust 执黑对 zhou 的单局 smoke 中，Lazy2 与串行棋谱一致，但 Rust 平均耗时略慢。
- 更早的 Lazy8 / 多开局 smoke 出现过走法漂移，说明共享 TT 会改变主线程 move ordering；这是 experimental 模式允许的行为，但不能用于 reference 等价验证。
- 后续如果继续推进并行，应先重设计 helper 写入策略或使用更保守的后台分析模式，而不是继续增加 worker 数。

串行热点优化记录：

- 基准 case：`root_deep_opening_10_4_d8_w15.json`
- 校验字段：move `(9,4)`、score `-14`、depth `8`、nodes `122196` 始终不变
- 初始 release probe：约 `1.330s`
- `getmi` 每候选预计算后：约 `0.968s`
- eval cache 固定数组化与 line 栈数组化后：约 `0.897s`
- `value_wide_compute` 的 `comp` 固定数组化后：约 `0.842s`
- `shape_raw_from_cells` 小循环手写后：约 `0.806s`
- `compute_direction_shape` 改用局部 point shape 读取后：约 `0.765s`
- 局部 point shape 读取继续改为直接按逻辑 line index 扫描后：最新单次 release probe 约 `0.63s`
- 旧完整 line 提取函数仍保留，不在热路径上；它用于保留 reference 形态，并作为局部读取路径的对照 oracle，降低斜线边界和 sentinel 语义漂移风险

Gomocup / zhou 对战验证：

- 参数：Rust/reference `depth=6, width=20, root_vct_depth=4`，zhou `depth=5`
- 说明：这是显式对齐 reference runtime 的历史验证；当前 Rust 默认运行参数已提升到 `depth=8, width=40, root_vct_depth=8`，后续对战验证必须记录 depth/width/VCF/VCT depth
- 开局：reference 的 9 个固定开局
- 颜色：黑白双边，共 18 线
- 结果：Rust 18 胜，reference 18 胜，18/18 完整走法序列一致
- 输出样例位置：`/tmp/rust_gomoku_fixed_vs_zhou_9_black.json`、`/tmp/rust_gomoku_fixed_vs_zhou_9_white.json`

Reference / Rust 差分脚手架：

```bash
cargo run --quiet --bin diff_probe -- --case cases/diff/root_center_11.json > /tmp/rust_diff.json
PYGOMOKU_REF_ROOT=~/python_ws/pygomoku python3 scripts/diff_reference.py --case cases/diff/root_center_11.json > /tmp/ref_diff.json
python3 scripts/compare_diff_outputs.py --rust /tmp/rust_diff.json --reference /tmp/ref_diff.json
```

`scripts/diff_reference.py` 和 `scripts/run_diff.py` 查找 reference 的顺序是：显式 `--ref-root`、环境变量 `PYGOMOKU_REF_ROOT`、旧本地路径 `./reference/pygomoku`、本机约定路径 `~/python_ws/pygomoku`。完整 Python reference 不提交进本仓库。

批量运行全部差分 case：

```bash
PYGOMOKU_REF_ROOT=~/python_ws/pygomoku python3 scripts/run_diff.py --profile all
```

日常快速差分默认只跑 fast case，可以并行执行：

```bash
python3 scripts/run_diff.py --jobs 10
```

只跑 slow case：

```bash
python3 scripts/run_diff.py --profile slow --jobs 10
```

Rust / Python reference 固定开局 Gomocup 对战：

```bash
cargo build --release --bin gomocup_engine
python3 scripts/run_engine_match.py \
  --opening-set 9 \
  --jobs 18 \
  --output /tmp/rust_vs_reference_9_openings.json
```

默认设置是 Rust 使用当前 engine 默认参数，Python reference 使用 `--depth 6 --width 20` 且 `INFO root_vct_depth 4`。脚本每手用 `BOARD` 全量同步局面，便于稳定复现；默认单手超时 `120s`、单局超时 `900s`，传 `--move-timeout-sec 0 --game-timeout-sec 0` 可关闭。最近一次 9 开局黑白双边 18 局结果：Rust 17 胜 1 负，唯一败局是 `[4,4]` 开局 Rust 执白。

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
- 当前重点仍然是“语义迁移 + 可验证优化”，不是重新设计搜索算法
- 串行主线已经做过一轮低风险热点优化，但更激进的候选范围、VCT 验证深度或并行策略仍需单独对战验证
- Rust 默认固定搜索 `depth=8, width=40, root_vct_depth=8` 是明确的运行参数偏离；带时间控制时搜索上限为 `depth=25, width=40`，实际完成深度由时间提前停止决定；需要与 reference 做严格差分时，应在 case 或协议 `INFO root_vct_depth 4` 等参数中显式设成 reference 对应值
- 已有最小 reference / Rust 差分脚手架，但覆盖范围还只到 root 搜索层面
- 本地 Web GUI 是用于人机对战和调试的 Rust 侧集成入口，不作为 reference pygame GUI 的语义等价证明
- `RootTrace.vct_ms` 是耗时调试字段，不应纳入确定性回归断言
- `root.nodes` 只应在单线程兼容模式下强断言；并行或优化模式应改为比较 move/score/depth/trace 等语义字段
- root tactical fast path 当前仍应保持 VCF 优先于 VCT，VCT 只在 trigger 命中后运行
- Lazy SMP 已有 experimental 执行路径，但默认关闭；当前实测没有稳定提速，且共享 TT 可能改变 move ordering，因此不作为 reference 等价路径

## 下一步

下一步不建议继续大面积扩搜索算法，应先把差分测试覆盖面补到能支撑后续优化和并行。

### 1. Root 搜索后续补强

1. 继续从 18 线 zhou 对战关键 ply 抽样，补充更多接近真实对局的 root 差分局面。
2. 检查 `RootTrace` 是否还需要补充协议或日志层会用到的字段，但不要把耗时字段纳入确定性回归断言。
3. 继续核对 root search 与 `$PYGOMOKU_REF_ROOT/pygomoku/search/root.py` 的结构差异，只保留有明确理由的差异。
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

### 5. 并行与性能后续

1. 保留单线程兼容路径作为默认 baseline，继续用 `nodes` 强断言守住 reference 对齐。
2. Lazy SMP 维持 experimental：只在无 `node_limit / time_limit` 时可启用，结果不要求与串行逐节点一致。
3. 先分析 helper TT 写入策略和主线程 TT 命中收益，再决定是否继续推进 Lazy SMP；不要简单增加 worker 数。
4. 已放弃 root-split full-window 方案，除非重新设计共享 alpha / PV 顺序，否则不再作为主方案。
5. 若尝试新的并行策略，必须补重复运行稳定性测试和 zhou 18 线对战对比。
6. 更现实的短期优化方向是继续做串行热点 profiling、差分夹具扩展和局部数据结构优化。
7. `evaluate_board_main` 的全局评估增量化风险较高，暂不建议直接替换主线；如果后续要做，应先以 shadow mode 同时计算 full scan 和 incremental summary，并用强断言长期验证完全一致。

## 一句话总结

当前仓库已经完成了 **基础状态机 + 配置 + pattern + eval + 单线程 classic 搜索 + VCF/VCT 战术路径 + Gomocup 协议入口 + 本地 Web GUI** 的 Rust 化，并建立了对应回归测试、一轮 zhou 对战对齐验证、root 搜索层面的 reference / Rust 差分脚手架，以及默认关闭的 Lazy SMP 实验路径；下一阶段重点是 **扩展差分覆盖、继续可验证的串行热点优化，并谨慎评估并行策略是否值得进入主线**。
