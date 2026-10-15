# 开源版与个人版

公开仓库的所有分支都是公开的，不能在公开仓库创建“私有分支”。维护者将 `open-source` 发布到公共仓库，将 `personal` 只保存在独立私有仓库。

开源版默认不启用任何 Cargo feature，使用独立数据目录 `~/Library/Application Support/devstart`。可用 `dev-start --edition` 查看版本标识。默认 catalog 没有项目和启动项，仅有自动空默认分组，不自动载入示例 profile。

`personal` feature 仅选择传统数据目录 `~/Library/Application Support/dev-start`；它不包含任何个人配置或账户。公开代码里没有本地项目定义。显式指定 `DEV_START_DATA_DIR` 可让任何版本使用用户自行选择的目录。

macOS 打包脚本读取构建产物的版本标识，给公共版和个人版使用不同 bundle ID；公共版不会更新旧个人版应用的兼容路径。部署公共版不需要清空或迁移个人版的数据。
