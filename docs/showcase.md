# Facebook 演示工作区

这是一组用于说明 DevStart 的虚构数据，与 Meta / Facebook 没有关联。截图由当前原生应用在独立 HOME、独立数据目录中实际渲染并捕获，没有使用私人项目截图。

## 使用场景 / Use case

这组截图展示一个人在多个项目间工作的日常：找到并启动 API，再切回前端 Console；需要时平铺查看多个项目的日志。相关项目与启动顺序由用户选择，演示不表示 DevStart 会自动解析项目依赖或编排服务。

These screenshots illustrate one developer working across several projects: find and start the API, then return to the frontend Console, or tile multiple Consoles to inspect their logs. The user selects related projects and their launch order; the demo does not imply automatic dependency resolution or service orchestration.

## 分组与启动项

Facebook 包含 Java API、后台管理、官网、手机 H5 四个项目；每个项目都有正式生产库和测试库两个启动项，分别展示不同命令参数和预设端口。所有路径均为虚构的 `~/Projects/facebook/` 路径。

Java 显示命令为 `java com.example.facebook.Api --profile production` 或 `test`；三个前端项目显示 `npm run dev:production` 或 `npm run dev:test`。这些是显示元数据，用于说明同一个项目的多个入口；并不包含可连接真实数据库的凭据或真实应用代码。

## 截图内容

- 项目图：完整展开四个项目和八个启动项，同时展示静态配置端口、运行状态与操作图标。
- Console 图：两个本地模拟进程同时运行，选择 Java API 的测试库标签，展示状态、端口、时间、彩色日志及可点击的本地链接。

实际模拟进程仅监听回环地址，并输出合成日志。不会访问数据库、外部服务、生产环境或执行部署；截图中的生产/测试名称不意味着连接相应数据库。Java/npm 显示命令不代表在此演示中执行了这些运行时。

- 平铺图：四个本地模拟服务同时运行，展示 Java API 测试库及三个前端正式生产库入口；英文界面、状态图例、工具栏和独立 Console。

## 空数据与隐私

公开源码没有导出演示工作空间、私人配置、日志、编辑历史或备份。应用首次打开仍只有空的默认分组。三张经过人工审阅的 PNG 只用于文档；不包含原始用户截图、个人目录、凭据或业务数据，PNG 也不带文字和 EXIF 元数据。
