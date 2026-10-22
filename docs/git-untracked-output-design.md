# Git 未跟踪目录输出上限

## Root Cause

Git 工作区调用 `status --porcelain -z --untracked-files=all`，将所有未跟踪目录递归展开。用户选择的仓库因此返回 3,202,254 字节、20,682 条 NUL 记录，超过每条 Git 流 2 MiB 的有界读取上限；仓库本身可正常读取。

## Architecture Design

保留 Git 子进程通用输出上限，并采用 Git 默认的 `--untracked-files=normal` 汇总未跟踪目录。后端通过 `directory` 字段标记折叠的目录项；选择目录提交时仍使用原 pathspec 路径，可一次包含目录下全部文件。目录不伪装成单文件预览或回滚对象。

## Implementation

变更列表使用 normal untracked 模式；目录行显示文件夹图标、说明提交范围并禁用 Diff 预览。`git.changes` API 返回 `directory` 布尔字段。输出超限日志补充命令、stdout/stderr、观测字节下限与上限，不记录文件路径。

## Verification

公开版 Release 编译通过，`git diff --check` 通过。未运行自动化测试，未修改仓库内容或暂存区。
