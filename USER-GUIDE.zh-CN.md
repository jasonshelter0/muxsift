# MuxSift 使用指南

## 快速开始

在 Ubuntu/Linux x86_64 解压发行包后，运行 `./release/start-tui.sh`，或在终端运行 `./release/muxsift tui`。发行包的 `bin` 目录已包含 aria2 和 FFmpeg，无需单独安装；首次界面为空时按 `a` 添加 BV 链接或 `b23.tv` 短链。请关闭中文输入法并使用英文键盘输入快捷键。Pages 页面可勾选分 P，`Space` 选择，`Enter` 确认。

登录：TUI 按 `l` 显示二维码；或运行 `./release/muxsift login`。按 `L` 或 `./release/muxsift logout` 清除本地凭据。

## 常用命令

```bash
./release/muxsift BV1xxxx --quality 1080p -o "$HOME/Videos"
./release/muxsift BV1xxxx --audio-only -o "$HOME/Music"
./release/muxsift BV1xxxx --subtitle --subtitle-format srt
./release/muxsift BV1xxxx --transcribe --transcript-format srt
./release/muxsift BV1xxxx --pages all
./release/muxsift BV1xxxx --dry-run --json
```

## TUI 快捷键

`a` 添加，`Space` 勾选，`S` 启动已勾选，`Enter` 启动当前项，`x` 取消，`P` 暂停/继续，`r` 从最近成功阶段继续失败或已取消任务，`d` 删除，`m` 切换模式，`b` 切换下载后端，`B` 切换 Bcut/Buzz 转写后端，`c` 输出目录与 Bcut 间隔，`C` AI 设置，`g` 中英文切换，`h` 查看完整帮助。

## 本地视频、音频转写与 AI 整理

`local` 命令先通过 FFmpeg 提取／转换音频，再生成 SRT；启用 AI 配置后，还会把 SRT 整理为同名 Markdown：

v0.2.0新增场景候选帧命令：从源码构建后运行 `target/debug/muxsift frames ./lecture.mp4 -o ./lecture-frames`。它输出 `frames/` 截图、`timeline.json` 时间线、`frames.md` 图文索引和可搜索字幕的离线 `frames.html`；`--interval`、`--threshold`、`--min-gap` 可调整采样、变化敏感度和最小间隔。加 `--srt ./lecture.srt` 可把字幕按开始时间关联到最近的前一张候选帧，并写入全部索引。该功能尚未包含在 v0.1.0 发行包中，不能保证识别所有 PPT 动画或公式增量。输出目录需不存在。

v0.2.0可用独立安装的 yt-dlp 处理单条网页视频，继续 Buzz/Bcut 转写和 AI 整理：`target/debug/muxsift web 'https://www.youtube.com/watch?v=VIDEO_ID' -o ./web-notes --asr buzz --buzz-project ~/Projects/buzz`。仅下载可省略 `--asr`；只生成 SRT 可加 `--transcript-only`；失败后加 `--resume` 复用已完成的媒体与后处理阶段。输出分放在 `media/` 和 `notes/`。可用 `--yt-dlp /路径/yt-dlp` 指定程序，或在配置的 `download.yt_dlp` 中设置。TUI 先按 `m` 选 Video 或 Transcript，再按 `a` 粘贴非 Bilibili 网页 URL；按 `c` 可设置 yt-dlp 路径，失败后按 `r` 继续。当前不处理整张播放列表，也不读取 yt-dlp 用户配置。该入口不改变 Bilibili 原生下载功能，也尚未包含在 v0.1.0 发行包中。

```bash
./release/muxsift local ./meeting.mp4 --asr bcut -o ./notes
./release/muxsift local ./meeting.wav --asr buzz --buzz-project /path/to/buzz -o ./notes
```

Bcut 需要先 `login`，并会上传音频；Buzz 在本地转写。运行 `config show` 查找 JSON 配置，设置 `ai.enabled` 为 `true`、填写 `ai.model` 和 `ai.base_url`，再导出 `ai.api_key_env` 指定的环境变量（默认 `OPENAI_API_KEY`）。只需 SRT 时加 `--transcript-only`；失败后加 `--resume`，可复用已提取的音频或已有 SRT。同名输出文件不会被覆盖。

TUI 按 `a` 输入本地视频、音频文件或文件夹路径。输入文件夹时会递归扫描非空的常见音视频文件，按目录树打开选择窗口：方向键移动，空格勾选文件或整个目录，`/` 搜索，`A` 全选，`E` 折叠/展开，`Enter` 确认。确认后每个文件成为一个本地任务；按 `A`、`S` 可启动全部任务。批量结果写入转写稿输出目录下的同名文件夹，并保留原目录层级；同一目录下不同扩展名的同名媒体会使用独立子目录，避免覆盖。按 `c` 配置 Bcut/Buzz 与 AI；失败后选中任务按 `r` 继续。

下载、FFmpeg、语音识别或 AI 中途失败时，已经完整验证的中间文件会保存在输出目录旁的隐藏 `.muxsift-work` 目录。修复错误后，在 CLI 原命令末尾添加 `--resume`，或在 TUI 选中失败/已取消任务并按 `r`。例如，ASR 失败会复用已封装的 M4A，AI 失败会复用已有 SRT；成功完成后自动清理检查点。Bcut 与 Buzz 都使用 M4A，因此失败后允许切换后端再继续；BV、分 P 或媒体流不匹配的检查点会被拒绝。

## 配置

运行 `./release/muxsift config show` 查看实际配置位置；运行 `config init` 创建默认文件。不要在 JSON 字符串中直接粘贴多行文本，换行必须写成 `\n`。

## AI 转写整理

Linux 可使用 Buzz 和已下载的 faster-whisper 模型在本地转写，不上传音频：

```bash
./muxsift BV1h3hb6jELV --transcribe --asr buzz \
  --buzz-project /home/you/Projects/buzz --buzz-uv /home/you/.local/bin/uv \
  --buzz-model large-v3-turbo --buzz-device cuda --buzz-compute-type float16
```

也可在 JSON 配置的 `asr` 段设置默认后端和 Buzz 路径。CLI 和 TUI 会实时显示 Buzz 识别百分比、已处理/总媒体时长及 ETA。Buzz 本地任务默认串行，避免同时加载多个大模型耗尽显存。

仅在 SRT 转写后启用。TUI 按 `c` 打开统一设置窗口，用 `Tab` 或左右方向键切换“通用／转写／AI／输出目录”，上下方向键选择字段，`Enter` 编辑或切换。转写页可设置 Bcut/Buzz 后端、Buzz 项目目录、`uv`、模型等；`C` 可直接进入 AI 页，按 `t` 测试连接。系统提示词支持多行粘贴和普通 `Enter` 换行，用 `F2` 保存；`Esc` 放弃未保存的编辑并关闭设置。保存的改动会写回配置文件。API Key 仍只放在环境变量，不写入配置：

```bash
export OPENAI_API_KEY='你的密钥'
./release/muxsift ai-test
./release/muxsift BV1xxxx --transcribe --transcript-format srt
```

会保留原始 `.srt` 并写入同名 `.md`。SRT 会发送至你配置的 AI Provider；请确认内容与服务的隐私政策相符。AI 失败时 SRT 不会删除。

如果同名 SRT 已存在，再次启动该转写任务时可跳过下载和语音识别，直接把已有 SRT 交给 AI。CLI 会显示 `[y/N]` 确认；TUI 会弹出“使用现有 SRT / 取消任务”窗口。默认选择取消。仅在 AI 已启用且同名 `.md` 尚不存在时提供此选项，已有文件不会被覆盖。

## 排错

`aria2c was not found`：发行包应带 `bin/aria2c`，或在配置中指定路径。`412` 或 Bcut 免费额度错误：这是服务端拒绝，检查登录、额度和网络。已有同名输出时程序不会覆盖，请改名或移动旧文件。

## 运行工具与配置目录

发行包应自带 `bin/aria2c` 和 `bin/ffmpeg`；可用
`--aria2 /路径/aria2c`、`--ffmpeg /路径/ffmpeg`
指定其他程序。若随包工具缺失，请下载完整发行包；从源码运行时可执行
`sudo apt install aria2 ffmpeg`，再显式指定路径。`config show` 可查看 XDG
配置目录中的 JSON 配置路径，登录凭据也保存在该目录，不包含在发行包中。

维护者可依次运行 `./scripts/fetch-linux-tools.sh` 和
`./scripts/package-linux.sh`，自动下载经过 SHA-256 校验的便携工具并生成发行包。
