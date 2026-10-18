# 关键帧融合原型

本页记录 `exp/frames-fusion` 分支的离线视觉实验。`frames-proto` 与现有 `frames` 独立；它不会修改视频或 v0.1.0 发布版。先执行 `cargo build -p muxsift-cli --locked`，再运行：

```bash
target/debug/muxsift frames-proto ./lecture.mp4 -o ./lecture-proto
target/debug/muxsift frames-proto ./ui.mp4 -o ./ui-proto \
  --adaptive-threshold 2.5 --min-content 1 --min-gap 3 --spatial-distance 0.05
```

输出目录必须不存在。结果包含 `frames/`、`frames.md` 和 `timeline.json`。后者的 `frames` 是保留帧；`decisions` 包含每个采样帧的 `reason`、综合 `score`、FFmpeg `scdet` 原始分数、滚动均值、相对比值，以及实际计算过的 pHash、直方图、空间像素变化量。`near_duplicate` 还指出匹配的既有帧时间。低变化或最小间隔挡下的帧不再解码计算视觉签名，因此相关字段为空。

## 检测规则

FFmpeg 默认每秒采样一帧，缩至最多 960 像素宽，用 `scdet=t=0` 记录所有采样帧的内容分数。候选满足 `scdet >= 5`，或同时满足 `scdet >= 0.75` 与 `scdet / max(最近 12 帧均分, 0.35) >= 2`。首帧必留；其余候选须距上次保留帧至少 2 秒。对最近 8 张已保留帧比较 64 位 DCT pHash、24 格 RGB 直方图和 160×90 灰度像素：仅当三者距离分别不超过 `4`、`0.025`、`0.02`，才判为近重复。空间校验避免白底课件或 UI 因 pHash 相同而被误删。所有阈值都可通过 `frames-proto --help` 查看与调整。这里的 `scdet` 分数范围是 0–100，与旧 `frames` 的 `scene` 阈值 0–1 **不可直接换算**。

## 三段样本对照

2026-10-19，在本机 FFmpeg 和相同原视频上分别运行旧 `frames` 默认参数与原型默认参数。旧版所有帧在原型中均有相距不超过 1 秒的对应帧。下表的漏检、误报为截图抽查结论，**不是带完整人工标注的精确率统计**。

| 视频 | 旧版 | 原型 | 抽查的漏检与误报 |
| --- | ---: | ---: | --- |
| 影视剪辑，8:13 | 85 | 94 | 原型多 9 张；约 68、153、176、280、450 秒补到机位或场景变化；3、8、277 秒属于同镜头动作冗余。仍会漏掉采样间隔内的短镜头。 |
| UI 操作，4:56 | 7 | 29 | 旧版漏掉大量操作状态，包括约 28、54、78、235、248 秒；原型找回了 `StretchText` 页面。2 秒和末尾 285／293 秒仍有近重复，滚动过程也会增加候选。 |
| 论文讲解，18:08 | 12 | 17 | 新增约 691、718、742、773、785 秒的不同图表／表格；保留了旧版的 306、796、1072 秒页切换。抽查新增 5 张未见明显重复。 |

实验输出在 `/tmp/muxsift-frames-comparison-20261019/`；各目录的 `timeline.json` 可核对完整分数。`ui-tuned` 用下表 UI 参数输出 23 帧，仍覆盖旧版 7 帧。

## 起步参数

| 类型 | 建议参数 | 取舍 |
| --- | --- | --- |
| 影视剪辑 | 默认：`--interval 1 --content-threshold 5 --adaptive-threshold 2 --min-gap 2` | 覆盖较多剪切；同镜头动作可能多取。 |
| UI 操作 | `--adaptive-threshold 2.5 --min-content 1 --min-gap 3 --spatial-distance 0.05` | 本样本从 29 减至 23 张，保留旧版页面；快速连贯操作可能漏掉中间态。 |
| 论文讲解 | 默认；如担心短暂公式，可试 `--interval 0.5` | 本样本找回 5 个图表／表格；更密采样需再人工复核。 |

## 尚未实现

首版只做视觉检测，没有 OCR、字幕触发、逐步动画完成态判断或图像区域屏蔽。`muxsift-core` 的可选 `ocr` feature 当前只公开 `OcrDiff` 接口，**不会启用 OCR**；后续 Tesseract 适配器可使用 `sudo apt install tesseract-ocr tesseract-ocr-chi-sim tesseract-ocr-eng`，无需 cv2 或 NumPy。字幕驱动需要 SRT 时间段或 Whisper 词级时间戳，不能从无字幕视频凭空判断“新概念”。
