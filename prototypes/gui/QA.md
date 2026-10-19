# GUI prototype verification

2026-10-19，使用 Playwright Chromium 在本机验证两个候选版。此记录只覆盖浏览器原型，不能作为真实媒体后端的验收结果。

| 检查 | Liquid Glass | Neo-brutalism |
| --- | --- | --- |
| 直接打开本地 HTML，资源完整加载 | 通过 | 通过 |
| 非法 URL 提示，示例链接创建任务 | 通过 | 通过 |
| 模拟进度、暂停后进度保持、继续至完成 | 通过 | 通过 |
| 本地多文件加入队列 | 通过 | 通过 |
| 音频输入关键帧模式时提示不支持 | 通过 | 通过 |
| Markdown、SRT、关键帧 JSON 导出 | 通过 | 通过 |
| 关键帧示意图、章节时间线、笔记切换 | 通过 | 通过 |
| 搜索、已完成笔记视图、重置队列 | 通过 | 通过 |
| Buzz/Bcut 偏好保存，刷新后恢复 | 通过 | 通过 |
| 设置窗口保存关闭与 Esc 关闭 | 通过 | 通过 |
| 320 / 390 / 768 / 1024 / 1440px 无横向溢出 | 通过 | 通过 |
| JavaScript 页面错误 | 0 | 0 |
| 原型触发外部网络请求 | 0 | 0 |

实际浏览器截图位于 `previews/`，包含桌面和 390px 手机布局；已人工检查布局。文件夹选择与系统剪贴板权限未做跨浏览器实测。尚未验证 Safari、Firefox 或屏幕阅读器。

## 复现

原型本身不需要 npm。仅重跑自动化验证时，在临时目录安装 Playwright：

```bash
npm install --prefix /tmp/muxsift-gui-qa playwright
node /tmp/muxsift-gui-qa/node_modules/playwright/cli.js install chromium
NODE_PATH=/tmp/muxsift-gui-qa/node_modules node prototypes/gui/qa.cjs
```

脚本默认直接加载本地 HTML，更新实际截图并执行交互断言。也可设置 `GUI_BASE_URL=http://127.0.0.1:8765/`，复验本地 HTTP 模式。
