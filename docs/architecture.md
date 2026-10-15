# 架构

DevStart 是一个 Rust 原生桌面应用。GUI 与本地 API 都操作同一个 Service，界面只负责展示与交互；文件、配置、进程和版本由后端校验和持久化。

| 模块 | 职责 |
|---|---|
| app_identity / store / model | 版本数据目录、原子配置、稳定实体 ID |
| discovery / profile / project_manifest | 只读扫描、显式启动清单、参数定义 |
| service / runtime / launch / lifecycle | 用例协调、异步预检查、进程组启停与退出确认 |
| external / runtime_ports / configured_ports | 外部实例身份、实际端口与配置端口来源 |
| console_buffer / output / console_search | 有界输出、流式 UTF-8/ANSI 处理、日志检索 |
| editor / editor_service / java_edit | 草稿、并发校验、源文件保存/回退、受控单文件 Java 编译 |
| api / api_config / api_transport / api_client | 本地 JSON-RPC、CLI、MCP 与协议契约 |
| ui* / appearance / preferences / presentation / windows | 原生界面、设置、展示 DTO 与多窗口 |

## 启动与退出

读取配置 → 核实同项目旧进程/端口 → 为编辑过的 Java 文件准备受控编译 → 请求优雅关闭 → 必要时强制关闭 → 复核并启动 → 并行读取输出。未知归属的占用进程会报错，不按端口盲目杀进程。Java 编译失败在停止原实例前返回。

生命周期检查在后台运行，GUI 继续响应。stdin 为关闭状态，因此 Console 是输出查看器，不是完整交互终端。退出码与停止状态直接来自实际进程。

## 数据边界

首次目录中只有自动创建的空默认分组，没有示例项目或个人配置。项目源文件仍在原项目路径。应用目录保存 catalog、日志、编辑历史和受权限保护的版本快照。每启动项最多十个源文件版本，关闭编辑历史最多十条，打开文件独立保留。

Console 使用有界缓存和虚拟行渲染；源文件有大小与打开数量上限；API 通过私有 Unix socket 和有界队列通信。实际运行时长使用单调计时，日期使用单独的墙上时钟。

## 标签、批量操作与展示元数据

进程运行 ID 标识一次执行，Console ID 标识稳定标签。重启生成新的运行 ID 并沿用 Console ID，旧回调不能覆盖新实例。关闭历史和打开标签分别管理，活跃进程与已打开标签不会因历史限额被裁掉。

批量操作由后端生成带配置修订信息的计划；前端只展示目标并收集确认，应用计划时再次核验修订和状态，配置变化时拒绝执行旧计划。展示名、简介是独立元数据，更新不触及命令参数、源文件及进程。状态列由后端根据已核实的运行关联生成。
