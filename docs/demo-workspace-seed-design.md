# 开源版首次启动演示工作区

## Root Cause

开源版工作区不存在 `catalog.json` 时，`Store::load` 返回只有默认分组的空 Catalog。公开版文档也将 demo 项目标注为不预装；当前运行目录确实没有 catalog，因此不是删除后可恢复的数据。

## Architecture Design

开源版使用数据驱动的 bundled Profile 初始化首次工作区。加载时仅当 catalog 文件不存在才合并示例数据并通过 Store 原子保存；已有 catalog（包括用户清空后的 catalog）保持不变。私人版使用现有空白/个人工作区行为。

## Implementation

示例 Profile 在 `assets/demo-workspace/profile.json` 声明四个虚拟项目及八个演示/测试入口。首次启动时将每个项目复制到本地应用数据目录，并生成统一的 Python 标准库 HTTP 模拟脚本。脚本只监听回环地址 `127.0.0.1`，入口需要用户明确点击后才会启动；不连接数据库、外网或部署环境。若同名示例脚本已存在，不覆盖用户文件。

## Verification

开源版空 workspace 首次启动应出现 demo project 分组、四个项目和八个入口；私人版行为不变；已有 catalog 不变。两个发行版均需 Release 构建；运行时确认公开 demo 窗口退出后不残留。
