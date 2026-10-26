# Console 停止并关闭

## Root Cause

平铺卡片与标签页 Console 头部只有停止按钮。关闭标签的入口藏在标签右键菜单中，平铺卡片没有可见的关闭入口。

## Architecture Design

UI 使用统一的关闭请求区分平铺窗口和标签目标，并在处理请求时重新读取实际运行状态；仅活动实例要求确认，关闭操作复用 Runtime.close_tab，不复制进程控制流程。Runtime 标记 close_requested，并复用现有停止控制器；进程确认退出后关闭对应视图，停止失败则保留视图并展示真实错误。

## Implementation

在停止按钮右侧显示 X。平铺模式确认关闭窗口，标签模式确认关闭当前标签。活动服务确认后先走现有优雅停止与超时强制停止；已停止、已完成或失败的记录直接关闭 Console，不显示确认框；若确认框打开期间服务已经退出，也直接关闭。关闭请求处理中禁用 X，避免重复请求。

## Verification

2026-10-26：`cargo check`、开源版 `cargo check --no-default-features`、两版原生 macOS Release 打包通过。现有 `closing_tabs_requires_consent_and_history_can_reopen` 回归通过，覆盖活动服务需要明确停止授权、退出后关闭、历史重开和结束后直接关闭。

隔离原生 GUI 演示验证：自然完成的进程在平铺模式与标签模式点击 X 均直接关闭，不出现确认框。私人版已重启并恢复“官网本地”服务；原外部 Java 进程 PID 9067 保持运行。开源版与临时 QA 实例保持退出。
