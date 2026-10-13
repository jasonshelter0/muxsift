# MuxSift

**Acquire, process, and distill media.**

MuxSift 是一个面向 Linux 的 Rust CLI/TUI 媒体工作流工具。它的目标不只是“下载视频”，而是把不同网站的媒体获取、文件保留、音视频后处理和 AI 内容整理统一成一条可恢复、可扩展、可审查的流水线。

## 愿景

```text
来源适配器
  当前：Bilibili 原生后端
  未来：yt-dlp 及其他来源
        │
        ▼
下载并生成媒体工件
  视频 / 音频 / 原生字幕 / ASR 字幕 / 元数据
        │
        ▼
保留策略
  决定原始流、中间文件和最终文件是否保留
        │
        ▼
后处理
  当前：音频封装、音视频合并、Bcut/Buzz ASR
  未来：关键帧提取及更多媒体处理器
        │
        ▼
AI 提炼
  字幕校对、结构化 Markdown、摘要及其他可配置输出
```

长期设计应把每一步视为具有明确输入、输出和状态的阶段。来源后端只负责解析资源和提供可下载工件；后处理器消费已经生成的工件；保留策略决定哪些中间结果在成功后清理；失败任务从最近一个完整阶段继续，而不重复已经成功的网络或计算工作。

## 当前进度

目前的程序已经可以作为完整的 Bilibili CLI/TUI 下载器使用，但通用流水线接口仍在设计中。

已完成：

- Rust workspace，由 `muxsift-core`、`muxsift-cli` 和 `muxsift-tui` 共用同一套协议与任务逻辑。
- BV、完整视频 URL 和 `b23.tv` 短链解析。
- QR 登录、凭据保存、登出，以及 Linux XDG 配置目录。
- 单 P、多 P、UGC 合集和嵌套 episodes/pages 解析与选择。
- DASH 画质和编码选择、视频与音频下载、FFmpeg 无损合并，以及 M4A 音频输出。
- aria2 常驻 RPC 下载后端和原生 HTTP 后端；支持并发、暂停、恢复和取消。
- 原生字幕导出为 SRT/WebVTT。
- Bcut 云端 ASR 与 Buzz/faster-whisper 本地 ASR，支持 SRT/TXT/LRC/JSON。
- Buzz 转写进度、已处理媒体时间和 ETA 在 CLI/TUI 中实时显示。
- OpenAI-compatible AI 整理：保留原始 SRT，并输出同名 Markdown。
- 阶段检查点：下载、FFmpeg、ASR 或 AI 失败后，可用 `--resume` 或 TUI 的 `r` 从最近成功阶段继续。
- 已存在 SRT 时可跳过下载与 ASR，直接继续 AI 整理。
- 中文默认 TUI、English/中文切换、CJK 光标宽度、粘贴和任务树。
- Ubuntu x86_64 发行包，包含 `aria2c`、`ffmpeg`、Buzz bridge、许可证和双语指南。

尚未完成：

- 把现有 Bilibili 实现收敛为稳定的通用 `Source`/adapter 接口。
- yt-dlp 来源适配器。
- 用工件图统一描述视频、音频、字幕、关键帧和 AI 输出之间的依赖。
- 用户可配置的中间文件保留策略。
- 视频关键帧提取处理器。
- 让 AI 总结可以选择字幕、关键帧、元数据或它们的组合。
- 通用任务清单、来源与处理参数记录，以及可迁移的流水线配置格式。

## 待讨论的设计问题

1. **流水线还是工件图**：ASR、关键帧和原生字幕可以并行产生，内部模型更适合 DAG；CLI 可以继续给用户呈现简单的线性流程。
2. **保留策略**：需要区分原始下载、可复用中间工件和最终输出，并定义任务成功、失败、取消时各自的清理规则。
3. **来源能力**：不同网站能提供的画质、字幕、合集结构和认证方式不同，adapter 应通过能力声明暴露差异。
4. **后处理契约**：处理器应声明输入格式、输出工件、进度、取消方式和可恢复边界。
5. **AI 输入与隐私**：本地处理和远程 Provider 必须明确区分；用户应能看到哪些工件会被发送到远程服务。
6. **兼容方式**：现有 CLI/TUI 行为应逐步映射到新模型，避免一次性重写造成已有功能回退。

## 快速开始

解压 Linux x86_64 发行包后：

```bash
./release/start-tui.sh
./release/muxsift --version
./release/muxsift login
./release/muxsift BV1TXtb6iEt4 --dry-run --json
```

发行包自带 `bin/aria2c` 和 `bin/ffmpeg`。也可以通过 `--aria2`、`--ffmpeg` 显式指定其他可执行文件。

配置和登录凭据使用 XDG 配置目录，通常位于：

```text
~/.config/muxsift/
```

运行 `muxsift config show` 查看当前实际路径。AI API key 只从配置指定的环境变量读取，不写入 JSON。

更多当前操作说明见 [中文指南](USER-GUIDE.zh-CN.md) 和 [English guide](USER-GUIDE.md)。

## 开发

### 本地音视频转写与 AI 整理

构建后可直接处理已有视频或音频文件。CLI 的 `local` 命令会用 FFmpeg 提取／转换音频，经 Buzz 本地 ASR 或 Bcut 云端 ASR 生成 SRT；启用 AI 配置后还会生成同名 Markdown。

```bash
cargo build -p muxsift-cli --locked
target/debug/muxsift local ./meeting.mp4 --asr bcut -o ./notes
target/debug/muxsift local ./meeting.wav --asr buzz --buzz-project /path/to/buzz -o ./notes
```

Bcut 需先运行 `target/debug/muxsift login`，并会上传音频。Buzz 的 `uv`、模型等参数可在 JSON 配置的 `asr.buzz` 下设置。AI 需在配置中设置 `ai.enabled: true`、`ai.model`，并导出 `ai.api_key_env` 指定的环境变量；查看路径用 `target/debug/muxsift config show`。仅生成 SRT 可加 `--transcript-only`，AI 失败后可加 `--resume` 复用已有 SRT。

TUI 中按 `a` 输入本地文件或文件夹路径；文件夹会递归打开目录树选择器，勾选后为每个文件建立任务。按 `c` 打开统一设置窗口（转写后端、Buzz 路径、AI 和输出目录等），按 `C` 可直达 AI 分类；按 `A`、`S` 可启动全部任务，失败后按 `r` 继续。批量输出保留输入目录层级。

```bash
cargo fmt --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets -- -D warnings
```

生成 Linux 发行包：

```bash
./scripts/fetch-linux-tools.sh  # 仅在需要刷新随包工具时运行
./scripts/package-linux.sh
```

核心目录：

```text
crates/muxsift-core/   来源协议、认证、下载、ASR、AI、检查点
crates/muxsift-cli/    clap CLI、配置与业务编排
crates/muxsift-tui/    ratatui/crossterm TUI
scripts/               Linux 打包、工具获取与 Buzz bridge
distribution/          发行模板、配置样例和第三方说明
third_party/           随 Linux 发行包提供的独立运行时与许可证
```

## Tribute

MuxSift 起步于 **BILIDOWN-CLI** 的独立 Rust 实现。它验证了 Bilibili 下载、CLI/TUI、ASR、AI 整理和 Linux 便携发行的完整链路。新的项目名称感谢这段基础工作，同时把后续方向扩展为多来源、可组合的媒体处理工具。项目不包含或调用原 Electron 应用及其专有代码。
