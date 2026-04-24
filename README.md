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
- reference 静态数据提取脚本
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
- alphabeta 的终局分数、深度、node limit、deadline、fallback 行为
- root 搜索的空盘中心、动态棋盘窗口、classic fallback RNG、固定开局回归
- threat board 的威胁点、A4/B4/A3 判定、嵌套 `play/undo` 恢复不变量
- VCF 的 begin depth 映射、序列 key、forcing threat 搜索
- VCT 的 trigger、OR/AND 搜索、memo 清理、root 接入与验证

### 还没有完成

- Gomocup 协议入口
- 命令行引擎入口
- GUI 或其它外部集成
- 与 `opponent/zhou` 的对战验证链路
- reference / Rust 双端差分测试脚手架
- Python fallback 与 Cython 加速路径的系统性交叉验证
- 并行搜索执行路径

也就是说，**当前已经有单线程 classic 搜索主链和战术层雏形，但还不能作为完整外部引擎使用。**

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
- root 搜索已经有清晰的 tactical fast path 和 alphabeta path，后续可以在 root candidates 层拆分任务
- 后续搜索更适合走“线程本地局面与缓存 + 稳定归并 + 可选共享 TT”的路线
- 默认必须保留稳定的单线程兼容模式
- 第一版并行不应默认共享 TT；应先做确定性的 root-split，再评估共享 TT、取消机制和时间控制

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
│   ├── types.rs
│   └── zobrist.rs
├── tests/
├── data/
│   ├── reference_text/
│   └── static/
├── scripts/
│   └── extract_static_data.py
└── reference/
    └── pygomoku/
```

其中：

- `reference/pygomoku/`：当前语义基准
- `data/reference_text/`：从 reference vendoring 进来的源文本副本
- `data/static/`：从 vendored 文本提取出的纯静态数据文件
- `scripts/extract_static_data.py`：负责重新生成和校验静态数据

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

这些测试既包含基础行为，也包含 handpicked 精确值回归和固定局面搜索回归。

当前验证命令：

```bash
python3 scripts/extract_static_data.py --check
cargo test
```

当前全量测试通过规模：140 个 Rust 测试通过。

## 当前限制

需要明确：

- 当前还没有协议入口和对外运行入口，因此仓库还不能当作完整引擎使用
- 当前重点仍然是“语义迁移”，不是性能冲刺
- 某些实现虽然已经有增量路径，但整体仍未进入最终优化阶段
- 目前主要靠 Rust 侧固定局面和手工提取的 reference 语义回归，还没有完整自动差分测试
- `RootTrace` 当前没有包含 reference 中的 `vct_ms` 时间字段；这是 trace 形状差异，不影响棋局语义，但需要在协议或调试输出前决定是否补齐
- root tactical fast path 当前仍应保持 VCF 优先于 VCT，VCT 只在 trigger 命中后运行
- 多线程只完成架构预留，没有实现执行路径

## 下一步

下一步不建议继续大面积扩搜索算法，应先收口可运行入口和差分验证。

### 1. 提交前收口

1. 更新 README 当前状态。
2. 检查 `git status`，确认 `reference/` 和 `.codex/` 不进入提交。
3. 运行 `python3 scripts/extract_static_data.py --check`。
4. 运行 `cargo test`。
5. 提交 Root VCT 接线、trace、嵌套 `play/undo` 测试和 README 更新。

### 2. Root 搜索小修正

1. 将 root 中 `recompute_all` 移到 VCF/VCT fast path 之后，减少与 reference `search/root.py` 的结构差异和无用计算。
2. 评估是否补 `RootTrace.vct_ms`。如果补，应避免把耗时字段纳入确定性回归断言。
3. 为 VCF 优先、VCT fallback 到 alphabeta、VCT reject reason 增加更接近 reference 的固定局面差分样例。

### 3. Gomocup 协议入口

1. 新增 `src/protocol/` 模块。
2. 对齐 `START`、`RESTART`、`BEGIN`、`TURN`、`BOARD`、`DONE`、`TAKEBACK`、`INFO`、`ABOUT`。
3. 重点测试非法输入、越界坐标、`BOARD` 重建、`INFO compute_vct/root_vct_depth` 配置变更。
4. 新增 `tests/protocol_alignment.rs`。

### 4. 命令行引擎入口

1. 新增 `src/bin/gomocup_engine.rs`。
2. 接入协议 reader/writer，不在入口层散落棋盘逻辑。
3. 保留 deterministic single-thread 默认模式。
4. 增加 smoke test 或协议 transcript 回归。

### 5. Reference / Rust 差分测试脚手架

1. 设计固定局面格式，统一表达 move list、side_to_move、limits、runtime config。
2. Python 侧调用 `reference/pygomoku` 输出 JSON。
3. Rust 侧输出同结构 JSON。
4. 先覆盖 board、eval、movegen、root search，再扩到 VCF/VCT。
5. 对发现的差异明确记录：Rust 对齐 Python fallback、Cython 路径，还是 reference 当前测试主线。

### 6. 并行架构第一版

1. 保留单线程兼容路径作为默认 baseline。
2. 新增 root candidate task 抽象，不改变候选顺序和最终归并规则。
3. 每个 worker 使用线程本地 `Board`、`EvalCaches`、`AlphaBetaSearcher`、`VCFSearcher`、`VCTSearcher`。
4. 第一版不共享 TT，只做确定性 root-split。
5. 加重复运行稳定性测试：同一局面、同一深度、多次运行返回同一 move、score、depth。
6. 后续再评估共享 TT、取消 token、统计聚合和时间控制。

## 一句话总结

当前仓库已经完成了 **基础状态机 + 配置 + pattern + eval + 单线程 classic 搜索 + VCF/VCT 战术路径** 的 Rust 化，并建立了对应回归测试；下一阶段重点是 **协议入口、差分验证和确定性并行架构落地**。
