"""Render static, editable UI proposals; no application state or network access.

Run from the repository root: python3 docs/design/tooling/render_direction.py
SVGs are design proposals, not screenshots or functional acceptance evidence.
"""
from pathlib import Path
from html import escape
import json

OUT = Path(".artifacts/ui-direction-20261020")
OUT.mkdir(parents=True, exist_ok=True)
PALETTES = {
    "light": dict(bg="#FFFFFF", rail="#F7F7F5", raised="#F4F4F2",
                  text="#20211F", secondary="#62655F", line="#E4E5E0",
                  outline="#858880", accent="#476650", primary="#242622",
                  onprimary="#FFFFFF", warning="#8A5A22"),
    "dark": dict(bg="#1C1D1B", rail="#171815", raised="#272925",
                 text="#ECEDE8", secondary="#B1B4AB", line="#3D4038",
                 outline="#7C8176", accent="#A9B99D", primary="#ECEDE8",
                 onprimary="#20211F", warning="#D9B980"),
}
ICONS = {
    "chevron": "m9 5 7 7-7 7",
    "down": "m6 9 6 6 6-6",
    "plus": "M12 5v14M5 12h14",
    "search": "M10.5 17a6.5 6.5 0 1 0 0-13 6.5 6.5 0 0 0 0 13m5-2 5 5",
    "panel": "M4 4h16v16H4zM9 4v16",
    "chat": "M5 4h14a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2H9l-5 3V6a2 2 0 0 1 1-2z",
    "folder": "M3 6h6l2 2h10v12H3z",
    "clock": "M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18m0 4v5l3 2",
    "model": "M6 6h12v12H6zM9 2v4m6-4v4M9 18v4m6-4v4M2 9h4m-4 6h4m12-6h4m-4 6h4",
    "settings": "M4 7h16M4 17h16M8 4v6m8 4v6",
    "spark": "m12 3 2.4 6.6L21 12l-6.6 2.4L12 21l-2.4-6.6L3 12l6.6-2.4z",
    "arrow": "M12 19V5m-6 6 6-6 6 6",
    "file": "M5 3h9l5 5v13H5zM14 3v6h5M8 13h8m-8 4h6",
    "copy": "M9 8h11v13H9zM15 8V3H4v13h5",
    "check": "m5 12 4 4L19 6",
    "shield": "M12 3 4 6v6c0 5 8 9 8 9s8-4 8-9V6zM12 8v5m0 3v.1",
    "menu": "M4 8h16M4 16h11",
    "link": "m10 8 3-3a4 4 0 0 1 6 6l-3 3m-2 2-3 3a4 4 0 0 1-6-6l3-3m0 8 8-8",
    "back": "m14 5-7 7 7 7",
    "stop": "M7 7h10v10H7z",
}


class Screen:
    def __init__(self, width, height, theme="light"):
        self.w, self.h = width, height
        self.c = PALETTES[theme]
        self.parts = [f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}">',
                      '<style>text { font-family: -apple-system, BlinkMacSystemFont, "PingFang SC", "Noto Sans CJK SC", sans-serif; } </style>']
        self.rect(0, 0, width, height, self.c["bg"])

    def rect(self, x, y, w, h, fill="none", radius=0, stroke=None):
        self.parts.append(f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="{radius}" fill="{fill}"'
                          + (f' stroke="{stroke}" stroke-width="1"' if stroke else '') + '/>')

    def text(self, x, y, value, size=14, weight=400, color=None, anchor="start"):
        self.parts.append(f'<text x="{x}" y="{y}" font-size="{size}" font-weight="{weight}" fill="{color or self.c["text"]}" text-anchor="{anchor}">{escape(value)}</text>')

    def lines(self, x, y, values, size=16, leading=28, color=None, weight=400):
        for i, value in enumerate(values):
            self.text(x, y + i * leading, value, size, weight, color)

    def line(self, x, y, x2, y2, color=None):
        self.parts.append(f'<path d="M{x} {y}H{x2}" stroke="{color or self.c["line"]}"/>' if y == y2
                          else f'<path d="M{x} {y}L{x2} {y2}" stroke="{color or self.c["line"]}"/>')

    def icon(self, kind, x, y, size=18, color=None):
        self.parts.append(f'<g transform="translate({x} {y}) scale({size / 24})" fill="none" stroke="{color or self.c["secondary"]}" stroke-width="1.65" stroke-linecap="round" stroke-linejoin="round"><path d="{ICONS[kind]}"/></g>')

    def more(self, x, y, color=None):
        for dx in (0, 5, 10):
            self.parts.append(f'<circle cx="{x+dx}" cy="{y}" r="1.3" fill="{color or self.c["secondary"]}"/>')

    def pill(self, x, y, w, label, primary=False, height=36):
        self.rect(x, y, w, height, self.c["primary"] if primary else "none", 8,
                  None if primary else self.c["outline"])
        self.text(x+w/2, y+height/2+5, label, 13, 500,
                  self.c["onprimary"] if primary else self.c["text"], "middle")

    def avatar(self, x, y, letter="N", size=24):
        self.rect(x, y, size, size, self.c["raised"], 7)
        self.text(x+size/2, y+size*.70, letter, size*.55, 500, anchor="middle")

    def save(self, name):
        (OUT / f"{name}.svg").write_text("".join(self.parts) + "</svg>")


def sidebar(s, selected="chat"):
    c = s.c
    s.rect(0, 0, 248, 960, c["rail"])
    s.text(24, 39, "Nola", 22, 600)
    s.icon("panel", 208, 23, 18)
    s.icon("plus", 26, 84, 19, c["text"])
    s.text(56, 99, "新对话", 14, 500)
    s.text(218, 99, "⌘ N", 11, color=c["secondary"], anchor="end")
    s.icon("search", 26, 129, 18)
    s.text(56, 144, "搜索", 14, color=c["secondary"])
    s.text(24, 201, "助手", 12, 500, c["secondary"])
    s.icon("plus", 211, 187, 16)
    for y, letter, label in [(234, "N", "Nola"), (274, "研", "研究助手"), (314, "码", "代码助手")]:
        s.avatar(26, y-17, letter)
        s.text(62, y, label)
    s.text(24, 389, "对话", 12, 500, c["secondary"])
    if selected == "chat":
        s.rect(16, 407, 216, 40, "none", 8, c["accent"])
    s.text(28, 432, "本周产品进展", 14, 500 if selected == "chat" else 400)
    s.more(207, 428)
    s.text(28, 474, "代码库结构梳理", 14, color=c["secondary"])
    s.text(28, 516, "给旅行留一点空白", 14, color=c["secondary"])
    s.text(24, 693, "工作区", 12, 500, c["secondary"])
    for y, kind, label, key in [(730, "folder", "资料", "data"), (770, "clock", "自动化", "auto"),
                                (810, "model", "模型", "models"), (850, "spark", "技能与 MCP", "skills")]:
        if selected == key:
            s.rect(16, y-26, 216, 40, "none", 8, c["accent"])
        s.icon(kind, 27, y-15, 18)
        s.text(57, y, label, 14, 500 if selected == key else 400)
    s.icon("settings", 27, 911, 18)
    s.text(57, 926, "设置")


def header(s, title="本周产品进展"):
    s.text(284, 37, title, 15, 500)
    s.text(1175, 37, "Qwen3.8", 13, color=s.c["secondary"])
    s.icon("down", 1247, 23, 16)
    s.icon("panel", 1326, 23, 18)
    s.more(1385, 32)


def composer(s, y=786, draft=True):
    c = s.c
    s.rect(464, y, 752, 116, c["bg"], 18, c["outline"])
    s.text(486, y+34, "先从对话界面开始。" if draft else "补充说明…", 16,
           color=c["text"] if draft else c["secondary"])
    s.icon("plus", 485, y+75, 20)
    s.icon("shield", 528, y+76, 17)
    s.text(552, y+90, "按需确认", 12, color=c["secondary"])
    s.rect(1164, y+64, 36, 36, c["primary"] if draft else c["raised"], 18)
    s.icon("arrow", 1172, y+72, 20, c["onprimary"] if draft else c["secondary"])


def desktop(theme="light", approval=False):
    s = Screen(1440, 960, theme)
    c = s.c
    sidebar(s)
    header(s)
    s.text(840, 105, "今天", 12, color=c["secondary"], anchor="middle")
    s.rect(700, 132, 516, 80, c["raised"], 16)
    s.lines(720, 163, ["帮我整理这周的产品进展，", "标出接下来需要我决定的事项。"], 16, 27)
    s.avatar(464, 256)
    s.text(500, 274, "Nola", 14, 600)
    s.text(1216, 274, "待你确认" if approval else "已完成", 12, color=c["secondary"], anchor="end")
    if approval:
        s.lines(464, 319, ["我已经整理了已有的工作记录。还有一份个人资料中的项目回顾，",
                           "需要你确认后才能读取。"], 16, 28)
        s.rect(464, 384, 752, 282, c["bg"], 12, c["outline"])
        s.icon("shield", 486, 408, 21, c["warning"])
        s.text(519, 425, "读取个人资料", 18, 600)
        s.text(486, 465, "项目回顾.pdf", 16, 500)
        s.text(486, 493, "~/Documents/个人资料/项目回顾.pdf", 13, color=c["secondary"])
        s.text(486, 532, "用途", 13, color=c["secondary"])
        s.text(558, 532, "补充本周进展中的背景与待办事项", 14)
        s.text(486, 561, "范围", 13, color=c["secondary"])
        s.text(558, 561, "仅此文件 · 仅本次读取", 14)
        s.pill(904, 606, 82, "拒绝")
        s.pill(998, 606, 194, "仅允许本次读取", True)
        s.text(464, 707, "等待确认期间，不会读取这份文件。", 13, color=c["secondary"])
    else:
        s.lines(464, 319, ["这周的重点，是让日常使用更稳定。", "下一步，建议先把注意力放在视觉统一和完整体验上。"], 16, 28)
        s.text(464, 404, "已经推进", 18, 600)
        for y, title, detail in [
            (441, "交互更一致", "展开标题与留白都能点击，独立操作保持清楚。"),
            (479, "模型配置更明确", "默认能力与显式设置有了统一的规则。"),
        ]:
            s.text(466, y, "·", 18, color=c["secondary"])
            s.text(486, y, title, 15, 500)
            s.text(610, y, detail, 15, color=c["secondary"])
        s.text(464, 535, "接下来，先做好两件事", 18, 600)
        s.lines(464, 572, ["让对话回到阅读和表达本身，减少长期露出的配置与管理按钮。",
                           "再用真实的长文本、工具过程和审批，检验整页是否自然。"], 16, 28)
        s.icon("chevron", 462, 632, 16)
        s.text(487, 645, "查看整理过程", 13, color=c["secondary"])
        s.icon("file", 464, 686, 22)
        s.text(498, 697, "本周进展.md", 14, 500)
        s.text(498, 719, "整理结果 · Markdown", 12, color=c["secondary"])
        s.icon("chevron", 690, 687, 18)
        s.icon("copy", 464, 750, 16)
        s.more(502, 758)
    composer(s, draft=not approval)
    s.save(f"desktop-{'approval' if approval else 'chat'}-{theme}")


def mobile(theme="light", approval=False):
    s = Screen(390, 844, theme)
    c = s.c
    s.text(24, 29, "9:41", 14, 600)
    s.rect(339, 18, 24, 11, "none", 3, c["text"])
    s.rect(342, 21, 17, 5, c["text"], 1)
    for i in range(4):
        s.rect(316+i*4, 25-i*2, 2, 4+i*2, c["text"], 1)
    s.icon("menu", 20, 59, 22, c["text"])
    s.text(60, 69, "本周产品进展", 16, 600)
    s.text(60, 90, "Qwen3.8", 12, color=c["secondary"])
    s.icon("down", 123, 79, 14)
    s.more(350, 70)
    s.rect(64, 118, 306, 76, c["raised"], 16)
    s.lines(80, 147, ["帮我整理这周的产品进展，", "标出需要我决定的事项。"], 16, 26)
    s.avatar(20, 225)
    s.text(56, 243, "Nola", 14, 600)
    if approval:
        s.lines(20, 281, ["已有记录已整理好。还需要读取一份", "个人资料中的项目回顾，请你确认。"], 16, 27)
        s.rect(20, 336, 350, 300, c["bg"], 12, c["outline"])
        s.icon("shield", 36, 357, 20, c["warning"])
        s.text(66, 374, "读取个人资料", 17, 600)
        s.text(36, 414, "项目回顾.pdf", 16, 500)
        s.text(36, 442, "个人资料 / 项目回顾.pdf", 13, color=c["secondary"])
        s.lines(36, 481, ["补充本周进展中的背景与待办。", "仅此文件 · 仅本次读取"], 14, 26, c["secondary"])
        s.pill(36, 550, 318, "仅允许本次读取", True, 44)
        s.text(195, 623, "拒绝", 15, 500, anchor="middle")
    else:
        s.lines(20, 286, ["这周的重点，是让日常使用更稳定。", "下一步，可以把重心转向视觉统一", "和完整的使用体验。"], 17, 28)
        s.text(20, 406, "接下来，先做好两件事", 20, 600)
        s.text(20, 448, "让对话更专注", 16, 600)
        s.lines(20, 478, ["把配置和管理动作收进合适的入口，", "给正文与输入留出空间。"], 16, 27, c["secondary"])
        s.text(20, 552, "用真实内容检验设计", 16, 600)
        s.lines(20, 582, ["长文本、工具过程、文件和审批，", "都应该清楚、自然地呈现。"], 16, 27, c["secondary"])
        s.icon("chevron", 18, 644, 16)
        s.text(43, 657, "查看整理过程", 13, color=c["secondary"])
    s.rect(12, 695, 366, 110, c["bg"], 18, c["outline"])
    s.text(29, 728, "补充说明…" if approval else "先从对话界面开始。", 17,
           color=c["secondary"] if approval else c["text"])
    s.icon("plus", 28, 765, 22)
    s.icon("shield", 73, 767, 18)
    s.text(99, 781, "按需确认", 12, color=c["secondary"])
    s.rect(326, 752, 40, 40, c["raised"] if approval else c["primary"], 20)
    s.icon("arrow", 335, 761, 22, c["secondary"] if approval else c["onprimary"])
    s.rect(128, 829, 134, 5, c["text"], 3)
    s.save(f"mobile-{'approval' if approval else 'chat'}-{theme}")


def models(theme="light"):
    s = Screen(1440, 960, theme)
    c = s.c
    sidebar(s, "models")
    s.text(284, 37, "工作区", 14, color=c["secondary"])
    s.icon("chevron", 339, 22, 16)
    s.text(367, 37, "模型", 14, 500)
    s.text(304, 123, "模型", 28, 600)
    s.text(304, 158, "管理连接，为每次对话选择合适的模型。", 15, color=c["secondary"])
    s.pill(1240, 104, 136, "＋ 添加连接", True, 40)
    s.rect(304, 198, 1072, 44, c["bg"], 9, c["outline"])
    s.icon("search", 320, 211, 18)
    s.text(350, 226, "搜索模型或连接", 14, color=c["secondary"])
    for x, w, text, active in [(304, 94, "全部模型", True), (410, 66, "本地", False), (488, 66, "云端", False)]:
        if active:
            s.rect(x, 266, w, 34, "none", 8, c["accent"])
        s.text(x+w/2, 288, text, 13, 500 if active else 400, anchor="middle")
    s.text(1376, 288, "3 个模型", 12, color=c["secondary"], anchor="end")
    s.text(304, 345, "名称", 12, color=c["secondary"])
    s.text(702, 345, "连接", 12, color=c["secondary"])
    s.text(1118, 345, "状态", 12, color=c["secondary"])
    s.line(304, 362, 1376, 362)
    for y, name, sub, connection, status, default in [
        (402, "Qwen3.8", "文字 · 工具调用", "本机服务", "可用", True),
        (498, "工作模型", "文字 · 图像 · 工具调用", "OpenAI", "可用", False),
        (594, "实验模型", "文字 · 工具调用", "自定义连接", "连接失败", False),
    ]:
        s.icon("model", 307, y-15, 24)
        s.text(350, y, name, 16, 500)
        s.text(350, y+27, sub, 13, color=c["secondary"])
        if default:
            s.rect(458, y-21, 42, 24, "none", 6, c["outline"])
            s.text(479, y-5, "默认", 11, anchor="middle")
        s.text(702, y, connection, 14)
        s.text(702, y+27, {"本机服务": "127.0.0.1 · 本地", "OpenAI": "api.openai.com · 云端",
                          "自定义连接": "未连接 · 查看配置"}[connection], 12, color=c["secondary"])
        s.text(1118, y, status, 14, color=c["warning"] if status == "连接失败" else c["secondary"])
        if status == "连接失败":
            s.text(1118, y+27, "检查连接", 13, color=c["accent"])
        s.more(1346, y-4)
        s.line(304, y+55, 1376, y+55)
    s.text(304, 731, "默认与能力", 18, 600)
    s.lines(304, 766, ["新对话默认使用 Qwen3.8。已有对话保留自己的模型设置。",
                       "工具调用、图像输入等能力可在模型详情中调整。"], 14, 26, c["secondary"])
    s.icon("chevron", 302, 832, 16)
    s.text(328, 846, "高级模型设置", 13, color=c["secondary"])
    s.save(f"models-{theme}")


def model_details():
    s = Screen(1440, 960)
    c = s.c
    sidebar(s, "models")
    s.icon("back", 282, 23, 18)
    s.text(310, 38, "模型", 14, color=c["secondary"])
    s.text(440, 130, "Qwen3.8", 28, 600)
    s.text(440, 164, "模型设置", 15, color=c["secondary"])
    s.rect(1114, 107, 78, 28, "none", 7, c["outline"])
    s.text(1153, 126, "默认模型", 12, anchor="middle")
    s.text(440, 224, "连接", 18, 600)
    s.text(440, 262, "本机服务", 15, 500)
    s.text(440, 287, "127.0.0.1 · OpenAI 兼容接口", 13, color=c["secondary"])
    s.text(1192, 266, "管理连接", 13, color=c["accent"], anchor="end")
    s.line(440, 319, 1192, 319)
    s.text(440, 360, "模型能力", 18, 600)
    s.text(440, 389, "按模型的实际支持情况设置。开启能力不代表授予操作权限。", 13, color=c["secondary"])
    for y, label, desc, enabled in [
        (438, "工具调用", "让助手使用工具完成任务", True),
        (518, "图像输入", "在对话中理解图片内容", False),
        (598, "推理", "使用模型支持的推理能力", True),
    ]:
        s.text(440, y, label, 15, 500)
        s.text(440, y+26, desc, 13, color=c["secondary"])
        s.rect(1148, y-10, 44, 26, c["accent"] if enabled else c["raised"], 13,
               None if enabled else c["outline"])
        s.parts.append(f'<circle cx="{1179 if enabled else 1161}" cy="{y+3}" r="10" fill="{c["bg"]}"'
                       + (f' stroke="{c["outline"]}"' if not enabled else '') + '/>')
        s.line(440, y+49, 1192, y+49)
    s.icon("chevron", 438, 693, 17)
    s.text(464, 707, "生成参数", 14, 500)
    s.text(464, 734, "上下文长度、输出长度与采样设置", 13, color=c["secondary"])
    s.text(440, 837, "修改仅在保存后生效。", 13, color=c["secondary"])
    s.pill(971, 811, 85, "取消", False, 40)
    s.pill(1068, 811, 124, "保存修改", True, 40)
    s.save("model-details-light")


def contrast(a, b):
    def lum(h):
        rgb = [int(h[i:i+2], 16)/255 for i in (1, 3, 5)]
        v = [x/12.92 if x <= .04045 else ((x+.055)/1.055)**2.4 for x in rgb]
        return sum(x*w for x, w in zip(v, [.2126, .7152, .0722]))
    x, y = sorted([lum(a), lum(b)])
    return round((y+.05)/(x+.05), 2)


for theme in PALETTES:
    desktop(theme)
    mobile(theme)
    models(theme)
desktop("light", True)
mobile("light", True)
model_details()

report = {}
for theme, c in PALETTES.items():
    report[theme] = {
        "text/bg": contrast(c["text"], c["bg"]),
        "secondary/rail": contrast(c["secondary"], c["rail"]),
        "secondary/raised": contrast(c["secondary"], c["raised"]),
        "accent/rail": contrast(c["accent"], c["rail"]),
        "outline/bg": contrast(c["outline"], c["bg"]),
        "onprimary/primary": contrast(c["onprimary"], c["primary"]),
        "warning/bg": contrast(c["warning"], c["bg"]),
    }
(OUT / "contrast.json").write_text(json.dumps(report, indent=2))
print(json.dumps(report, indent=2))
