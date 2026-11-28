# rust_gomoku

`rust_gomoku` 是一个用 Rust 编写、以高棋力为目标的 15×15 五子棋引擎，支持自由五子棋（Freestyle）和连珠（Renju）。项目重点是实战限时下的棋力、规则正确性和搜索响应。

[English](README.md) · [下载](https://github.com/chubbyk-uu/rust-gomoku/releases/latest) · [文档目录](docs/README.md)

- **搜索** — 原生 `Mix9svq` NNUE、SIMD 加速、增量棋盘评估、战术搜索和 Lazy SMP。经典搜索可在缺少权重时运行，也是休闲模式的基础。
- **对弈** — 桌面浏览器界面和 Android ARM64 应用，支持人机与本地双人对弈。专业模式可切换单手用时，并显示 NNUE 胜率估计曲线。
- **接入** — `gomocup_engine` 通过标准输入输出提供 Gomocup 协议。

## 下载和对弈

在 [GitHub Releases](https://github.com/chubbyk-uu/rust-gomoku/releases/latest) 下载 Windows x64 版本或 Android ARM64 APK。Android 也提供用于分发的 AAB。

![桌面界面的连珠对局](docs/screenshots/desktop-gui.png)

<p>
  <img src="docs/screenshots/android-portrait.png" alt="Android 竖屏连珠对局" width="280">
  <img src="docs/screenshots/android-difficulty.png" alt="Android 专业和休闲模式设置" width="280">
</p>

[Android 横屏界面](docs/screenshots/android-landscape.png)

专业模式默认单手 1 秒，也可选 0.5、2.5 或 5 秒。休闲模式提供入门、普通、高级和大师四档。连珠对局中，界面会标记并拦截黑方禁手。支持的规则和开局范围见[连珠指南](docs/renju-forbidden-design.md)。

## 构建和运行

```bash
cargo build --release --locked
cargo test --locked
cargo run --release --bin gomoku_gui
```

桌面界面会打开 `http://127.0.0.1:18080`。可在运行命令后加 `-- --nnue-dir /path/to/weights` 指定专业模式权重。文件名和查找顺序见[引擎指南](docs/engine.md#nnue-weights)。

启用 NNUE 的 Gomocup 引擎：

```bash
target/release/gomocup_engine --nnue-dir /path/to/weights
```

协议参数和冒烟检查见 [Gomocup 指南](docs/gomocup.md)。Android 构建与签名见 [Android 指南](docs/android-app-design.md)。

## 棋力评测

2026-11-28 的开发构建与 Rapfi 使用相同权重，在单手 1 秒、两种规则及 1T/4T 的 **600 局中得分 50.92%**，95% 区间为 **49.25%–52.58%**。各组成绩和 Renju 1T 复跑详见[性能与棋力日志](docs/perf-log.md)。

回归及对战命令见[评测指南](docs/testing.md)，当前工作重点见[引擎路线图](docs/optimization-roadmap.md)。

## 许可证

项目采用 [GPL-3.0-or-later](LICENSE)。网络权重是独立资源，许可见对应发布来源。
