> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# 日志系统文档

## 概述

后端应用已配置了完善的日志系统，支持：
- ✅ 控制台彩色输出
- ✅ 文件日志（带轮转）
- ✅ 分级日志（独立的错误日志）
- ✅ 详细的日志格式（包含文件名、行号、函数名）
- ✅ 第三方库日志过滤
- ✅ 灵活的环境变量配置

## 日志文件位置

日志文件存储在 `backend/logs/` 目录下：

- **app.log** - 所有日志（DEBUG, INFO, WARNING, ERROR, CRITICAL）
- **error.log** - 仅错误日志（ERROR, CRITICAL）

## 日志格式

### 文件日志格式（详细）
```
2026-01-29 00:13:31 - nola.gateway - INFO - main.py:104 - on_startup() - Gateway startup: forwarding to https://api.openai.com/v1
```

包含：
- 时间戳
- logger 名称
- 日志级别
- 文件名和行号
- 函数名
- 日志消息

### 控制台日志格式（简洁，带颜色）
```
2026-01-29 00:13:31 - nola.gateway - INFO - Gateway startup
```

## 使用方法

### 在现有模块中使用

所有现有模块已自动更新，使用新的日志系统：

```python
from .logging_config import get_logger

logger = get_logger("nola.gateway.your_module")

# 使用 logger
logger.debug("调试信息")
logger.info("一般信息")
logger.warning("警告信息")
logger.error("错误信息")
logger.critical("严重错误")

# 记录异常（包含完整堆栈跟踪）
try:
    # 你的代码
    pass
except Exception as e:
    logger.exception("发生异常")  # 自动记录堆栈跟踪
```

### 在新模块中使用

```python
from .logging_config import get_logger

# 建议使用层次化的 logger 名称
logger = get_logger("nola.gateway.your_new_module")

logger.info("你的日志消息")
```

## 环境变量配置

在 `nola_backend_env` JSON 中配置：

### 基本配置

```bash
# 日志级别：DEBUG, INFO, WARNING, ERROR, CRITICAL
GATEWAY_LOG_LEVEL=DEBUG

# 或使用通用的 LOG_LEVEL
LOG_LEVEL=INFO
```

### 高级配置

```bash
# 自定义日志目录（默认：backend/logs）
LOG_DIR=/path/to/custom/logs

# 是否启用文件日志（默认：true）
ENABLE_FILE_LOGGING=true

# 是否启用控制台日志（默认：true）
ENABLE_CONSOLE_LOGGING=true

# 单个日志文件最大大小，单位 MB（默认：10）
LOG_MAX_FILE_SIZE=10

# 保留的备份文件数量（默认：5）
LOG_BACKUP_COUNT=5

# 是否使用按日期轮转（默认：false，即按大小轮转）
LOG_USE_DAILY_ROTATION=false
```

## 日志轮转

### 按大小轮转（默认）
- 当 `app.log` 达到 10MB 时，自动重命名为 `app.log.1`
- 保留最近 5 个备份文件
- 文件命名：`app.log`, `app.log.1`, `app.log.2`, ... `app.log.5`

### 按日期轮转
设置 `LOG_USE_DAILY_ROTATION=true` 后：
- 每天午夜自动创建新的日志文件
- 文件命名：`app.log.2026-01-29`, `app.log.2026-01-30`, ...
- 保留最近 5 天的日志

## 测试日志系统

运行测试脚本验证日志配置：

```bash
cd backend
python tests/test_logging.py
```

检查生成的日志文件：
```bash
# 查看所有日志
tail -f logs/app.log

# 仅查看错误日志
tail -f logs/error.log

# 查看实时日志（跟随模式）
tail -f logs/app.log logs/error.log
```

## 日志级别说明

| 级别 | 使用场景 | 示例 |
|------|---------|------|
| DEBUG | 详细的诊断信息，用于开发和调试 | 函数参数、中间变量、详细流程 |
| INFO | 常规操作信息 | 服务启动、配置加载、正常业务流程 |
| WARNING | 警告信息，不影响正常运行但需要注意 | 配置缺失使用默认值、即将过期的证书 |
| ERROR | 错误信息，影响特定功能 | API 调用失败、数据库错误 |
| CRITICAL | 严重错误，可能导致应用崩溃 | 无法连接数据库、关键服务不可用 |

## 最佳实践

### 1. 使用合适的日志级别
```python
# ❌ 不好
logger.info(f"详细的调试信息：{large_data_structure}")

# ✅ 好
logger.debug(f"详细的调试信息：{large_data_structure}")
```

### 2. 使用参数化日志消息（而不是 f-string）
```python
# ❌ 不好（即使日志级别不够也会执行字符串格式化）
logger.debug(f"Processing user {user_id} with data {expensive_operation()}")

# ✅ 好（仅在日志级别足够时才格式化）
logger.debug("Processing user %s with data %s", user_id, data)
```

### 3. 记录异常时使用 exception()
```python
# ❌ 不好
try:
    risky_operation()
except Exception as e:
    logger.error(f"Error: {e}")  # 丢失了堆栈跟踪

# ✅ 好
try:
    risky_operation()
except Exception as e:
    logger.exception("Error occurred")  # 自动包含完整堆栈跟踪
```

### 4. 避免记录敏感信息
```python
# ❌ 不好
logger.info(f"User login: {username} with password {password}")

# ✅ 好
logger.info("User login: %s", username)
logger.debug("Login attempt for user %s", username)  # 密码永远不记录
```

### 5. 使用层次化的 logger 名称
```python
# 主模块
logger = get_logger("nola.gateway")

# 子模块
push_logger = get_logger("nola.gateway.push")
search_logger = get_logger("nola.gateway.search")
```

这样可以：
- 方便地过滤和搜索日志
- 更清晰地了解日志来源
- 可以为不同模块设置不同的日志级别

## 生产环境建议

```bash
export nola_backend_env='{
  "GATEWAY_LOG_LEVEL": "INFO",
  "ENABLE_CONSOLE_LOGGING": false,
  "ENABLE_FILE_LOGGING": true,
  "LOG_MAX_FILE_SIZE": 50,
  "LOG_BACKUP_COUNT": 10,
  "LOG_USE_DAILY_ROTATION": true
}'
```

## 故障排查

### 日志文件未生成
1. 检查 `ENABLE_FILE_LOGGING` 是否为 `true`
2. 检查 `logs/` 目录权限
3. 查看控制台错误信息

### 日志文件太大
1. 调整 `LOG_MAX_FILE_SIZE` 为更小的值
2. 减少 `LOG_BACKUP_COUNT` 以减少磁盘使用
3. 降低日志级别（从 DEBUG 改为 INFO）

### 控制台没有颜色
- 颜色仅在 TTY 环境下显示
- 管道输出 (`|`) 或重定向 (`>`) 时自动禁用颜色

### 第三方库日志过多
编辑 `logging_config.py`，添加：
```python
logging.getLogger('noisy_library').setLevel(logging.WARNING)
```

## 日志系统

### 旧代码
```python
import logging
logger = logging.getLogger("nola.gateway")
```

### 新代码
```python
from .logging_config import get_logger
logger = get_logger("nola.gateway")
```

日志调用方式（`logger.info()`, `logger.error()` 等）保持不变。

## 监控和分析

### 实时监控
```bash
# 监控所有日志
tail -f logs/app.log

# 监控错误日志
tail -f logs/error.log

# 过滤特定模块
tail -f logs/app.log | grep "nola.gateway.push"

# 仅显示错误
tail -f logs/app.log | grep "ERROR\|CRITICAL"
```

### 日志分析
```bash
# 统计错误数量
grep -c "ERROR" logs/app.log

# 查找特定错误
grep "Failed to" logs/error.log

# 按日期查看（如果使用日期轮转）
cat logs/app.log.2026-01-29 | grep "ERROR"
```

## 性能考虑

- 日志 I/O 是异步的，不会阻塞主线程
- 使用参数化日志可以避免不必要的字符串格式化
- 在生产环境使用 INFO 级别而不是 DEBUG
- 日志轮转自动处理，不需要手动清理
