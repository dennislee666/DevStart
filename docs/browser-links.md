# Console 与网页入口交互

启动项名称单击打开其对应运行会话的标签视图，并切换为 Console。项目、收藏中的名称复用已存在的最新运行记录；没有运行记录时提示先启动，单击名称不会隐式启动服务。平铺标题同样支持单击进入标签视图。

列表访问地址与 Console HTTP(S) 链接直接单击打开，无需修饰键。Console 仍支持拖动选择日志；文件路径保留 Control + 点击打开，以免在选择日志时误开文件。

## 默认浏览器标签复用

macOS 通过 NSWorkspace 查找当前 HTTP(S) 默认浏览器，通过浏览器原生脚本接口读取窗口与标签。Safari 使用其标签接口；提供 Chromium `active tab index` / `tab` 脚本协议的浏览器按能力识别，包括 Chrome、Edge 和 ego lite 等，不强制切换默认浏览器。

打开流程由后端串行执行，UI 异步提交请求、显示真实失败，不阻塞渲染或运行服务轮询：

1. 校验 HTTP(S) URL，规范化协议、主机、默认端口和根路径；路径、参数及片段保留其含义。拒绝控制字符、非网页协议及 URL 内嵌账号密码。
2. Chromium 优先复用此前记录的标签 ID，核对浏览器进程及启动时间，标签重新排序、登录跳转、SPA 路由变化后仍能定位。重新点击入口时在原标签打开目标地址；当前已经是该地址时只激活，不刷新。
3. 没有可用标签身份时，遍历已有窗口，激活同一 URL 的标签；只有找不到时才创建。关闭过的标签或浏览器新会话会重新查找，避免把旧 ID 指向无关标签。
4. 工作空间 `browser-targets.json` 仅记录用户在软件中点击的网址的 SHA-256 指纹与对应原生标签身份，最多 512 项；不保存网址明文、扫描到的其他标签、浏览内容或登录信息。DevStart 重启后仍可复用。Safari 没有稳定的原生标签 ID，仅按当前 URL 查找，不保证重定向后的跨地址身份关联。

不会关闭浏览器中原本已有的重复标签，也不会重命名网页标题。首次操作可能需要用户允许系统“自动化”授权；不要求开启“允许 Apple Events 执行 JavaScript”，脚本只操作浏览器窗口与标签，不执行网页 JavaScript。网址通过进程参数传入，不拼入可执行脚本。默认浏览器不提供兼容接口、系统拒绝或 45 秒无响应时明确报错，不静默降级成重复新建标签。当前非 macOS 平台暂未实现原生标签复用，返回明确错误。

## 技术依据

- [Apple NSWorkspace](https://developer.apple.com/documentation/AppKit/NSWorkspace)：查找 URL 的默认应用。
- [Chromium AppleScript 支持](https://www.chromium.org/developers/design-documents/applescript/)与[原生脚本字典](https://chromium.googlesource.com/chromium/src.git/+/lkgr/chrome/browser/ui/cocoa/applescript/scripting.sdef)：窗口、标签身份、URL 与当前标签索引。

URL 规范化单元检查覆盖默认端口、IPv6、查询参数/片段和危险输入。原生界面验证覆盖名称单击、Console 与列表跨入口重复打开、标签 ID 复用。
