# Flutter 与前端启动项识别

## 分层协议

`flutter_scan.rs` 仅读项目标志与配置；`flutter_launch.rs` 定义可序列化的启动协议并负责启动时设备探测与模拟器准备。扫描不运行 SDK、安装依赖、启动设备或修改项目。Flutter 平台子工程不再作为独立 Java 项目重复导入。技术栈识别和新增入口均由后端执行。

Flutter 应用须有 `pubspec.yaml`、Flutter SDK 依赖（或 Flutter `.metadata`）和 `lib/main.dart`。按实际已有的 `ios`、`android`、`ohos`、`web`、`macos`、`windows`、`linux` 目录提供入口：移动平台各有模拟器与真机调试，桌面/Web 有开发调试。

SDK 优先使用 `.vscode/settings.json` 的 `dart.flutterSdkPath`，其次项目 FVM SDK，最后登录 shell 的 `flutter`。已声明 FVM 但 SDK 未安装时直接报错，不悄悄换到系统版本。鸿蒙可设置 `FLUTTER_OHOS_ROOT` 指向适配版 SDK，启动时覆盖通用 SDK；先检查 `flutter build --help` 确认支持 `hap`。

## 设备与模拟器

点击启动后使用 `flutter devices --machine`，严格按平台、模拟器/真机类别与可选设备 ID 过滤。没有连接真机时失败，不代替用户启动模拟器。多个匹配设备时失败并要求指定 ID，不静默选择。

模拟器未运行时：

- iOS：`xcrun simctl list devices available --json` → 启动指定或排序后的首个 iOS 实例 → 打开 Simulator → 等待 Flutter 发现设备。
- Android：从 `ANDROID_HOME` / `ANDROID_SDK_ROOT` 或当前系统标准 SDK 目录找 emulator，`-list-avds` 列表中选指定或排序首个实例，`-avd` 启动。
- 鸿蒙：从 `DEVECO_EMULATOR`、`DEVECO_STUDIO_HOME`、标准 macOS DevEco 安装位置或 PATH 寻找 Emulator，使用 `-list -details` JSON 和 `-start <name>`。不会自动同意许可或下载镜像。

设备准备最多等待 120 秒；单次设备命令最多 20 秒、输出最多 1 MiB。随后 `flutter run --debug -d <id>`，额外运行参数原样传递。日志进入现有 Console；模拟器服务独立于调试进程，退出调试不关闭共享模拟器。模拟器工具的输出保留在软件数据目录 `emulator-logs/`，复用日志大小与行数上限。启动失败报告真实错误及日志位置。

运行时保留用户已有依赖源；未指定时使用 Flutter 官方中国镜像 `https://pub.flutter-io.cn`、`https://storage.flutter-io.cn`。无自动外网回退，镜像错误会明确报告。

iOS 实际调试仍要求 macOS/Xcode、可用模拟器或已授权真机；应用构建依赖同步和签名由 Flutter/Xcode 正常执行，DevStart 不伪造环境或跳过构建错误。鸿蒙要求适配 SDK、DevEco 工具及可用设备。

## 声明格式

```json
{"Flutter":{"launch":{"program":"flutter","platform":"Android","target":"Simulator","device":null,"emulator":null,"entrypoint":null}}}
```

`platform`：`Ios` / `Android` / `Ohos` / `Web` / `Macos` / `Windows` / `Linux`。`target`：`Simulator` / `Device` / `Desktop`。`device` 为已连接设备 ID；`emulator` 仅用于启动未运行的 Android/鸿蒙实例时指定名称；`entrypoint` 可指定如 `lib/main_dev.dart`。额外 `args` 可提供 `--flavor`、`--dart-define` 等 SDK 参数。可通过软件管理的启动配置调整，不写入项目仓库。

## 主流前端

识别 UniApp、Vue 3、Nuxt、Next.js、React、React Native、Expo、Angular、SvelteKit、Astro。仍以 `package.json` 中存在的启动脚本为执行来源，保留 npm/pnpm/yarn 和原有端口识别；除常规 dev/start/serve/watch 名称，还识别明确的开发 CLI 命令（如名为 local 的 `vite --port 4567`）。不虚构不存在的脚本，不把 build/deploy/publish 当启动任务。

UniApp 的 H5、小程序与鸿蒙相关已有开发脚本会被识别；App 真机运行仍取决于项目脚本与 HBuilderX 工具链，不能把编译资源等同于已运行到真机。

## 验证

- Rust：Flutter 平台发现、去除内部 Android 重复项目、导入及 JSON 往返、设备类别过滤和多设备拒绝、Vue 3 / UniApp 自定义脚本与端口。
- `python3 scripts/test-flutter-launch.py`：用隔离 SDK 替身验证 Android/iOS/鸿蒙的模拟器准备后调试、真机调试、参数原样传递、缺失设备和多设备错误，不启动真实模拟器或构建用户项目。

## 依据

- [Flutter CLI](https://docs.flutter.dev/reference/flutter-cli)
- [Flutter Android 环境与设备](https://docs.flutter.dev/platform-integration/android/setup)
- [OpenHarmony Flutter](https://gitee.com/openharmony-sig/flutter_flutter)
- [DevEco 模拟器命令](https://developer.huawei.com/consumer/cn/doc/harmonyos-guides/ide-emulator-command-line)，并核对本机官方工具 `Emulator -help` / `-list -details`。
- [UniApp CLI](https://uniapp.dcloud.net.cn/quickstart-cli)

验证记录（2026-10-22）：180 项 Rust 测试、8 项隔离 CLI/设备流程测试、Clippy 和格式检查通过；已打包并验证扫描界面。未对用户项目执行真实设备构建、签名或安装。
