# DevStart 托管启动清单

启动定义统一保存于 DevStart 用户数据目录的 `projects/<project_id>/.dev-start.json`。项目根目录中的同名文件不参与导入、刷新或启动，软件配置管理不会改动业务仓库或 Git 设置。项目 ID 在重命名后保持稳定。软件安装包、源码仓库不保存用户运行配置。

首次添加项目只读取原有 package.json、Shell/Python 启动入口；Java和其他复杂入口通过 API 显式登记。自动发现模式继续扫描原项目入口；通过 API 明确增删改定义后切换为显式清单模式，删除项不会在刷新时重生。清单缺失或损坏时明确报错，不回退到项目目录。详细事务与一次性迁移流程见 [配置存储设计](managed-config-storage.md)。

## 最小例子

```json
{
  "version": 1,
  "launches": [
    {
      "name": "前端开发",
      "file": "package.json",
      "entry": {"Package": {"manager": "npm", "name": "dev"}},
      "description": "运行项目自身 dev 脚本"
    },
    {
      "name": "本地脚本",
      "file": "scripts/dev.sh",
      "entry": {"File": {"interpreter": "/bin/bash"}},
      "description": "运行原脚本",
      "args": ["--port", "8000"],
      "expected_port": 8000
    }
  ]
}
```

启动项 name 唯一，file 指向真实原文件，cwd 默认项目根目录。可选 args 为字面参数数组，environment 为环境变量键值，resource_key 为共享资源键。路径相对项目根目录解析。清单最多2MiB，错误明确报出，不回退到IDE配置。

## Java
Java 必须显式声明 JDK、主类、运行依赖和参数，不能由类名猜测启动方式。软件数据目录中的托管清单使用项目实际目录解析相对路径，file 指向对应原 Java 源文件；Java entry 的字段为：

- program：Java 可执行文件。
- main_class：完整类名，例如 com.example.Main。
- vm_args：独立的JVM参数数组；不允许替换主类/classpath。
- class_paths：已有 classes 目录或依赖路径数组。
- classpath_file：可选的已有classpath文本文件，按本机路径分隔符解析。
- enabled：明确是否启用；部署、生成器仍为false。

不执行 Maven、构建生命周期、IDE task；没有依赖或编译主类时拒绝运行。清单原定义快照在启动前复核，变更后先点项目菜单“重新读取启动脚本”。configuration、snapshot、manifest 是后端解析时生成的运行元数据，不需要在项目清单中填写。

托管清单包含全部登记启动定义，参数预设也写入同一清单并引用原项目入口；刷新保留预设及稳定 ID。名称与参数、权限均不依赖外部IDE。

## 自动化编辑

使用本地 API 的 scripts.create/update/delete；不要把清单放进业务项目。后端校验执行定义并在同一个事务中保存托管清单及目录索引，失败不提交内存状态。enabled 默认 true；display_command 仅作展示。Java Entry.enabled 独立生效。显示名、简介、收藏与分组仍通过同一套 API 管理。详见 [API 文档](api.md)。

编辑器的“保存”与“回退源文件”会按用户明确操作修改原代码；启动命令自身产生的构建产物、业务日志仍由原程序决定，这些行为与软件配置存储分开。
