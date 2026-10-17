# demo project 演示工作区

所有截图均由当前原生应用在独立 HOME 和数据目录中实际渲染。使用虚拟分组 `demo project`，包含 Java API、Admin、Website、Mobile H5 四个项目，各有“正式生产库”和“测试库”两个演示启动项。路径使用 `~/Projects/demo project/`，没有真实项目、提交、数据库或账户信息。

## 截图

- `demo-empty.png`：空项目页，居中的自动扫描提示与“开始”。
- `demo-scan.png`：静态扫描四个项目、八个入口及配置端口，用户可按项目或入口勾选。
- `demo-projects.png`：导入后的分组、项目、端口与启动控制。
- `demo-git.png`：虚拟仓库的分支、远程、工作区、终端及四条虚拟提交历史，作者为 Demo Developer。
- `demo-console.png`：Website 模拟服务的标签日志及 ANSI 颜色。
- `demo-tiles.png`：Admin、Website、Mobile H5 三个回环模拟服务的平铺日志。

Java API 的显示入口来自演示 Maven 声明与 Shell 脚本，wrapper 在演示中仅运行本机 Python 模拟服务。三个前端使用 npm 启动不依赖第三方包的 Node HTTP 模拟程序，输出合成健康检查日志；不访问外部远程、数据库或部署环境。“生产库/测试库”仅是启动项显示名。

截图中的远程 `https://github.com/example/demo-project.git` 为占位地址；没有进行 fetch、push 或网络仓库操作。Git 历史与工作区内容均在临时目录创建。

公开源码和安装包不包含演示工作空间、日志、历史、备份或项目配置。只有经过审核、无文本/EXIF 元数据的截图用于说明界面。扫描功能不会预装这些项目；用户首次打开仍只有空默认分组。
