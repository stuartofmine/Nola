# 跨平台设计 Token（Warm Ink & Warm Paper）

`tokens.json` 是 Nola 跨端表面、文字层级、状态语义色、字号阶梯、圆角与间距的唯一机器可读真源。

## 目录职责

| 文件 | 说明 |
| --- | --- |
| [tokens.json](tokens.json) | 浅色（Warm Paper）与深色（Warm Ink）核心表面、正文/次级文字、边界、控制描边、语义状态色，以及字号、圆角、间距和动效基准 |

## 平台对齐入口

- 视觉总规范：[docs/design/shared/DESIGN_SYSTEM.md](../../docs/design/shared/DESIGN_SYSTEM.md)
- 原型样式：[docs/design/prototype/style.css](../../docs/design/prototype/style.css)、[docs/design/prototype/surface-material.css](../../docs/design/prototype/surface-material.css)
- Web 前端：[apps/web/frontend/src/theme.css](../../apps/web/frontend/src/theme.css)
- Windows 桌面壳：[apps/windows/src/main.ts](../../apps/windows/src/main.ts)、[apps/windows/src/startupPage.ts](../../apps/windows/src/startupPage.ts)
- Android Compose：[apps/android/app/src/main/java/com/nola/ui/theme/Theme.kt](../../apps/android/app/src/main/java/com/nola/ui/theme/Theme.kt)
- iOS / iPadOS：[apps/ios/UI/DesignSystem.swift](../../apps/ios/UI/DesignSystem.swift)
- macOS：[apps/macos/UI/MacPageChrome.swift](../../apps/macos/UI/MacPageChrome.swift)

## 静态门禁

运行以下命令校验 WCAG AA 对比度（正文与次级文字在 `canvas` / `surface` 上 `>= 4.5:1`）及各端核心表面一致性：

```sh
python3 -B scripts/check_design_tokens.py
```
