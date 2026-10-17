# 自动扫描项目

项目工具栏的“自动扫描项目”或空项目页居中的“开始”打开同一居中弹层。选择本机目录后点击开始扫描。结果按项目展示建议分组、技术栈、项目目录、启动名称、实际命令、可静态识别的端口。默认勾选新入口；已添加入口弱化且不可重复选择。确认只添加勾选项，不启动任何程序。

## 架构与规则

`project_scan` 管理有界后台任务、目录遍历、不可变扫描报告和事务导入；`project_scan_detect` 提供静态启动适配器；现有 `discovery` 与 `configured_ports` 继续负责基础脚本和端口读取；UI 只渲染报告与收集选择。API 与图形界面使用相同的服务和提交事务。

- Node.js / Vue / UniApp / Next.js：现有 package.json 的 dev/start/serve/watch 系启动脚本，保留已有参数，按 pnpm/yarn/npm 锁文件选择包管理器。
- Python：现有启动命名脚本、app.py/main.py 的直接运行入口、Django manage.py、明确的 FastAPI app 或 Flask 声明，以及 pyproject.toml 的单行 project.scripts / tool.poetry.scripts 声明。Python console script 需要其项目环境已安装该入口；本工具不安装依赖。
- Java：声明 spring-boot-maven-plugin 的 Maven 项目、带 mainClass 的 exec-maven-plugin、Spring Boot Gradle / application Gradle 入口。优先使用已有 Maven/Gradle wrapper。没有声明可运行任务的普通 Java 类库只展示项目，不虚构运行命令。
- PHP：composer.json 的 dev/start/serve/watch 系入口，及 Laravel artisan serve。
- Rust：带 src/main.rs 的 Cargo 项目。
- VS Code：读取 JSONC launch.json 的 launch 配置，支持 Python/debugpy 的 program/module、Node 的 program，以及 Java 明确声明 mainClass/classPaths 的配置；保留 args、cwd、字符串环境变量和 Java VM 参数。`${workspaceFolder}` 可解析；依赖当前编辑器文件、交互输入、自动 classpath 等未解析变量的入口不生成可执行项。attach 配置不属于启动入口。

未知/动态端口显示 `--`，不根据常见端口猜测。已有命令参数和 Vite/Vue/Uni 配置沿用现有解析器；Java默认 application.properties / application.yml 中唯一明确 server.port 可识别。生产/测试 profile 的动态表达式不能静态确定时保持未知。

分组按扫描目录的一级业务子目录归类；直接位于所选目录下的项目使用扫描目录名作为建议分组。新项目名称优先使用清单声明，缺失时使用目录名。嵌套清单视为独立项目，父项目不重复收录其启动项。

## 边界与导入

扫描不启动脚本，不 import Python，不求值 JS/Gradle 配置，不执行 Maven/Composer/npm，不获取网络资源。跳过符号链接、隐藏依赖目录、node_modules、vendor、构建产物和虚拟环境；遍历最大深度 8、最多 30000 条目录记录、200 项目、2000 启动项、约 30 秒及 4 MB 启动结果。上限触发会标记 truncated 并提示缩小扫描范围。文件读取错误进入 warnings，单个清单上限 2 MB。最多保留 100 条错误提示，四份任务报告，同时只运行一个扫描任务；关闭运行中的弹层会请求取消。

选择与导入通过任务 ID 和启动项 ID 对应后端报告，调用方不能上传任意执行定义冒充扫描结果。导入前校验所选源文件 SHA256、实际路径、入口协议及去重；失败不提交 catalog。已有项目按规范路径匹配，保留原名称、分组、收藏和旧启动项，只追加新入口。新项目/分组及启动项一次提交到 DevStart 数据目录，不向业务项目写 .dev-start.json，也不改业务 Git 配置。扫描完成后项目文件仍可能被外部编辑，点击启动前仍遵循原启动校验流程。

## API

1. `scan.start {"path":"/path/to/workspaces"}` 返回 job_id。
2. `scan.get {"id":job_id}` 轮询 running/completed/failed/cancelled。完成结果包含 root、projects、launches、port、port_source、existing、warnings、truncated。
3. `scan.import {"id":job_id,"launch_ids":[报告中的启动项ID]}` 返回 added、project_ids。无 UI 二次确认，不执行启动。重复入口跳过；空选择、重复 ID、未知 ID、过期结果及源文件已变更均报错。
4. `scan.cancel {"id":job_id}` 请求取消，已完成结果仍可读取。

验证覆盖多技术栈、配置端口、JSONC IDE 参数、嵌套项目、依赖目录和符号链接、源文件变化、重复导入，以及实际服务 API 仅写应用存储。

去重比较源文件、实际调用命令、工作目录和环境变量；不同环境的入口不会合并。多个声明或已有配置发生名称冲突时，新入口自动使用唯一序号后缀，原入口名称和配置保持不变。
