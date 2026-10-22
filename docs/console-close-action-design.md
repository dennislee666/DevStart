# Console 停止并关闭

## Root Cause

平铺卡片与标签页 Console 头部只有停止按钮。关闭标签的入口藏在标签右键菜单中，平铺卡片没有可见的关闭入口。

## Architecture Design

UI 使用统一的关闭请求区分平铺窗口和标签目标；确认后复用 Runtime.close_tab，不复制进程控制流程。Runtime 标记 close_requested，并复用现有停止控制器；进程确认退出后关闭对应视图，停止失败则保留视图并展示真实错误。

## Implementation

在停止按钮右侧显示 X。平铺模式确认关闭窗口，标签模式确认关闭当前标签。活动服务确认后先走现有优雅停止与超时强制停止；已结束的记录确认后直接关闭 Console。关闭请求处理中禁用 X，避免重复请求。

## Verification

公开版 `cargo build --release --locked --no-default-features` 通过；`cargo fmt` 与英文 locale JSON 校验通过。当前没有打开的 Console 会话，未启动真实业务服务做交互演练；自动化测试未运行。
