# demo project 演示工作区

所有截图均由当前原生应用在独立 HOME 和数据目录中实际渲染。开源版首次启动会在本地工作区生成虚拟分组 `demo project`，包含 Java API、Admin、Website、Mobile H5 四个项目，各有本地演示与测试入口。项目文件位于应用数据目录下，不写入真实项目仓库。没有真实项目、提交、数据库或账户信息。

## 截图

- `demo-empty.png`：空项目页，居中的自动扫描提示与“开始”。
- `demo-scan.png`：静态扫描四个项目、八个入口及配置端口，用户可按项目或入口勾选。
- `demo-projects.png`：导入后的分组、项目、端口与启动控制。
- `demo-git.png`：虚拟仓库的分支、远程、工作区、终端及四条虚拟提交历史，作者为 Demo Developer。
- `demo-console.png`：Website 模拟服务的标签日志及 ANSI 颜色。
- `demo-tiles.png`：Admin、Website、Mobile H5 三个回环模拟服务的平铺日志。

八个入口都使用随开源版提供的 Python 标准库 HTTP 模拟脚本。脚本只绑定 `127.0.0.1`，返回合成健康检查结果；不安装依赖，不访问外部远程、数据库或部署环境。入口不会自动启动，用户点击后才运行。演示与测试名称仅用于区分虚拟入口。

截图中的远程 `https://github.com/example/demo-project.git` 为占位地址；没有进行 fetch、push 或网络仓库操作。Git 历史与工作区内容均在临时目录创建。

公开源码和安装包只包含演示 profile 与无依赖的回环模拟脚本；运行时项目配置、日志、历史和备份生成在用户本地工作区。已有 catalog 不会被自动写入或覆盖。只有经过审核、无文本/EXIF 元数据的截图用于说明界面。空状态截图来自隔离工作区中的空目录场景。
