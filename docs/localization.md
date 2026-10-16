# DevStart 多语言 / Localization

## 产品行为

首批语言为简体中文 `zh-CN` 和 English `en`。Preferences.language 默认 `system`；设置内提供原生语言名，保存后即时生效。默认首选系统语言，按用户首选语言顺序选取受支持的中文/英文；没有匹配语言时使用英文。中文区域及脚本标签归入简体中文界面。

macOS 使用 CoreFoundation 的首选界面语言列表，不依赖 Terminal 的 LANG。Windows 语言读取实现使用 GetUserPreferredUILanguages；其他平台读取 LANGUAGE/LC_ALL/LC_MESSAGES/LANG。该检测器不代表新增了 Windows 整体平台支持。GUI 在跟随系统时每两秒检查一次；手动语言选择始终优先。

## 模块和存储

- system_language 只获取系统偏好及确定性匹配。
- i18n 提供 Locale、Language、受作用域限制的翻译上下文和单遍占位符格式化。
- src/locales/en.json 为随二进制打包的 gettext 风格目录；中文源文案是稳定消息键，不请求网络翻译。
- Preferences / Service 负责类型校验和原子持久化；UI 只维护选择草稿。API 参数与机器状态保持原协议。
- 状态、时长、端口提示通过显式带 Locale 的后端展示接口生成。API 默认 DTO 保持既有格式；界面语言不污染业务状态。

原项目名、用户分组名、启动项名、简介、源代码、命令和原始 Console 字节均不翻译。默认分组仅在 ID=1 且名称仍为内置默认值时翻译；自定义同名分组不受影响。应用的通用错误与经审查的诊断前缀翻译，文件名、用户值和外部工具原始诊断保留。

## 格式化与线程边界

占位符按名称替换，只扫描一次模板；用户值中的花括号不会再次解释。英文行数和批量操作使用单复数模板。翻译目录常驻只读内存，逐帧不读取磁盘；UI作用域守卫不可跨线程，析构恢复先前语言，覆盖标签、平铺和即时独立窗口。

添加语言时应扩展 Language / Locale、打包目录及设置选项，并保持消息键及占位符集合一致。CI 检查界面中文静态字面量是否绕过翻译入口、目录是否完整，以及每条译文的占位符是否匹配。不要向翻译函数传入用户名称或 Console 原文。

## 验证

覆盖偏好缺省、三种设置值持久化、非法值整批拒绝、重置恢复 system、系统语言排序/区域匹配、嵌套翻译作用域、占位符注入隔离、单复数、英文时长格式、启动定义和源日志不变；再进行中英文设置及真实窗口目视检查。

## English

Choose Follow system, 简体中文, or English in Settings → Language. Apply and save updates the interface immediately; no service restart is involved. Localization is bundled offline. Your project names, launch aliases, descriptions, source files, commands and raw console output stay exactly as entered. The local API accepts `settings.update {"patch":{"language":"en"}}`; reset restores `system`.
