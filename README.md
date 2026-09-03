# MemRW3 — 嵌入式内存读写与变量监控工具

基于 Rust + egui + probe-rs 的实时嵌入式 MCU 变量监控工具。通过 DWARF 调试信息解析 ELF 文件中的变量树，连接调试探针（CMSIS-DAP / ST-Link / J-Link）后实时采集变量数据，以时域曲线和 FFT 频谱图可视化显示，同时支持变量读写、CSV 日志、配置保存/加载。

## 特性

- **DWARF 变量树**: 解析 ELF 文件中的 DWARF 2/3/4/5 调试信息，自动构建变量树（结构体、数组、嵌套类型），支持跨编译单元类型引用
- **实时数据采集**: 独立采集线程，串行化控制握手，有界 lock-free ring buffer 数据传递，采集槽位数组跨轮复用
- **时域图表**: 多曲线叠加，自动/固定坐标轴，切换到其他插件后仍持续摄取与记录数据，图例支持显隐和属性编辑
- **FFT 频谱分析**: 自包含 Radix-2 FFT（零外部依赖），4 种窗函数（Rectangular/Hann/Hamming/Blackman），可配置取样点数（4~65536，从数据末尾取），多曲线频谱叠加，频率游标追踪
- **滚轮缩放**: 时域 + 频域均支持 X / Y / Both 三模式滚轮缩放，手动模式下锚定视图中心
- **变量读写**: Table 面板支持按 ExtendType 写入（u8~u64, i8~i64, f32, f64），带范围校验
- **树形变量表**: Table 可递归添加结构体并完整展开数组，父子勾选独立控制实际采集
- **SVD 寄存器浏览**: Table 右侧后台加载 CMSIS-SVD，展示外设、寄存器地址与位字段
- **固件烧录**: Control Bar 直接烧录并校验 ELF/AXF、HEX、BIN 或 UF2，完成后自动复位目标
- **CSV 日志**: 可选择 CSV 文件，开始采集时覆盖写入时间戳 + 所有曲线数据行
- **插件化界面**: Chart 与 Table 均实现 `MemRWPlugin` trait，Dock、变量树添加、写入、删除、Toast、配置保存/加载统一通过动态插件池分发
- **统一主题**: App 背景、Activity Bar、控制栏、Dock、BottomSheet、状态提示和 Dialog 语义色来自同一套 `theme` palette
- **配置保存/加载**: JSON 格式保存 Probe 配置、变量池、插件 payload、ELF 路径；Chart/Table 各自保存图例和表格条目
- **多探针支持**: CMSIS-DAP / ST-Link / J-Link，SWD / JTAG 协议，可调速度 100-20000 kHz
- **跨平台**: Linux / macOS / Windows

## 依赖

### 系统依赖

**Linux (Ubuntu/Debian)**:
```bash
sudo apt install build-essential cmake pkg-config libudev-dev libusb-1.0-0-dev
# 可选: 中文字体
sudo apt install fonts-noto-cjk
```

**macOS**:
```bash
brew install cmake pkg-config libusb
```

**Windows**: 无需额外系统依赖。

### Rust 工具链

Rust 1.80+ (edition 2024):

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

### Crate 依赖

| Crate | 版本 | 用途 |
|-------|------|------|
| eframe | 0.34 | GUI 框架 (egui + 平台后端) |
| egui_plot | 0.35 | 时域/频域图表绘制 |
| egui_ltreeview | 0.7 | DWARF 变量树视图 |
| egui-notify | 0.22 | Toast 通知 |
| probe-rs | 0.31 | MCU 调试探针连接与采集 |
| crossbeam-queue | 0.3 | 有界 lock-free 采集环形队列 |
| gimli | 0.31 | DWARF 调试信息解析 |
| object | 0.36 | ELF 文件解析 |
| rfd | 0.15 | 系统文件对话框 |
| serde / serde_json | 1 | 配置序列化 |
| svd-parser | 0.14 | CMSIS-SVD 解析、数组与 derivedFrom 展开 |
| anyhow | 1.0 | 错误处理 |

## 编译

```bash
# 克隆仓库
git clone <repo-url>
cd MemRW3

# Debug 编译
cargo build

# Release 编译 (推荐)
cargo build --release

# 运行
cargo run --release
```

Release 模式下生成的二进制在 `target/release/MemRW3`。

## 使用方法

### 1. 启动

```bash
cargo run --release
```

启动后窗口 1280×720，左侧是从窗口顶部贯穿到底部的 VS Code 风格 Activity Bar，Chart 与 Table 作为内置 `MemRWPlugin` 通过图标切换。控制栏和当前插件内容都位于右侧区域；插件可通过右上角 **Pop out** 弹出为原生操作系统窗口，再通过 **Pop in** 回到主界面。

### 2. 加载 ELF 文件

点击 Chart 或 Table 面板中的 **📋 打开变量树** 按钮，底部面板会出现在按钮所在窗口；插件 Pop out 后不会回到主窗口显示：
- 在 **ELF 文件** 输入框中输入固件路径，或点击 **浏览** 选择文件
- 点击 **加载** 解析 DWARF 调试信息
- 展开左侧 DWARF 变量树，选择变量后在右侧查看/编辑属性
- 点击 **追踪** 批量更新所有已添加变量的地址

### 3. 连接 MCU

- 点击控制栏 **⚙ 设置**，选择 MCU 型号、协议（SWD/JTAG）、速度
- 点击 **连接** 通过调试探针连接目标设备

### 3.1 烧录固件

- 连接目标后点击控制栏 **烧录固件**，选择 ELF/AXF、HEX、BIN 或 UF2 文件
- 确认目标芯片与文件后开始烧录；采集会自动暂停，界面显示烧录状态
- BIN 文件自动使用目标芯片的启动 Flash 基址；写入后执行校验并复位目标

### 4. 添加监控变量

在变量树中选择节点：
- 右侧显示 **Basic** 属性（只读 DWARF 原始信息）和 **Extend** 属性（可编辑）
- 在 **Add** 区域配置曲线名/颜色 → 点击 **添加到 Chart**
- 或配置显示名 → 点击 **添加到 Table**
- Table 支持直接添加结构体或数组；结构体递归显示字段，数组会物化全部元素

### 5. 开始采集

- 点击控制栏 **▶ 开始** 启动实时采集
- Chart 面板显示时域曲线，Table 面板显示最新读取值
- Table 叶节点默认勾选；取消勾选后该绑定不再请求采集，父节点可批量切换全部后代
- 每个叶节点可设置 1–60 Hz 的当前值显示刷新频率；该设置不改变底层采集率
- 可通过 **延迟** 滑块控制采集间隔（0=全速）

### 6. FFT 频谱分析

- 点击工具栏 **📊 FFT** 按钮开启频谱视图
- 视图上下分屏: 上方时域图 (55%) + 下方 FFT 频谱图 (45%)
- FFT 图表头部可配置:
  - **窗函数**: Rectangular / Hann / Hamming / Blackman
  - **取样点数**: 4 ~ 65536 (从数据末尾取)
  - **缩放模式**: X / Y / Both (滚轮缩放轴选择)
- 所有可见曲线各自计算 FFT，叠加显示，颜色与图例一致
- 鼠标悬停频谱图显示频率游标

### 7. 滚轮缩放

- 时域图和 FFT 图均支持独立滚轮缩放模式
- 工具栏 **缩放: X Y Both** 控制时域图; FFT 图表头 **缩放: X Y Both** 控制 FFT 图
- Both: 标准双轴缩放；X/Y: 单轴缩放
- 时域图缩放自动关闭 auto-scroll，双击恢复

### 8. 变量写入

- Table 左侧变量树的 **写入** 列输入数值
- 按 ExtendType 自动校验范围（如 u8: 0-255）
- 点击 **写** 按钮执行写入

### 8.1 浏览 SVD 寄存器

- 在 Table 右侧点击 **加载 SVD**，选择 `.svd` 或 `.xml` 文件
- 展开外设和寄存器查看绝对地址、位宽、访问权限、复位值与字段位范围
- 搜索框输入顶层外设名称后按 Enter 生效；搜索不会自动展开，可用 **全部折叠** 一键收起

### 9. CSV 日志

- 点击 Log 区域的 **选择文件**，选择 CSV 保存路径
- 开始采集时自动创建文件并写入 header
- 每帧数据追加写入
- 暂停采集时自动关闭文件

## 项目结构

```
src/
├── main.rs              # 入口
├── app.rs               # 主 App + 采集/连接/插件池/配置编排
├── sync.rs              # 同步原语 (双 Condvar 握手)
├── dwarf/
│   ├── mod.rs           # DWARF 模块入口
│   ├── types.rs         # TreeNode / DwarfState / ExtendConfig / ExtendType
│   └── extract.rs       # ELF + DWARF 解析
├── model/
│   ├── mod.rs           # Model 模块入口
│   ├── state.rs         # AppSession
│   ├── variable_pool.rs # VariablePool (Vec + HashMap)
│   └── ring_buffer.rs   # 有界 lock-free 环形队列
├── probe/
│   ├── mod.rs           # ProbeCell (线程安全 Session owner)
│   └── session.rs       # ProbeSession (probe-rs 连接/采集)
├── svd/
│   └── mod.rs           # CMSIS-SVD 解析与轻量寄存器树
└── ui/
    ├── mod.rs           # UI 模块入口
    ├── control_bar.rs   # 控制栏
    ├── dock.rs          # VS Code 风格左侧插件栏 + 原生 OS 窗口 pop-out/pop-in
    ├── plugin.rs        # MemRWPlugin trait + 统一 PluginAction/FrameData/配置 payload
    ├── theme.rs         # 统一 palette + egui Visuals/WidgetVisuals 配置
    ├── vari_tree.rs     # DWARF 变量树
    ├── vari_properties.rs # 属性面板
    ├── variable_tree_panel.rs # 按 viewport 路由的变量树覆盖层
    ├── chart_plugin/
    │   ├── legend.rs    # ChartLegend
    │   ├── fft.rs        # FFT 频谱计算 (自包含)
    │   ├── panel.rs     # 图表插件实现 (时域+频域)
    │   └── line_dialog.rs # 曲线属性 Dialog
    └── table_plugin/
        ├── panel.rs     # 左侧树形变量表 + 双面板布局
        ├── tree.rs      # 结构体/数组节点、勾选和持久化
        └── svd_panel.rs # 右侧 SVD 寄存器浏览器
```

## 许可证

MIT
