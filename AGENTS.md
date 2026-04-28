# AGENTS.md

## 项目目标

`rust_gomoku` 是 Python reference 项目 `pygomoku` classic 主线的 Rust 重构。当前主线优先级是：

1. 守住已经完成的 classic 语义。
2. 扩大 reference/Rust 差分覆盖。
3. 在不降低棋力的前提下优化平均耗时和长尾耗时。

完整 Python reference 不随仓库提交；本机约定路径为 `~/python_ws/pygomoku`，也可用 `PYGOMOKU_REF_ROOT` 指定。仓库内 `opponent/zhou` 只是轻量对手，不是语义基准。

## 进入仓库后先读

1. `AGENTS.md`
2. `README.md`
3. 本次相关 Rust 模块、测试和 diff case
4. 如需核对 reference，再读 `$PYGOMOKU_REF_ROOT` 或 `~/python_ws/pygomoku` 下的对应模块

具体命令、默认参数和目录说明以 `README.md` 和 `src/config.rs` 为准，本文件不重复维护参数表。

## Base 规则

base 是默认主线，优先行为一致和可回归。

- 固定局面、固定深度、固定宽度下，默认不改变 best move、score、depth、tactical trace。
- 当前 root 差分兼容模式下，默认也不改变 nodes。
- 不擅自改变候选排序、TT 读写语义、VCF/VCT 触发顺序、fallback RNG。
- 候选排序成本优化应优先保持排序 key 完全一致；改变排序策略属于 fast/实验，不应混入 base。
- root tactical fast path 保持 VCF 优先于 VCT；VCT 只在 trigger 命中后运行。
- 如果优化导致 base 固定 case 的 move/score/nodes 变化，必须停下来说明原因；除非用户明确批准，否则回退或转入实验分支。

## Fast 和实验规则

fast/profile 或实验分支用于尝试激进优化，但不能以棋力下降为代价。

- 当前 `fast` profile 默认开启第三版 history/killer ordering；base 仍关闭。该策略只应通过 fast 路径使用，并保留 `--no-fast-history-ordering` / `INFO fast_history_ordering 0` 回退。
- 可以尝试 TT 策略、现代 pruning/reduction/ordering/window、并行、线程池和 `unsafe` 热点优化。
- 可以不等价 base 的固定局面结果，但必须保留可回退到 base 的路径。
- fast 对 base 胜率不能低于 `50%`。
- 必须有可见性能收益，至少体现在 avg、p95、max 单手耗时之一。
- reference 和 zhou 只适合作 smoke；fast 棋力主验证应是 fast vs base。

fast 实验合入或设为推荐配置前，至少要报告胜率、avg、median、p95、max、错误率和超时率。

## 核心语义

- 坐标统一为 `(x, y)`，即 `(列, 行)`。
- `Move` 是扁平化索引，坐标转换优先使用统一辅助函数。
- 常量语义保持：`BOARD_SIZE = 15`，`BLACK = 1`，`WHITE = -1`，`EMPTY = 0`。
- 搜索主流程通过 `play / undo` 修改局面。
- `winner / side_to_move / move_history / move_count / zobrist_key` 必须同步。
- 嵌套 `play / undo` 必须严格配对，尤其是 threat board、VCF、VCT。
- `BOARD` 重建、复制、快照恢复必须支撑搜索和测试。

## 高风险区域

以下改动必须先设计验证，不能只凭直觉修改：

- movegen 或候选排序。
- TT probe/store/replacement/capacity。
- alpha-beta 窗口、PVS、depthdown、root bonus。
- VCF/VCT 搜索顺序、memo key、验证深度。
- `evaluate_board_main` 或 eval cache 增量语义。
- Gomocup `INFO` 对 runtime config 和 limits 的影响。
- 并行搜索或共享状态。
- `unsafe`。

已验证不适合默认主线的方向包括 root full-window split、Lazy SMP helper 填表、root YBWC 和 aspiration window。不要把这些旧方案直接塞回默认路径。

## 并行边界

base 当前只允许默认关闭的 `overlap_vct_alphabeta` 实验：

- VCF 仍同步优先。
- 只在固定搜索、无 node/time limit、VCF miss 且 VCT trigger 命中后启用。
- VCT accepted 时取消并丢弃 alphabeta。
- VCT miss/rejected 时等待并采用 alphabeta。
- alphabeta worker 使用独立 TT snapshot，不共享写入主 TT。

GUI 入口可以为手动对局体感单独默认开启该开关；这不代表 Gomocup、diff、case probe 或库默认开启。

新的并行方案优先进入 fast 或独立分支。base 并行必须证明结果等价；fast 并行必须用 fast vs base 证明胜率和速度。

## 验证要求

- 文档或注释改动：至少跑 `git diff --check`。
- 普通 Rust 改动：跑格式、单元测试和相关二进制构建。
- eval/movegen/ordering/TT/alphabeta/root 改动：增加 root diff。
- VCF/VCT 改动：增加对应 diff case 和慢手复现。
- protocol/GUI 改动：增加 smoke、transcript 或手动说明。
- base 默认参数、候选顺序、搜索窗口、TT 语义改动：增加 reference 对战或等价固定局面回归。
- fast 实验：至少跑 fast vs base smoke；推荐配置前扩大局数。

如果任务涉及搜索、评估、候选、TT、VCF/VCT，不能只跑 `cargo test`。

## 性能工作格式

每次性能工作至少说明：

- 热点证据和目标。
- 修改范围。
- 净收益假设：预计减少了什么开销、额外增加了什么开销、两者是否可能抵消。
- 止损条件。
- 正确性验证。
- 耗时对比。
- 是否改变 move、score、nodes、trace。
- 如果是 fast，还要说明 fast vs base 结果。

性能设计不能只计算“少做了多少工作”，还要计算新增验证、同步、拷贝、缓存失效、线程调度、重搜和额外分支的成本。没有稳定净收益的复杂实验应回退或默认关闭，不要留在主线增加维护成本。

## 提交前检查

- 没有误加入完整 reference、大型对战 JSON、profiling 输出或本地备份。
- 默认参数只在 `src/config.rs` 集中修改，并同步 README。
- 涉及 reference 语义时，已说明对齐 reference、修正 reference 问题，还是进入实验路径。

一句话准则：base 守住 `pygomoku` classic 语义，性能优化必须证明净收益；fast 可以不等价，但对 base 胜率不能低于 `50%`。
