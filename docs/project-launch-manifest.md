# 项目启动清单

Dev Start 从项目根目录 `.dev-start.json` 读取明确的运行定义，不读取 `.vscode`、IDE运行缓存或其它软件设置。清单只是原程序的入口引用，不复制启动脚本或业务代码。没有清单时按 dev/start/serve/watch 命名协议读取项目本身的 package.json 启动入口和 Shell/Python 启动文件（排除构建、部署、上传等任务）；导入、刷新均不执行。

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
Java 必须显式声明 JDK、主类、运行依赖和参数，不能由类名猜测启动方式。项目根目录使用 `.dev-start.json`，file 指向对应原 Java 源文件；Java entry 的字段为：

- program：Java 可执行文件。
- main_class：完整类名，例如 com.example.Main。
- vm_args：独立的JVM参数数组；不允许替换主类/classpath。
- class_paths：已有 classes 目录或依赖路径数组。
- classpath_file：可选的已有classpath文本文件，按本机路径分隔符解析。
- enabled：明确是否启用；部署、生成器仍为false。

不执行 Maven、构建生命周期、IDE task；没有依赖或编译主类时拒绝运行。清单原定义快照在启动前复核，变更后先点项目菜单“重新读取启动脚本”。configuration、snapshot、manifest 是后端解析时生成的运行元数据，不需要在项目清单中填写。

项目级清单是基础定义，应用中另存的参数预设仍保留在 catalog 中并引用同一项目入口；刷新不丢失这些预设。名称与参数、权限均不依赖外部IDE。

## 自动化编辑

本地API的scripts.create/update/delete会直接持久化此清单，刷新不丢失修改。所有入口还支持enabled（默认true）和display_command（可选展示摘要，不改变执行）。Java的Entry.enabled仍独立生效。项目描述、项目分组和收藏在应用catalog中保存。详见[API文档](api.md)。
