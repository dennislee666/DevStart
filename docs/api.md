# DevStart 本地 API v1

面向 Codex、Cursor、其它编程 AI 和本机软件。所有接口由正在运行的 Dev Start 实例执行，与界面共用配置、进程状态及日志。无需安装 SDK 或新增依赖。

## 0. 人与 AI 的阅读入口

本文既是人工使用手册，也是 AI 调用契约。**参数与方法名保持原样，示例 ID 仅为占位值，必须换成查询返回的实际 ID。**

- 首次接入：阅读本节 → [连接方式](#1-连接方式) → 调用 `api.describe` 获取当前二进制的参数 Schema。
- 日常操作：按下方任务流程选择方法；需要完整字段时查 [接口清单](#2-接口清单) 和 [启动项配置](#3-启动项配置)。
- 启停与排错：阅读 [启动与状态](#4-启动与状态)、[日志跟踪](#5-日志分页与实时跟踪) 和 [错误与限制](#8-错误与限制)。
- 自动化入口：CLI `--api`、本地 JSON-RPC、MCP `dev_start` 工具均使用相同的方法和参数。GUI 或 `--serve` 必须先运行，客户端必须指向同一工作空间。

### 给 AI 的最小执行约定

1. 先调用 `api.describe`，再查询 `projects.list` / `scripts.list` / `runs.list`；不要编造 ID、路径、参数或端口，不以显示名作为唯一标识。多项同名时用项目路径、所属项目及入口参数确认目标。
2. 只有获得用户授权的启动、停止、重启才执行；创建项目或启动项不会自动启动。部署、打包、上传不能因“启动项目”而顺带执行，禁用入口也不能擅自启用。
3. 使用后端 API 修改配置，不直接覆盖 catalog；配置成功后读取对应对象确认。添加/刷新读取项目自己的启动文件，不依赖 IDE 配置。
4. 启动/重启返回后继续查询 `runs.get` 和日志。`state:running` 只证明进程已创建，业务就绪需要项目自身日志或用户指定健康检查；不要宣称数据库/API 可用。监听端口同样不代表健康检查成功。
5. 停止返回后查询 `active:false` 才确认结束；重启使用返回的**新 run_id**跟踪，使用稳定的 console_id 关联原标签。
6. 超时、断线或响应不明时先查询实际状态，不能盲目重发增删改或启停请求。保留真实 error.message，向用户解释失败对象、原因与下一步。
7. 日志分页只传上一页 next_offset，不按字符数推算。查看日志无需打开外部程序；`logs.open_target` 仅在用户要求打开时调用。
8. 批量操作先生成计划，展示 targets/blocked 并获得确认，随后原样提交计划；配置或状态变化时重新生成并确认，不能伪造 confirmed 或重建计划。

### 常见任务与调用顺序

| 用户目标 | 调用顺序与完成条件 |
|---|---|
| 新建/修改/删除分组 | `groups.list` → `groups.create/update/delete` → 再读列表；默认组不可删除，删其它组仅把项目移回默认组 |
| 添加项目 | 检查绝对路径 → `projects.create` → `scripts.list`；未传 group 使用默认组，不自动运行 |
| 修改项目名、描述或分组 | `projects.get` → `projects.update` → 再读项目；展示更新不改变路径、不重启 |
| 增加或修改执行入口 | `scripts.list/get` → 必要时停止目标并等 active:false → `scripts.create/update` → 再读；只提交要变更的字段 |
| 修改显示名和简介 | `scripts.update` 的一个 patch 同时传 display_name/description → 再读；允许运行中修改，不改变 argv/PID |
| 启动或重启 | 找到 script_id → `runs.start/restart` → 保存返回 run_id/console_id → 查询状态并增量读取日志 |
| 停止 | 从 runs.list 确定当前实例 → `runs.stop` → 轮询 runs.get，直到 active:false；保留实际退出状态 |
| 查错误日志 | `logs.search` 查询当前有界缓存；需要保留范围内的历史内容时 `logs.list` → `logs.read` 分页，不全量灌入 AI 上下文 |
| 删除启动项/移除项目 | 先查关联活动实例 → 停止并等退出 → `scripts.delete/projects.delete` → 再读列表；不会删除原项目目录或原脚本 |
| 批量启停 | `runs.bulk_plan` → 展示目标并确认 → `runs.bulk_apply` → 检查 errors 并跟踪返回的 run_ids |

### 标识、精度与工作空间

| 字段 | 含义 | 使用位置 |
|---|---|---|
| group / 分组 id | 分组身份，默认组为 1 | projects.create/update 的 group |
| project_id / 项目 id | 项目登记身份 | projects.get/update/delete/refresh，scripts.list/create |
| script_id / 启动项 id | 启动入口身份 | runs.start/restart、editor.*；scripts.get/update/delete/status/ports 参数名为 id |
| run_id / 运行实例 id | 一次执行的身份；重启后变化 | runs.get/stop/open/close 参数名为 id；logs.* 通常为 run_id |
| console_id | 标签/Console 的稳定身份 | 用于客户端关联；不能代替 runs.get/stop 的 id |
| revision / version_id | 乐观并发摘要 / 文件备份版本 | 编辑器按原样传回；不要自行生成 |

运行/Console ID 可能超过 JavaScript 安全整数范围。普通 id/run_id 参数目前要求**精确 JSON 整数**，不接受随意改成字符串；Python 标准 json 保留整数精度。Node.js 普通 JSON.parse 会损失大整数，运行接口应使用无损 JSON 解析/序列化方案，或委托下文 Python 客户端处理。批量计划的 targets[].run_id 与编辑器版本 id 明确为十进制字符串，必须保持原类型。

个人版默认 macOS 数据目录末级名称为 `dev-start`；开源版为 `devstart`（均位于 `~/Library/Application Support/` 下）。调用 `api.describe` 查看 edition；使用 `DEV_START_DATA_DIR` 时 GUI、headless、CLI、MCP 必须一致。两版不会自动共享配置，调用到空工作空间不代表个人配置丢失。下文默认目录示例以当前文档对应的版本为准；显式覆盖时使用真实目录。

### 状态与结果判定

| state | active | AI 应如何处理 |
|---|---|---|
| starting | true | 预检或启动中，继续查询和读取日志 |
| running | true | 进程已创建；结合项目日志确认业务就绪 |
| stopping | true | 正在退出，继续查询，不开始依赖“已停止”的修改 |
| succeeded | false | 进程正常结束；对一次性任务可成功，对常驻服务应说明其已退出 |
| failed | false | 失败；读取日志与 exit_code，不自动循环重启 |
| stopped | false | 停止完成，可进行需停机的配置修改 |

`scripts.status.label` 是界面简化列，仅为“运行中”或 `--`，不能代替完整运行状态。停止使用 TERM 宽限 5 秒，未退出再 KILL 并复核；相关事件进入 Console。未知归属的端口占用会报错，不应由 AI 另行按端口盲目杀进程。外部运行实例原终端的历史 stdout 无法补取，重启接管后才有新输出。

### 重试与失败处理

| 情况 | 处理策略 |
|---|---|
| 字段校验失败/未知字段 | 对照当前 api.describe 修正参数；不要原样重复提交 |
| 请求超时、断线、CLI 非零退出 | 检查 JSON error 和 stderr；先查询对象及 runs.list 判断是否已执行 |
| 创建请求结果不明 | 先按路径/分组/来源入口查询，避免重复创建；API 没有通用幂等键 |
| 启停结果不明 | 查询原 run_id 及关联 script_id 的新实例；确认后再决定后续动作 |
| 编辑冲突 | 保留草稿，重新读取 revision/磁盘状态并解决冲突；不能自动 discard=true |
| 批量计划失效 | 重新获取计划并核对目标；不得修改旧 revision 以绕过检查 |
| 多目标部分失败 | 分别报告已提交、同步 errors 和后续异步失败；批量执行不是全有或全无事务 |

## 1. 连接方式

正式可执行文件：

```bash
DEV_START='/absolute/path/DevStart.app/Contents/MacOS/dev-start'
"$DEV_START" --api api.describe
"$DEV_START" --api api.docs
"$DEV_START" --api config.get
```

先打开桌面应用。也可在不需要GUI的独立工作空间运行：

```bash
DEV_START_DATA_DIR=/tmp/dev-start-agent "$DEV_START" --serve
```

`--serve` 与GUI都是服务所有者；同一工作空间只允许一个所有者，客户端不占工作空间锁。SIGINT/SIGTERM会正常退出headless服务并清理受管进程。API/CLI/MCP均继承 `DEV_START_DATA_DIR`，默认macOS工作空间为 `~/Library/Application Support/devstart`，Linux为 `~/.local/share/devstart`。API错误时CLI退出码1，成功为0；JSON响应写stdout，诊断写stderr。

```bash
"$DEV_START" --api groups.create '{"name":"示例工作区"}'
"$DEV_START" --api projects.create @/absolute/path/request.json
# 从stdin读取，避免环境变量等配置进入shell历史/命令行参数：
"$DEV_START" --api scripts.update - < /absolute/path/request.json
```

客户端连接 `<工作空间>/api/control.sock`。这是 **Unix域socket的JSON-RPC，不是HTTP**；不要用curl HTTP请求此socket。目录权限0700，socket0600，仅本机当前用户可访问；没有TCP监听、外部账号、访问令牌或浏览器入口。同用户AI进程拥有配置与显式启停权限，保护工作空间目录即可控制访问。

每个连接发送一行UTF-8 JSON（含末尾换行），读取一行响应后断开。每次只发送一个请求，不支持batch。客户端使用带id的请求，不发送通知。协议为JSON-RPC 2.0：

```json
{"jsonrpc":"2.0","id":"agent-1","method":"groups.list","params":{}}
```

```json
{"jsonrpc":"2.0","id":"agent-1","result":[{"id":1,"name":"默认分组"}]}
```

请求上限2MiB，响应上限8MiB；响应过大时按项目拆分查询。4个socket工作线程、16个待连接、32个命令队列。读取请求2秒超时，服务处理等待15秒，CLI读取20秒。超时或连接中断不表示操作未执行；查询当前配置/运行状态后再决定是否重试。运行启动是异步操作，返回run_id不等于程序已就绪。

## 2. 接口清单

所有 params 必须是对象，未知字段报错。下表列出主要管理方法；editor.* 的完整参数表见下方“内置启动文件编辑器”，所有 46 个方法的机器可读参数以 api.describe 为准。返回值包装在JSON-RPC的result内；错误在error内。`id`参数指分组、项目、启动项或运行实例的对应id，不能混用。返回的id由服务分配，调用者不指定。

| 方法 | params | result / 行为 |
|---|---|---|
| `api.describe` | `{}` | 版本、限制、方法与完整参数JSON Schema |
| `api.docs` | `{}` | 内置完整Markdown文档，不依赖当前目录 |
| `settings.get` | `{}` | 当前持久化外观设置 |
| `settings.update` | `{"patch":{"console_font_size":14}}` | 验证、保存并实时应用设置 |
| `settings.reset` | `{}` | 恢复默认设置并保存 |
| `config.get` | `{}` | 完整catalog配置（含分组、项目、启动项） |
| `groups.list` | `{}` | 分组数组 |
| `groups.create` | `{"name":"组名"}` | 新分组对象 |
| `groups.update` | `{"id":2,"name":"新组名"}` | 分组id |
| `groups.delete` | `{"id":2}` | 删除分组，其项目移入默认组1 |
| `projects.list` | `{}` | 项目数组，包含各项目scripts |
| `projects.get` | `{"id":10}` | 项目对象 |
| `projects.create` | `{"path":"/absolute/project","group":2,"name":"项目名","description":"说明","favorite":false}` | 导入并发现项目自己的启动项（无清单时仅按启动命名协议）；仅path必填 |
| `projects.update` | `{"id":10,"patch":{"name":"新名","description":"新描述","group":1,"favorite":true}}` | 修改后的项目对象；patch中字段均可选 |
| `projects.delete` | `{"id":10}` | 移除登记；保留项目目录和原启动清单 |
| `projects.refresh` | `{"id":10}` | 重新读取项目清单，返回项目对象 |
| `scripts.list` | `{"project_id":10}` 或 `{}` | 指定项目或全部启动项数组 |
| `scripts.ports` | `{"id":12}` | 启动前配置/推导端口、实际端口、显示值与来源 |
| `scripts.status` | `{"id":12}` | `{id,label}`；仅确认运行中显示“运行中”，其它为 `--`，完整状态见 runs.get |
| `scripts.get` | `{"id":12}` | 启动项对象 |
| `scripts.create` | `{"project_id":10,"spec":{…}}` | 新启动项对象；spec见下文 |
| `scripts.update` | `{"id":12,"patch":{…}}` | 修改后的启动项对象；字段见下文 |
| `scripts.delete` | `{"id":12}` | 从项目清单删除该启动项，保留原脚本文件 |
| `runs.list` | `{}` | 本次服务的活动实例、全部打开标签及最近8条已关闭记录（含tab_open、close_requested） |
| `runs.get` | `{"id":123}` | 运行实例详情 |
| `runs.start` | `{"script_id":12}` | `{run_id,state:"starting",asynchronous:true}` |
| `runs.restart` | `{"script_id":12}` | 在原console_id内替换进程，返回新run_id；无历史时启动 |
| `runs.stop` | `{"id":123}` | 请求优雅停止，已停止时幂等；异步确认见runs.get |
| `runs.open` | `{"id":123}` | 重新打开保留的运行历史标签 |
| `runs.close` | `{"id":123,"stop":true}` | 活动标签须明确 stop:true，退出后关闭；结束标签可省略 stop |
| `runs.bulk_plan` | `{"action":"stop"}` / `start` / `restart` | 只读生成当前打开标签的目标与阻止原因 |
| `runs.bulk_apply` | `{"confirmed":true,"plan":原样计划对象}` | 确认并验证计划后执行；结果含 run_ids 和 errors，继续查询异步结果 |
| `runs.stop_all` | `{}` | 请求停止全部受管/已核实运行实例 |
| `logs.search` | `{"run_id":123,"keyword":"error","case_sensitive":false,"offset":0,"limit":100}` | 搜索当前Console缓存，分页返回匹配行与高亮范围 |
| `logs.open_target` | `{"run_id":123,"target":"./src/main.rs:20"}` | 打开外部浏览器/文件程序；仅用户明确要求时调用 |
| `logs.list` | `{"after_id":0,"limit":100}` | 持久化日志索引，含logs、next_after_id、has_more |
| `logs.read` | `{"run_id":123,"offset":0,"limit":16384}` | 有界日志页与下一字节游标 |

机器可读协议：[api-schema.json](api-schema.json)。`api.describe`内嵌相同内容，可让AI先调用它发现字段。

### 默认分组

默认组固定id为1，初始名称“默认分组”，可重命名但不可删除。创建项目未传group或传null时使用1；修改项目group:null也表示移回默认组。指定不存在的组会报错，不静默创建。历史配置中没有有效分组的项目在加载时迁入组1。删除其它组不会删除其项目。

### 项目对象

`id, name, description, path, group, favorite, scripts, explicit_scripts`。路径在导入时规范化为真实绝对目录；修改项目路径请移除登记后重新导入，避免旧运行与来源被错误关联。`projects.update`仅接受name/description/group/favorite。其它字段为后端生成，不接受直接覆盖catalog。

## 3. 启动项配置

启动项仍引用项目自己的程序。API不会复制业务代码，也不会调用VS Code或其它IDE运行配置。

```json
{
  "project_id": 10,
  "spec": {
    "name": "H5 本地调试",
    "file": "package.json",
    "entry": {"Package":{"manager":"npm","name":"dev:h5:local"}},
    "description": "启动本地H5调试服务",
    "cwd": ".",
    "args": ["--port","5173"],
    "environment": {"NODE_ENV":"development"},
    "expected_port": 5173,
    "resource_key": "h5-development",
    "enabled": true,
    "display_command": "npm run dev:h5:local -- --port 5173"
  }
}
```

spec必填 `name,file,entry,description`。路径相对项目根目录解析，file必须是真实文件、cwd必须为目录；cwd省略或null时为项目根目录。args为字面参数数组，environment为环境变量对象，不进行shell字符串拼接。Package额外参数按包管理器规则加`--`。description说明用途；display_command只是界面/tooltip展示摘要，**不会改变执行命令**，null恢复自动摘要。

可选字段：args默认[]、environment默认{}、expected_port默认null（1–65535）、resource_key默认null（有值时不可为空）、enabled默认true、display_command默认null。Java Entry本身的enabled与顶层enabled必须同时允许，才能执行。

entry采用下列一种结构，不能混合：

```json
{"File":{"interpreter":"/bin/bash"}}
```

file为原Shell/Python脚本。解释器也可为python3等。

```json
{"Package":{"manager":"npm","name":"dev"}}
```

file为原package.json，其scripts内必须存在dev。不会改写package.json脚本。

```json
{"Command":{"program":"/absolute/tool"}}
```

program是可执行程序，命令参数放args；file仍需指向该入口所属的项目原文件作为来源记录。

```json
{"Java":{
  "program":"/absolute/jdk/bin/java",
  "main_class":"com.example.Main",
  "vm_args":["-Dfile.encoding=UTF-8"],
  "class_paths":["starter/target/classes"],
  "classpath_file":".classpath.txt",
  "enabled":true
}}
```

file指向原.java文件。classpath_file可选，引用项目已有依赖路径列表；不自动编译或执行Maven生命周期。configuration/snapshot/manifest是服务生成的元数据，不要从scripts.get返回对象中直接复制为新spec。Java约束详见[项目清单](project-launch-manifest.md)。

scripts.update的patch可包含spec的全部字段，额外支持favorite布尔值。entry/environment/args是**整体替换**，不是深度合并；只修改描述时不要传其它字段。

```bash
"$DEV_START" --api scripts.update '{"id":12,"patch":{"description":"本地接口联调","display_command":"java com.example.Main test"}}'
"$DEV_START" --api scripts.update '{"id":12,"patch":{"args":["test"],"expected_port":8082}}'
"$DEV_START" --api scripts.update '{"id":12,"patch":{"enabled":false}}'
```

启动项增删改会写回该项目根目录.dev-start.json，再重新校验，成功才提交应用配置。首次编辑无清单项目时，会把当前登记的启动项写为明确清单。之后刷新读取此清单，不再自动附加其它发现项。删除最后一项保留空清单，避免刷新重新发现被删项。删除项目登记不删除清单。

运行中的目标启动项禁止修改执行配置或删除，运行中的项目禁止移除登记；先 runs.stop 并轮询 active:false。纯 display_name/description 更新是例外：可在运行中原子修改，不改变启动清单、参数或 PID；混入任何其它字段时仍需停止。新增其它启动项、修改项目元数据、移动分组不需要停止。应用刷新后保留API编辑与id；重新命名仍保留id。清单校验失败回滚原字节；并发外部编辑检测到时拒绝覆盖，先refresh再重试。跨进程崩溃不承诺多文件事务，恢复后以项目清单为权威并执行refresh。

## 4. 启动与状态

```bash
"$DEV_START" --api runs.start '{"script_id":12}'
"$DEV_START" --api runs.get '{"id":123}'
"$DEV_START" --logs 123
"$DEV_START" --api runs.stop '{"id":123}'
```

每次start/restart都复用同一预检：核实同启动项/资源的已运行实例，TERM宽限5秒，仍未退出则KILL，确认退出后启动。未知端口占用不会猜测性关闭；失败原因写入该run的Console和日志。

状态为starting/running/stopping/succeeded/failed/stopped。running表示进程已经创建，**不是业务健康检查通过**。active覆盖前三个状态。运行详情含title、launch_name、script_id、associated_scripts、pid、listening_ports、external、exit_code、elapsed_seconds、display_command、cwd和log_path。只有本次服务进程的运行记录可启停；之前的磁盘日志仍可读取。

## 5. 日志分页与实时跟踪

logs.read可指定offset（默认0）或tail_bytes（二选一），limit默认16384字节、上限65536。tail_bytes也最多65536。返回：

```json
{"run_id":123,"offset":0,"next_offset":6,"size":6,"base_offset":0,"retained_bytes":6,"text":"ready\n","bytes":[114,101,97,100,121,10],"active":true,"reset":false,"eof":true}
```

offset/next_offset/size 是**累计原始字节游标**，不能按字符串长度或当前文件物理大小计算；size 是当前累计末尾，保留范围为 `[base_offset, size)`。未裁剪时 base_offset=0，行为与原字节偏移一致。retained_bytes 为当前保留的输出字节数，日志文件的内部游标头不计入输出。bytes 是保留范围内的无损原始字节；text 是便于阅读的 UTF-8 有损解码，跨页或超长单行裁剪可能处于字符中间，精确还原应拼接 bytes 或使用增量解码器。

请求 offset 早于 base_offset 或晚于 size 时，reset:true，并从当前 base_offset 返回，不一定是0。客户端必须记录历史已被裁剪，使用响应 next_offset 继续，不得把缺口当作“无错误”。eof 仅表示当前读到末尾，active:true 时仍可能继续输出。

跟踪流程：每次使用next_offset发起下一次logs.read，eof且active时等待300ms再查，active:false且eof时结束。`--logs RUN_ID [OFFSET]`已封装此流程，直接输出原始字节。磁盘分页读取不占 Console 缓存；UI 缓存仍为每实例3000行/2MiB和全局30000行/32MiB。磁盘日志也受下述保留上限约束。

重启应用后先 logs.list 查询仍保留的 run_id，再 logs.read；历史文件可能已按配额或过期策略清理，不保证永久存在。日志页只返回实际文件内容；外部识别服务的历史stdout无法被接管，因此只保留监测和操作记录及重启后的新输出。

### 磁盘保留策略

| 范围 | 默认硬上限 | 超出处理 |
|---|---|---|
| 单个运行日志 | 10000 行或 10 MiB，先触及者生效 | 裁掉最旧内容，保留约8000行/8MiB的最新片段，给后续输出留空间 |
| 工作空间日志合计 | 200 MiB、50份 | 优先清理最旧修改时间的运行日志；运行中的旧日志也可能被淘汰，下次输出可重新记录，不停止项目 |
| 历史保留时间 | 最后写入后7天 | 启动时及约每30秒清理 |

限制仅针对 DevStart 数据目录 `logs/run-数字.log`，不删除原项目文件，也无法限制项目自己另行写出的业务日志。行数包括换行记录；超长单行还受字节上限保护。日志文件可能多一行内部游标说明，业务内容会预留该行，文件总行数仍不超过10000行。原子裁剪时可临时额外占用约8MiB，保留配额不包含文件系统元数据。

`api.describe.log_retention` 返回上述策略。`logs.list.size` 是当前磁盘文件物理字节数，`logs.read.size` 是累计输出末尾游标，两者在裁剪后不可混用。已清理且未重新输出的历史文件不在 logs.list 中；读不到文件时应报告历史不可用，不得伪造日志。活动实例日志被回收且尚未重建时可返回空页和 reset:true。

旧版本已有日志也会在新版首次打开时按同一规则整理。Console 缓存和磁盘日志分别限额，“打开日志文件”仅查看仍保留的片段，不再承诺完整永久历史。需要长期留档时，使用者应主动导出所需内容到自己管理的位置。

## 6. Python与Node调用

```python
import json, os, socket
from pathlib import Path
root = Path(os.environ.get("DEV_START_DATA_DIR", str(Path.home()/"Library/Application Support/devstart")))
def call(method, params=None):
    with socket.socket(socket.AF_UNIX) as client:
        client.settimeout(20)
        client.connect(str(root/"api/control.sock"))
        client.sendall((json.dumps({"jsonrpc":"2.0","id":1,"method":method,"params":params or {}})+"\n").encode())
        with client.makefile("rb") as stream:
            line = stream.readline(8*1024*1024+1)
        if len(line) > 8*1024*1024: raise RuntimeError("response too large")
        response = json.loads(line)
        if "error" in response: raise RuntimeError(response["error"])
        return response["result"]
print(call("groups.list"))
```

### 可复用的启动跟踪示例（Python）

下面复用上面的 `call` 函数。仅在用户已授权启动时执行，`script_id` 必须来自前一步查询，不能直接使用示例值。客户端自己的等待超时不等于服务启动失败，也不会因此自动停止服务。

```python
import time

def start_and_inspect(script_id, wait_seconds=20):
    script = call("scripts.get", {"id": script_id})
    started = call("runs.start", {"script_id": script["id"]})
    run_id = started["run_id"]  # Python int 保留精确整数，不转 float。
    deadline = time.monotonic() + wait_seconds
    while True:
        run = call("runs.get", {"id": run_id})
        if run["state"] != "starting" or time.monotonic() >= deadline:
            break
        time.sleep(0.3)
    recent = call("logs.read", {"run_id": run_id, "tail_bytes": 16384})
    return {"run": run, "recent_log": recent["text"],
            "next_offset": recent["next_offset"],
            "business_ready": "未验证：需按项目日志或指定健康检查判断"}
```

后续日志查询：`logs.read {"run_id":当前运行ID,"offset":上一页next_offset}`；停止：`runs.stop {"id":当前运行ID}` 后轮询 `runs.get` 至 `active:false`；重启：`runs.restart {"script_id":启动项ID,"id":原运行ID}`，并用返回的新 run_id 继续跟踪。示例不会自动停止、重启或修改启动项。

Node 可用 execFile 调用同一个 CLI，不经过 shell。以下 groups.list 示例仅解析小整数分组 ID；涉及 run_id/console_id 时需使用无损整数解析与序列化，不能直接照搬 JSON.parse：

```javascript
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
const exec = promisify(execFile);
const binary = '/absolute/path/DevStart.app/Contents/MacOS/dev-start';
const { stdout } = await exec(binary, ['--api', 'groups.list', '{}'], { maxBuffer: 8*1024*1024 });
const response = JSON.parse(stdout);
console.log(response.result);
```

## 7. MCP接入Codex / Cursor

内置stdio桥接器：`dev-start --mcp`。桥接器仅连接已运行GUI或headless服务，不另建工作空间或自动启动项目。使用[官方MCP stdio协议](https://modelcontextprotocol.io/specification/2025-06-18/basic/transports)及[工具协议](https://modelcontextprotocol.io/specification/2025-06-18/server/tools)，协议版本2025-06-18。

Cursor或采用mcpServers格式的客户端配置示例（合并到已有MCP配置，不覆盖其它server）：

```json
{
  "mcpServers": {
    "dev-start": {
      "command": "/absolute/path/DevStart.app/Contents/MacOS/dev-start",
      "args": ["--mcp"]
    }
  }
}
```

Codex 使用 `~/.codex/config.toml` 中的配置：

```toml
[mcp_servers.dev_start]
command = "/absolute/path/DevStart.app/Contents/MacOS/dev-start"
args = ["--mcp"]
```

也可以自行运行：

```bash
codex mcp add dev_start -- "/absolute/path/DevStart.app/Contents/MacOS/dev-start" --mcp
```

格式依据[OpenAI官方MCP文档](https://learn.chatgpt.com/docs/extend/mcp?surface=cli)。

如使用独立工作空间，在该server配置的env内添加DEV_START_DATA_DIR，与运行的服务一致。没有自动修改Codex/Cursor现有设置，避免覆盖其它MCP配置。

桥接器提供一个工具dev_start，参数为`{"method":"groups.list","params":{}}`。先调用api.describe取得所有方法及参数Schema，再调用需要的方法。

可直接交给AI的指令：

> 用dev_start工具先调用api.describe、projects.list、scripts.list了解当前登记。按用户要求增删改分组/项目/启动项，显示命令只修改display_command。不要直接覆盖catalog，不依赖IDE配置。只有用户明确要运行时才调用runs.start/runs.restart；从返回run_id用runs.get和logs.read跟踪。部署/打包/上传入口保持禁用，除非用户明确授权该操作。未分组项目使用默认组1，未知端口占用不强杀。

## 8. 错误与限制

- -32600：无效JSON-RPC请求。
- -32601：方法不存在。
- -32602：params类型不正确。
- -32000：配置校验、找不到对象、运行中保护、来源读写失败等，message含真实原因。
- -32001：socket请求解析/超时/队列繁忙；执行结果可能未知，先查询再重试。
- -32002：响应超过8MiB，请缩小查询范围。

CLI返回JSON-RPC结果并以退出码表示成功/失败；MCP工具执行失败通过isError:true返回原因。API不提供任意文件写入或删除原项目目录的操作；原脚本代码修改继续用编辑器。传输使用 Unix socket；macOS 已验证，Linux 尚未完成端到端平台验证；不提供 Windows 命名管道。没有新增外部下载源；当前功能使用Rust标准库和既有serde依赖。

无项目清单时的启动发现规则：名称按冒号/连字符/下划线拆段，首段为dev/start/serve/watch，且不含build/deploy/upload/publish/release/test/lint/format/install任务段。其它名称请用项目清单或scripts.create明确登记；现有显式清单不受命名过滤。读取本身不执行代码，命名约定不等于源码行为验证。

## 9. 外观与字体设置

GUI的“设置”与API共用catalog.preferences，重开应用保留。settings.update只更新patch中明确的字段，其余保留；未知字段、非法枚举和超范围数值都报错，失败不写配置。

| 字段 | 可选值 / 范围 | 默认 |
|---|---|---|
| theme | dark / light / system | dark |
| ui_font | system / monospace | system |
| ui_font_size | 10–18 | 12 |
| console_font | system / menlo / monaco（后两项macOS字体） | system |
| console_font_size | 10–24 | 12 |
| console_line_height | 1.0–1.8 | 1.25 |
| density | compact / comfortable | compact |
| sidebar_collapsed | boolean | false |

```bash
"$DEV_START" --api settings.update '{"patch":{"theme":"dark","console_font":"menlo","console_font_size":14,"density":"compact"}}'
"$DEV_START" --api settings.get
```

GUI字体设置提供局部预览，通过“应用并保存”统一提交，避免滑块拖动时持续写盘。API修改实时同步已打开的界面。侧栏按钮或Cmd+B即时保存收展状态；Cmd+,打开设置。主界面、Console、项目启停与独立Console复用相同主题与字体。

## 10. Console 关键词搜索

logs.search复用GUI搜索器，只查询当前服务中该run保留的Console缓存（每实例最多3000行/2MiB），不加载完整磁盘文件，也不修改GUI的查询条件。仍保留的历史磁盘记录用 logs.read 按累计字节游标读取。keyword为字面子串，最多256字符；默认Unicode小写匹配，case_sensitive:true区分大小写，不做正则或模糊推断。空关键词返回全部缓存行，matched_lines为0。

offset为过滤后结果的行偏移，limit默认100、最多200；返回source:"console_cache"、cached_lines、dropped_lines、matched_lines、rows、next_offset、has_more。每个row含cached_row、line_number、stderr、text与highlights。highlights的start/end是text原UTF-8字节的闭开区间，每行最多64处，用于显示而非总命中次数。line_number为当前Console缓冲的序号（包含已淘汰行计数），不是磁盘原始字节或原始ANSI文件行号。日志持续输出时索引会变化，分页不保证跨请求快照；实时轮询可用相同keyword重新查询。

```bash
"$DEV_START" --api logs.search '{"run_id":123,"keyword":"错误","limit":50}'
"$DEV_START" --api logs.search '{"run_id":123,"keyword":"ERROR","case_sensitive":true}'
```

GUI点击放大镜或主窗口Cmd+F展开搜索；Aa控制大小写，箭头/Enter/Shift+Enter跳转，跳转暂停跟随输出。“只看匹配”仅过滤当前视图；可勾选跟随输出继续查看新的相关行。关闭搜索恢复完整缓存视图，原始日志不变。查询按运行实例独立，主标签、平铺与独立窗口复用同一组件。

## 11. 实际运行端口

runs.get和runs.list的每个运行对象新增listening_ports（升序去重的TCP端口数组）。后台约每2秒读取实际监听快照，关联启动进程专用进程组及子进程；外部实例只关联已核实身份的进程与其后代，不包含共享终端的其它服务。

例如`"listening_ports":[8000,9229]`表示实际检测到这两个TCP监听端口。`[]`表示未运行、还未检测到或当前无法确认实际监听。GUI端口列优先显示实际listening_ports；无实际结果时显示启动项expected_port，二者均未知才显示`--`，tooltip标注来源。该字段不会用配置expected_port填充，也不会修改启动参数。停止后listening_ports清空，GUI若有配置则仍显示配置端口；一个程序监听多个端口会全部返回。UDP等本次未检测范围同样显示未知。此接口为只读观测，不执行启动或端口清理。


### 启动前端口来源

scripts.ports返回script_id、configured_port、configuration_source、configuration_error、actual_ports、display、tooltip、source（actual/configured/unknown）。GUI与该API采用同一展示协议：实际监听优先；没有实际结果时使用明确配置或可静态解析的启动端口；确实无法确定时才显示--。

配置解析顺序：启动清单expected_port；字面--port/--server.port及JVM的-Dserver.port/-Djboot.server.port；package.json中启动命令及额外参数；单条静态Shell启动命令；Python中绑定HTTP服务的静态默认port与sys.argv[1]回退；Vite server.port/preview.port与Vue devServer.port的数字字面量。Next CLI支持-p，Uni的-p是平台选项，不作端口解析；mp-weixin等编译监视入口不会套用H5的Vite端口。优先明确声明的expected_port，不将推导值写回expected_port或改变启动约束。

不执行Shell、JavaScript配置、构建、上传；不从代理地址、依赖服务URL或常见默认端口猜测。动态表达式、环境变量插值、多个含糊配置、非字面Python默认端口及配置语言条件分支/对象展开等不能静态确定时返回unknown，必要时在项目清单明确expected_port。配置结果在加载、重新读取项目启动项或API修改后刷新并缓存，UI每帧不读磁盘。外部修改配置后可调用projects.refresh。

### 启动项显示名

`name` 保留项目读取的原始入口名；`display_name` 是 DevStart 本地保存的独立显示别名，默认 `null`。使用 `scripts.update`，例如 `{"id":24,"patch":{"display_name":"Java-Starter"}}`；传 `null` 恢复原名。服务端去除首尾空格，拒绝空白、控制字符和超过 50 个 Unicode 可见字符的别名。仅修改显示名可在运行中执行，立即更新项目列表、会话和标签，不改变源文件、入口、参数或运行进程。重新读取启动脚本、编辑其他配置时保留别名。与执行字段混合修改仍要求先停止。列表名称最多显示 50 个字符（含省略号），标签保持 12 个字符上限，tooltip 保留完整显示名。

### 内置启动文件编辑器

所有方法均通过相同本地 Unix socket/CLI/MCP 调用。只能访问已注册启动项的 `file`，不接受任意路径。不会执行脚本来读取内容；禁用的部署/生成启动项也可编辑，但仍不可运行。

| 方法 | 参数 | 作用 |
|---|---|---|
| `editor.list` | `{}` | 打开的文件优先、最近使用优先；返回项目、启动项、路径、dirty、open、tooltip |
| `editor.open` | `script_id` | 打开/切换已有缓冲区，返回文档 |
| `editor.read` | `script_id` | 读取已打开的草稿和版本信息 |
| `editor.change` | `script_id, expected_revision, content` | 更新内存草稿，不写磁盘 |
| `editor.save` | `script_id, expected_revision` | 冲突校验、版本保留、原子保存，保留文件权限 |
| `editor.reload` | `script_id, expected_revision, discard?` | 重新载入磁盘；未保存时需显式 discard=true |
| `editor.close` | `script_id, expected_revision, discard?` | 关闭到历史；未保存时默认拒绝关闭 |
| `editor.versions` | `script_id` | 最近 10 次版本元数据，不返回文件内容 |
| `editor.restore` | `script_id, expected_revision, version_id` | 回退并直接写入源文件；需先保存/放弃未保存草稿 |

版本 `id` 以十进制字符串返回，`version_id` 直接传该字符串，避免 JavaScript/AI 客户端丢失 64 位整数精度（也接受精确整数）。

文档返回 `content, revision, disk_revision, dirty, language, path, script_id`。`revision` 为当前草稿 SHA-256；所有草稿变更请求携带读取到的 `expected_revision`，避免 AI/API 与 UI 相互覆盖。`disk_revision` 为打开/上次保存的磁盘摘要；外部修改会使保存和回退失败，草稿保留，可重新读取后决定保存或放弃。文件限制为 UTF-8、无 NUL、1 MiB；最多 16 个打开文件，最多 10 个已关闭历史记录。退出 GUI 时未保存文件会要求选择保存、放弃或取消；强制终止进程不会保存草稿，已保存版本及历史持久保留。

示例顺序：`editor.open {"script_id":24}` → `editor.change {"script_id":24,"expected_revision":"返回的revision","content":"新的完整文件内容"}` → `editor.save {"script_id":24,"expected_revision":"change返回的revision"}`。保存返回新 revision。用 `editor.versions` 选择 version_id，再以最新 revision 调用 `editor.restore`。回退后磁盘立即是该版本，下一次 `runs.start` / `runs.restart` 使用当前磁盘文件。

备份位于数据目录 `source-versions/<script_id>/`，目录 0700、内容 0600；首次打开保存原始基线，保存、源文件变化后的启动及回退保留版本，修改前外部内容也会保留。每个版本附带源文件的修改时间与字节大小，用于启动备份去重。每启动项仅保留最近 10 个版本，包含回退本身，版本与源文件路径绑定；旧版本超过上限后自动删除。无更改保存不制造重复版本。启动时若文件修改时间与大小都和最近版本一致，或内容版本摘要一致，则跳过备份；只有两个判断都不匹配时才创建版本。回退不自动启动任何命令。

Java `.java` 若曾经由内置编辑器保存/回退，后续启动检查会用该启动项 JDK 同目录的 `javac`，仅编译对应源文件，禁用注解处理，不执行 Maven/Gradle、上传、部署或生成器主类。编译在后台、原服务停止之前完成，60 秒超时或错误会取消启动；输出进入对应 Console。编译成功后才优雅切换服务并应用 class，确保 Java 源文件回退也在下次启动生效。没有内置编辑保存记录的 Java 项目保持原有已编译启动行为。Python、Rust、前端等仍使用项目自己的运行命令和编译流程，DevStart 不代写业务构建脚本。

### 项目与分组重命名

GUI 的重命名入口复用 `projects.update {"id":项目ID,"patch":{"name":"example-java"}}` 和 `groups.update {"id":分组ID,"name":"示例开发组"}`。仅修改展示元数据，可在启动项运行或文件编辑期间执行，不修改目录、启动项、文件、命令或参数，不触发重启。项目名去除首尾空格；名称需非空且无换行/控制字符，分组名不能重复。运行标题和编辑器关联名称即时同步；默认分组的名字也可修改，ID 和默认分组语义保持不变。

### 启动时间和运行时长

`runs.list` / `runs.get` 增加 `started_at`（本地 `YYYY-MM-DD HH:mm:ss`，未真正启动或无法确定则 null）、`started_at_unix_ms`（Unix 毫秒或 null）、`elapsed_display`、`timing_display`。保留 `elapsed_seconds` 数值字段。正常显示如 `2026-10-15 08:30:00 - 1天2小时3分钟4秒`；零天/小时/分钟省略，秒始终出现，例如 1279 秒显示 `21分钟19秒`，3600 秒显示 `1小时0秒`。结束后时长固定。

本工具启动的进程以成功创建进程时为准，预检查时间不计入运行时长。外部进程从系统 `ps lstart` 获取实际启动时间，初始化其已运行时长，再用单调计时递增；无法确定时明确显示“启动时间未知 - 已监测 …”，不会把发现时间当作启动时间。编辑器版本日期与运行时间使用相同本地时间格式。

`editor.list` 的已关闭历史上限为 10 个，所有打开缓冲区单独保留（最多 16 个），因此最多返回 26 条有效关联记录。关闭、打开和软件重新载入时，服务端按最近使用时间维护并持久化上限；历史淘汰只移除列表记录，不删除源文件或其备份版本。重开软件后未恢复的缓冲区都属于历史，同样只保留最近 10 个。

## 标签关闭与 Console 交互

`runs.close {"id":123}` 关闭已结束标签；活动实例拒绝无确认关闭。确认后调用 `runs.close {"id":123,"stop":true}`，使用原优雅停止/超时强制停止流程，退出后 `tab_open:false`。取消确认不调用该方法。`runs.open {"id":123}` 重新打开历史标签，停止并关闭的请求完成前不允许重新打开。

左栏显示全部活动实例和最近8条已结束记录（历史图标），关闭不删除原日志。历史属于本次软件会话，重开软件后仍保留的日志可经 `logs.list/read` 查询。

Console 各视图自动换行，保留最多3000行/2MiB；可搜索纯文本。支持标准 ANSI SGR 前景色、256色/真彩色、强调、斜体及下划线；不执行终端控制序列或日志中的命令。链接与路径使用区别颜色和下划线，悬停提示 Control+点击打开。支持HTTP(S)、绝对路径、~/、相对路径及编译器行列号后缀；路径不存在会显示真实错误。

`logs.open_target {"run_id":123,"target":"./src/main.rs:20"}` 使用该运行目录解析文件或目录，并用系统默认文本编辑器/文件管理器打开；HTTP(S)使用默认浏览器。此方法会打开外部程序，AI必须在用户明确要求打开时调用；查看日志本身不需要调用。

### 原标签内重启

`runs.restart` 与界面重启共用服务路径：原 Console 的 `console_id` 保持不变，新进程获得唯一 `run_id`。旧记录 `superseded:true,tab_open:false`，用于诊断，不再列入会话导航。`runs.start` 表示显式新启动，与原标签重启区分。实例详情包含 `console_id`、`superseded`；客户端应以 `console_id` 保持窗口/标签身份，以 `run_id` 追踪进程事件。重启仍执行原优雅/强制关闭流程，校验失败且原进程尚未关闭时恢复原 Console 并显示失败原因。

### 项目信息状态

`runs.list/get` 的 `state_display` 与项目信息容器一致，例如 `运行中：8082`，多端口 `运行中：3000, 3001`。仅采用实际探测的监听端口；未获取时为 `运行中：--`，停止后显示停止状态，不沿用旧端口。原始数据仍见 `state` 与 `listening_ports`。

### 批量标签操作

`runs.bulk_plan {"action":"stop"}` 只选择当前打开的活动标签（含启动中），`action:"start"` 只选择未运行的打开标签，`action:"restart"` 只选择运行中的打开标签。三种动作相互独立，不读取关闭的历史或恢复旧批次，也不扫描项目下所有脚本。重复启动项在原逻辑 Console 中合并。未启用/删除项、参数不明的外部实例以及已知端口/共享资源冲突会阻止启动或重启；全部启动不会停止已经运行的其他标签。

计划返回 `action`、`targets`、`blocked` 与配置/源文件指纹 `revision`。向用户显示动作和对象并获得确认后，调用 `runs.bulk_apply {"confirmed":true,"plan":<原样计划>}`。未确认或过期计划不产生进程副作用。`targets[].run_id` 是十进制字符串，避免 JavaScript 精度损失；不要重建或修改计划。返回 `{action,run_ids,errors}`；启动/重启仍为异步，应跟踪新的 run_id。同步部分失败列在 errors，异步失败查看对应 Console。

所有打开的标签都保留，已停止标签不会因历史上限被移除。左侧启动会话只展示最多8条结束历史；关闭历史记录另有8条缓存上限，磁盘日志在保留上限内可查。原 `runs.stop_all` 是程序化的全实例停止；工具栏三种动作使用上述标签计划协议。

### 启动项修改信息与状态列

` scripts.update {"id":12,"patch":{"display_name":"Java-Starter","description":"本地调试后端"}} ` 同时原子保存展示名和简介；校验失败两项均不变。纯 `display_name/description` 更新在运行时也可调用，不改启动清单、argv 或 PID。`display_name:null` 恢复来源默认名。简介作为 `description_override` 展示覆盖值持久化，刷新项目仍保留覆盖；该字段为后端维护，客户端只编辑 description。

` scripts.status {"id":12} ` 返回 `{id,label}`；label 仅为 `运行中` 或 `--`。启动中、停止中、已结束以及不能确认具体参数的外部实例显示 `--`，与项目列表的状态列一致。
