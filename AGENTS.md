# AGENTS.md

## 项目目标

`rust_gomoku` 是 Python reference 项目 `pygomoku` classic 主线的 Rust 重构。

当前按两条线推进：

- `classic/base`：默认路径，保持确定、可回归，继续对齐 reference classic 语义。
- `fast`：性能实验路径，可以改变固定局面的 move/score/nodes，但对 base 胜率不能低于 `50%`，并且要有实际速度收益。

执行任务前先判断本次改动属于 base 还是 fast，不要把两条线的验收标准混在一起。

## 进入仓库后先读

1. `AGENTS.md`
2. `README.md`
3. 本次相关 Rust 模块、测试和 diff case
4. 如需核对 reference：`$PYGOMOKU_REF_ROOT/AGENTS.md` 或 `~/python_ws/pygomoku/AGENTS.md`

完整 Python reference 不随本仓库提交；本机约定路径为 `~/python_ws/pygomoku`。仓库内 `opponent/zhou` 只是轻量对手，不是语义基准。

## Base 规则

base 是默认主线，优先级是行为一致和可回归。

- 固定局面、固定深度、固定宽度下，默认不改变 best move、score、depth、tactical trace。
- 当前 root 差分兼容模式下，默认也不改变 nodes。
- 不擅自改变候选排序、TT 读写语义、VCF/VCT 触发顺序、fallback RNG。
- root tactical fast path 保持 VCF 优先于 VCT，VCT 只在 trigger 命中后运行。
- 默认固定搜索为 `depth=8,width=40,root_vct_depth=8,tt_bits=20,overlap_vct_alphabeta=false`。
- 与 reference 严格差分时通常显式使用 `depth=6,width=20,root_vct_depth=4`。

如果优化导致 base 固定 case 的 move/score/nodes 变化，必须停下来说明原因；除非用户明确批准，否则回退或归入 fast。

## Fast 规则

fast 目标是提速，但不能以棋力下降为代价。

- fast 可以尝试更大的 TT、不同 replacement policy、现代 pruning/reduction/ordering/window、并行搜索、线程池和 `unsafe` 热点优化。
- fast 可以不等价 base 的固定局面结果，但必须保留可回退到 base 的路径。
- fast 对 base 胜率不能低于 `50%`。
- fast 必须有可见性能收益，至少体现在 avg、p95、max 单手耗时之一。
- 当前 fast 额外启用 VCF 多合法应手验证；base 保持单一合法应手语义。
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

以下改动不要只凭直觉做，必须先设计验证：

- movegen 或候选排序。
- TT probe/store/replacement/capacity。
- alpha-beta 窗口、PVS、depthdown、root bonus。
- VCF/VCT 搜索顺序、memo key、验证深度。
- `evaluate_board_main` 或 eval cache 增量语义。
- Gomocup `INFO` 对 runtime config 和 limits 的影响。
- 并行搜索或共享状态。
- `unsafe`。

已验证不适合 base 的方向包括 root full-window split、Lazy SMP helper 填表、root YBWC 和 aspiration window。不要把这些旧方案直接塞回默认主线。

## 并行边界

base 当前只允许默认关闭的 `overlap_vct_alphabeta` 实验：

- VCF 仍同步优先。
- 只在固定搜索、无 node/time limit、VCF miss 且 VCT trigger 命中后启用。
- VCT accepted 时取消并丢弃 alphabeta。
- VCT miss/rejected 时等待并采用 alphabeta。
- alphabeta worker 使用独立 TT snapshot，不共享写入主 TT。

新的并行方案优先进入 fast 或独立分支。base 并行必须证明结果等价；fast 并行必须用 fast vs base 证明胜率和速度。

## 验证要求

具体命令以 `README.md` 为准，避免两份文档重复维护命令。

- 文档或注释改动：至少跑 `git diff --check`。
- 普通 Rust 改动：跑格式、单元测试和相关二进制构建。
- eval/movegen/ordering/TT/alphabeta/root 改动：增加 root diff。
- VCF/VCT 改动：增加对应 diff case 和慢手复现。
- protocol/GUI 改动：增加 smoke、transcript 或手动说明。
- base 默认参数、候选顺序、搜索窗口、TT 语义改动：增加 reference 对战或等价固定局面回归。
- fast 实验：至少跑 9 开局双边 18 局 fast vs base smoke；推荐配置前扩大局数。

如果任务涉及搜索、评估、候选、TT、VCF/VCT，不能只跑 `cargo test`。

性能和棋力要分开验证：

- `scripts/bench_match_cases.py`：同一批 `cases/match/*.jsonl` 前缀局面的一手搜索对照，用于判断 fast 是否真的减少 time/nodes。
- `scripts/run_gomocup_match.py`：真实对战，用于判断 fast 对 base 胜率是否不低于 `50%`。
- `cases/match/smoke_quick.jsonl`：快速冒烟集，用于崩溃、协议和长尾检查，不作为最终棋力结论。
- fast 优化至少应先过同局面 benchmark，再跑 fast vs base 对战。

## 性能工作格式

每次性能工作至少说明：

- 热点证据和目标。
- 修改范围。
- 净收益假设：预计减少了什么开销、额外增加了什么开销、两者是否可能抵消。
- 止损条件：哪些 benchmark / 对战结果说明该方向应回退或默认关闭。
- 正确性验证。
- 耗时对比。
- 是否改变 move、score、nodes、trace。
- 如果是 fast，还要说明 fast vs base 结果。

性能设计不能只计算“少做了多少工作”，还要计算新增验证、同步、拷贝、缓存失效、线程调度、重搜和额外分支的成本。没有稳定净收益的复杂实验应回退或默认关闭，不要留在主线增加维护成本。

## 文件约定

- `src/config.rs`：主要搜索默认参数。
- `src/board.rs`：核心状态机。
- `src/eval/`：评估和缓存。
- `src/search/`：movegen、ordering、TT、alphabeta、root。
- `src/threats/`：threat board、VCF、VCT。
- `src/protocol/`：Gomocup 协议。
- `src/bin/gomocup_engine.rs`：Gomocup 入口。
- `src/bin/gomoku_gui.rs`：本地 Web GUI。
- `src/bin/diff_probe.rs`：Rust 差分探针。
- `src/bin/case_probe.rs`：单局面搜索 benchmark 探针。
- `cases/diff/`：固定差分局面。
- `cases/match/`：fast/base 对战局面 JSONL。
- `scripts/run_diff.py`：批量差分。
- `scripts/run_engine_match.py`：Rust/reference 对战。
- `scripts/run_gomocup_match.py`：通用 Gomocup 对战，主要用于 fast vs base。
- `scripts/bench_match_cases.py`：同局面 base/fast 性能对照。
- `scripts/extract_match_cases.py`：从对战 JSON 抽取中盘对战 case。
- `README.bak.md`：本地备份文件，不要提交，除非用户明确要求。

## 工作流程

1. 判断任务类型和目标线：base 还是 fast。
2. 若涉及 reference 语义，读取 reference 对应模块。
3. 先明确验证范围，再改代码。
4. 修改后跑最小必要验证。
5. 验证失败且原因不清楚时，不要继续扩大改动。
6. 提交前确认没有误加入完整 reference、大型对战 JSON、profiling 输出或本地备份。

## 明确禁止

- 在 base 中为了速度改变默认 best move、score 或候选排序。
- 未验证就重写搜索主链。
- 把 zhou 行为当作 reference 语义。
- 修改 TT、movegen、ordering、VCF/VCT 后只跑编译。
- 把完整 Python reference 提交进本仓库。
- 宣称 fast 棋力不降但没有 fast vs base 对战数据。

一句话准则：base 守住 `pygomoku` classic 语义，fast 负责用现代办法提速；fast 可以不等价，但对 base 胜率不能低于 `50%`。
