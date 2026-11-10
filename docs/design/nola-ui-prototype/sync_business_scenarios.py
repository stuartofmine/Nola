#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Validate the explicit scenario graph and regenerate its human-readable inventory."""
import argparse
import csv
import io
import json
from pathlib import Path
import re

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]


def generated():
    catalog = json.loads((HERE / 'BUSINESS-SCENARIOS.json').read_text())
    assert catalog['schema_version'] == 1
    scenarios = catalog['scenarios']
    ids = [scene['id'] for scene in scenarios]
    assert len(ids) == len(set(ids)), 'Duplicate business scenario'
    groups = {group['id']: group['title'] for group in catalog['groups']}
    rows = []
    for scene in scenarios:
        assert scene['group'] in groups
        assert scene['platforms'] and set(scene['platforms']) <= set(catalog['platforms'])
        states = {state['id']: state for state in scene['states']}
        assert len(states) == len(scene['states']) and '0' in states
        reached, pending = set(), ['0']
        while pending:
            key = pending.pop()
            if key in reached:
                continue
            assert key in states, f'Broken transition in {scene["id"]}: {key}'
            reached.add(key)
            state = states[key]
            assert state['title'] and state['body'] and state['actions']
            pending.extend(action['to'] for action in state['actions'])
        assert reached == set(states), f'Unreachable state in {scene["id"]}'
        for source in scene['sources']:
            path = ROOT / source
            assert path.resolve().is_relative_to(ROOT) and path.is_file(), source
        rows.append([scene['id'], groups[scene['group']], scene['title'], scene['entry'],
                     ' / '.join(catalog['platforms'][key] for key in scene['platforms']),
                     ' → '.join(state['title'] for state in scene['states']),
                     '; '.join(scene['sources']), scene['contract'], scene['boundary'],
                     f'scenarios.html?scene={scene["id"]}&platform={scene["platforms"][0]}'])
    route_source = (ROOT / 'web/frontend/src/lib/pageRoutes.ts').read_text()
    routes = set(re.findall(r"'(/[^']+)':\s*\(\)", route_source))
    assert routes == set(catalog['route_coverage']), 'Production routes need explicit scenario coverage'
    for mapped in catalog['route_coverage'].values():
        assert mapped and set(mapped) <= set(ids)
    buffer = io.StringIO(newline='')
    writer = csv.writer(buffer, lineterminator='\n')
    writer.writerow(['场景ID', '业务分组', '业务场景', '入口', '设计适用平台（非生产支持声明）', '可检查状态', '生产源码依据', '交互合同', '验证边界', '原型入口'])
    writer.writerows(rows)
    document = '''# 全业务场景与状态目录

更新：2026-11-10。主入口：[可操作场景原型](../scenarios.html)；原有完整交互原型：[index.html](../index.html)。

## 范围与使用方法

本清单以当前仓库用户可见业务为范围，共 **%d 个场景、%d 个状态、6 个平台视角**。每项有独立状态图、原型入口、适用平台与生产源码依据；Web 当前 **%d 个路由**全部关联。它覆盖可枚举的业务主线、异常和恢复，不声称穷尽任意数据、语言、系统版本和网络组合。

选择业务与平台，直接操作预览中的按钮；右侧可跳到任意状态。每个状态有独立 URL，浏览器前进／后退可复现。原型目录和状态面板属于评审工具，不是产品导航。字段只保存为当前页面内、当前平台及场景的草稿；取消保留已保存基线，切换场景不覆盖其他草稿，刷新重置。破坏性样例先打开可取消的确认框。没有模型、账户、Host、文件、下载或系统权限请求，不能输入真实凭据。

“适用平台”表示这项设计应在哪里提供，**不是已实现或已测试的能力声明**。不适用时明确显示范围，不生成假系统入口。macOS/iOS/Android 的控件只是浏览器布局示意；Windows 表示共享 Web Host 客户端方向；CLI 为文本布局草图，下方按钮仅用于评审，不是新 CLI 命令。Web 更新安装、手机永久后台调度及跨 Host 接管不作为已支持能力。

## 共同行为

- 桌面检查器原地独立展开，没有二级返回栈；知识、Wiki、历史召回不混成一种对象。
- 手机输入紧凑，保留系统返回与显式按钮；五档模型只显示一组实际推理档位。
- 正文以平台字体为默认，主题决定边框和焦点；回复复制无常驻填充，悬停与键盘焦点均可见。
- 新增内容短暂浮现，历史不重播；尊重减少动态效果，不靠动画延迟模型完成。原型不声称验证原生侧滑或语音识别。
- 授权显示具体操作、位置和范围。缺少安全审阅文本时禁用允许，仍可拒绝；配对、Skill/MCP 启用与工具授权分别处理。
- 客户端离开不取消 Host 任务；恢复读取原 Run。停止等待权威确认；未知写入结果先核对，不能自动重放。
- 上下文占用、压缩效率、缓存命中率分开。未上报用量不得显示成 0%%；一次执行结束不等于 Goal 完成。
- Nola 自有版本 1 基线不变；未知格式拒绝，保护原始数据，不自动删除或重建。外部标准协议沿用自身版本。

## 场景覆盖

| ID | 业务 / 场景 | 入口 | 状态数 | 适用设计平台 |
| --- | --- | --- | ---: | --- |
''' % (len(scenarios), sum(len(scene['states']) for scene in scenarios), len(routes))
    for row, scene in zip(rows, scenarios):
        document += f'| [{row[0]}](../{row[-1]}) | {row[1]} / {row[2]} | {row[3]} | {len(scene["states"])} | {row[4]} |\n'
    document += '\n## 路由关联\n\n| 生产 Web 路由 | 对应业务场景 |\n| --- | --- |\n'
    for route, mapped in catalog['route_coverage'].items():
        document += f'| `{route}` | '+', '.join(f'[{key}](../scenarios.html?scene={key}&platform=web)' for key in mapped)+' |\n'
    document += '''
## 维护与验收

`BUSINESS-SCENARIOS.json` 为当前目录的场景清单来源；`sync_business_scenarios.py` 校验状态图、源码路径与 Web 路由并生成本文和 CSV。新增业务需同时增加清单和原型状态；新增生产路由但没有映射时检查失败。`--check` 只检查同步，不修改文件。

```sh
python3 docs/design/nola-ui-prototype/sync_business_scenarios.py --check
python3 -m http.server 8071 --bind 127.0.0.1 --directory docs/design/nola-ui-prototype
PROTOTYPE_URL=http://127.0.0.1:8071 node docs/design/nola-ui-prototype/verify_business_scenarios.cjs
PROTOTYPE_URL=http://127.0.0.1:8071 node docs/design/nola-ui-prototype/verify_business_layout.cjs
```

浏览器验收范围与实际计数见 [VALIDATION.md](VALIDATION.md)。原型通过不代表生产链路通过；真实 Host 长任务的独立结果见 [宿主机流程](../../../../Doc/UI和交互设计/HOST_CONNECTION_WORKFLOW.md)。外部旧 Site 未随本地原型自动发布。
'''
    return {HERE / 'FEATURE-COVERAGE.csv': '\ufeff' + buffer.getvalue(),
            HERE / 'docs/BUSINESS-SCENARIOS.md': document}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    for path, content in generated().items():
        if args.check:
            assert path.read_bytes() == content.encode(), f'Out of sync: {path.name}'
        else:
            path.write_bytes(content.encode())
    print('Scenario graph, source paths, route coverage and generated inventory passed')


if __name__ == '__main__':
    main()
