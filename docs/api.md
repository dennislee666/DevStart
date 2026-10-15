# Dev Start 本地 API v1

面向 Codex、Cursor、其它编程 AI 和本机软件。所有接口由正在运行的 Dev Start 实例执行，与界面共用配置、进程状态及日志。无需安装 SDK 或新增依赖。

## 1. 连接方式

正式可执行文件：

```bash
DEV_START='/path/to/DevStart/dist/DevStart.app/Contents/MacOS/dev-start'
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
"$DEV_START" --api groups.create '{"name":"示例"}'
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

所有params必须是对象，未知字段报错。返回值包装在JSON-RPC的result内；错误在error内。`id`参数指分组、项目、启动项或运行实例的对应id，不能混用。返回的id由服务分配，调用者不指定。

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
| `scripts.get` | `{"id":12}` | 启动项对象 |
| `scripts.create` | `{"project_id":10,"spec":{…}}` | 新启动项对象；spec见下文 |
| `scripts.update` | `{"id":12,"patch":{…}}` | 修改后的启动项对象；字段见下文 |
| `scripts.delete` | `{"id":12}` | 从项目清单删除该启动项，保留原脚本文件 |
| `runs.list` | `{}` | 本次服务进程的全部运行记录（含已停止、外部识别实例） |
| `runs.get` | `{"id":123}` | 运行实例详情 |
| `runs.start` | `{"script_id":12}` | `{run_id,state:"starting",asynchronous:true}` |
| `runs.restart` | `{"script_id":12}` | 同checked-start流程；无历史实例时也可启动 |
| `runs.stop` | `{"id":123}` | 请求优雅停止，已停止时幂等；异步确认见runs.get |
| `runs.stop_all` | `{}` | 请求停止全部受管/已核实运行实例 |
| `logs.search` | `{"run_id":123,"keyword":"error","case_sensitive":false,"offset":0,"limit":100}` | 搜索当前Console缓存，分页返回匹配行与高亮范围 |
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

运行中的目标启动项禁止编辑/删除，运行中的项目禁止移除登记；先runs.stop并轮询active:false。新增其它启动项、修改项目元数据、移动分组不需要停止。应用刷新后保留API编辑与id；重新命名仍保留id。清单校验失败回滚原字节；并发外部编辑检测到时拒绝覆盖，先refresh再重试。跨进程崩溃不承诺多文件事务，恢复后以项目清单为权威并执行refresh。

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
{"run_id":123,"offset":0,"next_offset":6,"size":6,"text":"ready\n","bytes":[114,101,97,100,121,10],"active":true,"reset":false,"eof":true}
```

offset/next_offset/size均为**原始文件字节**，不能按字符串长度计算。bytes为无损原始字节数组；text为方便阅读的UTF-8有损解码，跨页中文可能截在字符中间，要求精确还原时拼接bytes或使用增量解码器。文件被截断且请求offset超过当前size时reset:true、offset重置为0。eof只是当前已读到文件末尾，active:true时仍可能继续输出。

跟踪流程：每次使用next_offset发起下一次logs.read，eof且active时等待300ms再查，active:false且eof时结束。`--logs RUN_ID [OFFSET]`已封装此流程，直接输出原始字节。磁盘读取不占Console缓存，不全量读日志；UI缓存仍为每实例3000行/2MiB和全局32MiB。

重启应用后先logs.list查持久化run_id，再logs.read。日志页只返回实际文件内容；外部识别服务的历史stdout无法被接管，因此只保留监测和操作记录及重启后的新输出。

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

Node最简单的方式是调用同一个CLI，不经过shell：

```javascript
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
const exec = promisify(execFile);
const binary = '/path/to/DevStart/dist/DevStart.app/Contents/MacOS/dev-start';
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
      "command": "/path/to/DevStart/dist/DevStart.app/Contents/MacOS/dev-start",
      "args": ["--mcp"]
    }
  }
}
```

Codex 使用 `~/.codex/config.toml` 中的配置：

```toml
[mcp_servers.dev_start]
command = "/path/to/DevStart/dist/DevStart.app/Contents/MacOS/dev-start"
args = ["--mcp"]
```

也可以自行运行：

```bash
codex mcp add dev_start -- "/path/to/DevStart/dist/DevStart.app/Contents/MacOS/dev-start" --mcp
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

CLI返回JSON-RPC结果并以退出码表示成功/失败；MCP工具执行失败通过isError:true返回原因。API不提供任意文件写入或删除原项目目录的操作；原脚本代码修改继续用编辑器。支持macOS与Linux的Unix socket，不提供Windows命名管道。没有新增外部下载源；当前功能使用Rust标准库和既有serde依赖。

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

logs.search复用GUI搜索器，只查询当前服务中该run保留的Console缓存（每实例最多3000行/2MiB），不加载完整磁盘文件，也不修改GUI的查询条件。历史磁盘记录用logs.read按字节流读取。keyword为字面子串，最多256字符；默认Unicode小写匹配，case_sensitive:true区分大小写，不做正则或模糊推断。空关键词返回全部缓存行，matched_lines为0。

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

配置解析顺序：启动清单expected_port；字面--port/--server.port及JVM的-Dserver.port/-Djboot.server.port；package.json中启动命令及额外参数；单条静态Shell启动命令；Vite server.port/preview.port与Vue devServer.port的数字字面量。Next CLI支持-p，Uni的-p是平台选项，不作端口解析；mp-weixin等编译监视入口不会套用H5的Vite端口。优先明确声明的expected_port，不将推导值写回expected_port或改变启动约束。

不执行Shell、JavaScript配置、构建、上传；不从代理地址、依赖服务URL或常见默认端口猜测。动态表达式、环境变量插值、多个含糊配置、条件分支/对象展开等不能静态确定时返回unknown，必要时在项目清单明确expected_port。配置结果在加载、重新读取项目启动项或API修改后刷新并缓存，UI每帧不读磁盘。外部修改配置后可调用projects.refresh。

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

备份位于数据目录 `source-versions/<script_id>/`，目录 0700、内容 0600；首次打开保存原始基线，保存、每次启动与回退保留版本，修改前外部内容也会保留。每启动项仅保留最近 10 个版本，包含回退本身，版本与源文件路径绑定；旧版本超过上限后自动删除。无更改保存不制造重复版本，启动则每次记录。回退不自动启动任何命令。

Java `.java` 若曾经由内置编辑器保存/回退，后续启动检查会用该启动项 JDK 同目录的 `javac`，仅编译对应源文件，禁用注解处理，不执行 Maven/Gradle、上传、部署或生成器主类。编译在后台、原服务停止之前完成，60 秒超时或错误会取消启动；输出进入对应 Console。编译成功后才优雅切换服务并应用 class，确保 Java 源文件回退也在下次启动生效。没有内置编辑保存记录的 Java 项目保持原有已编译启动行为。Python、Rust、前端等仍使用项目自己的运行命令和编译流程，DevStart 不代写业务构建脚本。

### 项目与分组重命名

GUI 的重命名入口复用 `projects.update {"id":项目ID,"patch":{"name":"example-java"}}` 和 `groups.update {"id":分组ID,"name":"示例开发组"}`。仅修改展示元数据，可在启动项运行或文件编辑期间执行，不修改目录、启动项、文件、命令或参数，不触发重启。项目名去除首尾空格；名称需非空且无换行/控制字符，分组名不能重复。运行标题和编辑器关联名称即时同步；默认分组的名字也可修改，ID 和默认分组语义保持不变。

### 启动时间和运行时长

`runs.list` / `runs.get` 增加 `started_at`（本地 `YYYY-MM-DD HH:mm:ss`，未真正启动或无法确定则 null）、`started_at_unix_ms`（Unix 毫秒或 null）、`elapsed_display`、`timing_display`。保留 `elapsed_seconds` 数值字段。正常显示如 `2026-10-15 08:30:00 - 1天2小时3分钟4秒`；零天/小时/分钟省略，秒始终出现，例如 1279 秒显示 `21分钟19秒`，3600 秒显示 `1小时0秒`。结束后时长固定。

本工具启动的进程以成功创建进程时为准，预检查时间不计入运行时长。外部进程从系统 `ps lstart` 获取实际启动时间，初始化其已运行时长，再用单调计时递增；无法确定时明确显示“启动时间未知 - 已监测 …”，不会把发现时间当作启动时间。编辑器版本日期与运行时间使用相同本地时间格式。

`editor.list` 的已关闭历史上限为 10 个，所有打开缓冲区单独保留（最多 16 个），因此最多返回 26 条有效关联记录。关闭、打开和软件重新载入时，服务端按最近使用时间维护并持久化上限；历史淘汰只移除列表记录，不删除源文件或其备份版本。重开软件后未恢复的缓冲区都属于历史，同样只保留最近 10 个。
