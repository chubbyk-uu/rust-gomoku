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

本仓库目前仍处于早期阶段，但已经不再是纯空骨架。

### 已完成

- Rust crate 基础骨架
- `constants`
- `types`
- `move` 编码 / 解码
- `board`
- `zobrist`
- `config`
- `patterns` 的基础静态语义
- `eval/caches`
- `eval/local`
- `eval/global_eval`
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

### 还没有完成

- `search/movegen`
- `search/ordering`
- `search/tt`
- `search/alphabeta`
- `search/root`
- `threats/threat_board`
- `vcf / vct`
- Gomocup 协议入口
- 命令行引擎入口
- 并行搜索执行路径

也就是说，**当前已经有基础层和评估层，但还没有搜索器。**

## 当前实现原则

项目当前遵循这些原则：

- 以 `reference/pygomoku` 作为唯一主语义基准
- 先保证单线程、确定性、可回归
- Rust 代码从一开始就考虑未来并行架构边界
- 但默认执行路径先不因为并行而改变 reference 结果
- 先做可验证等价实现，再谈性能优化

当前的并行设计立场是：

- `Board` 和 `EvalCaches` 保持可快照、可恢复
- 后续搜索更适合走“线程本地局面与缓存 + 明确共享接口”的路线
- 默认必须保留稳定的单线程兼容模式

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

这些测试既包含基础行为，也包含 handpicked 精确值回归。

## 当前限制

需要明确：

- 当前还没有搜索器，因此仓库还不能当作完整引擎使用
- 当前也还没有协议入口和对外运行入口
- 当前重点仍然是“语义迁移”，不是性能冲刺
- 某些实现虽然已经有增量路径，但整体仍未进入最终优化阶段

## 下一步

按当前迁移顺序，接下来应优先进入：

1. `search/movegen`
2. `search/ordering`
3. `search/tt`
4. `search/alphabeta`
5. `search/root`

然后再继续：

1. `threat_board`
2. `vcf`
3. `vct`
4. Gomocup 协议入口

## 一句话总结

当前仓库已经完成了 **基础状态机 + 配置 + pattern + eval 主链** 的 Rust 化，并建立了对应回归测试；但 **搜索层、战术层和协议层仍未开始或尚未落地**。
