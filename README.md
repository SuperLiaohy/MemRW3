# MemRW3 — 嵌入式内存读写与变量监控工具

基于 Rust + egui + probe-rs 的实时嵌入式 MCU 变量监控工具。通过 DWARF 调试信息解析 ELF 文件中的变量树，连接调试探针（CMSIS-DAP / ST-Link / J-Link）后实时采集变量数据，以时域曲线和 FFT 频谱图可视化显示，同时支持变量读写、CSV 日志、配置保存/加载。

## 特性

- **DWARF 变量树**: 解析 ELF 文件中的 DWARF 2/3/4/5 调试信息，自动构建变量树（结构体、数组、嵌套类型），支持跨编译单元类型引用
- **实时数据采集**: 单一 ProbeWorker 硬件线程串行调度，连接期间复用 Core handle，有界 lock-free ring buffer 数据传递，采集槽位数组跨轮复用
- **时域图表**: 多曲线叠加，自动/固定坐标轴，切换到其他插件后仍持续摄取与记录数据；可在当前视图点数低于自定义临界值时标注采样点，隐藏曲线不参与统计；图例按 Plot 右边界对齐
- **FFT 频谱分析**: 自包含 Radix-2 FFT（零外部依赖），4 种窗函数（Rectangular/Hann/Hamming/Blackman），可配置取样点数（4~65536，从数据末尾取），多曲线频谱叠加，频率游标追踪
- **滚轮缩放**: 时域 + 频域均支持 X / Y / Both 三模式滚轮缩放，手动模式下锚定视图中心
- **变量读写**: Table 面板支持按 ExtendType 写入（u8~u64, i8~i64, f32, f64），带范围校验
- **树形变量表**: Table 可递归添加结构体并完整展开数组，父子勾选独立控制实际采集
- **SVD 寄存器读写**: Table 右侧后台加载 CMSIS-SVD；寄存器可独立勾选、按 1–30 Hz 批量读取，并按访问权限写入
- **固件烧录**: Control Bar 直接烧录并校验 ELF/AXF、HEX、BIN 或 UF2，完成后自动复位目标
- **CSV 日志**: 可选择 CSV 文件，开始采集时覆盖写入时间戳 + 所有曲线数据行
- **插件化界面**: Chart、Table 与 Debug 均实现 `MemRWPlugin` trait，Dock、硬件请求、Toast 和配置保存/加载统一通过动态插件池分发
- **IDE 风格调试**: DebugPlugin 支持目标暂停/继续、单指令与源码级步入/步过/步出、源码和地址硬件断点、CPU 寄存器、源码/汇编联动、调用栈、原始栈内存及局部变量懒加载
- **单一 ProbeWorker**: 连接、采集、Table/SVD 读写、烧录和调试命令由唯一硬件线程串行调度；运行时高频采集，断点暂停时停止 Chart 数据流并以 1 Hz 维持 Table 最新值
- **插件独立暂停**: 每个插件标题栏可单独暂停更新和交互，当前画面和已打开的变量树保持不变，不影响其他插件
- **变量树独立窗口**: DWARF 变量树可从 Bottom Sheet 弹出为原生窗口，并可一键 Pop in 回对应插件；再次点击打开时保持当前 Pop out 状态
- **统一主题**: 默认使用浅色；App 背景、Activity Bar、控制栏、Dock、BottomSheet、状态提示和 Dialog 使用统一 palette，并可切换深色、浅色或跟随系统（含 Ubuntu/GNOME 回退检测）
- **采集状态提示**: Control Bar 的底色、描边和开始/暂停按钮随未连接、已暂停、采集中状态联动
- **断开数据保留**: 断开 Probe 只停止采集并释放连接，Chart、Table 与 SVD 保留最后一次显示值
- **物理断链检测**: 采集、空槽及暂停状态都会检测 Probe/Core 链路；连续失败后自动断开并显示 Toast
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

Rust 1.85+ (edition 2024):

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
| fontdb / ttf-parser | 0.24 / 0.25 | 系统字体发现、TTC face 选择与中文字形校验 |
| probe-rs | 0.31 | MCU 调试探针连接与采集 |
| probe-rs-debug | 0.31 | 源码断点、源码级步进、调用栈和局部变量求值 |
| capstone | 0.14 | ARM/Thumb/AArch64/RISC-V 反汇编 |
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

启动后窗口 1280×720，左侧是从窗口顶部贯穿到底部的 VS Code 风格 Activity Bar，Chart 与 Table 作为内置 `MemRWPlugin` 通过图标切换。控制栏和当前插件内容都位于右侧区域；插件可通过右上角 **Pop out** 弹出为原生操作系统窗口，再通过 **Pop in** 回到主界面。插件已 Pop out 时，点击主窗口 Activity Bar 图标会先显示确认对话框，避免误触返回。

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
- FFT 自动忽略暂停前的非连续数据段，并将采样抖动线性重采样到均匀时间网格
- 单边幅度谱按窗函数相干增益归一化，DC 与 Nyquist 频点不会错误翻倍
- 鼠标悬停频谱图显示频率游标
- 时域图与 FFT 的游标浮窗会自动翻转并裁剪在各自 Plot 范围内

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
- 寄存器默认不读取；勾选可读寄存器并设置 1–30 Hz 刷新率后，同一帧到期项会合并为一次 Probe 请求
- SVD 标题栏实时显示全部已勾选寄存器数量，不受搜索和折叠状态影响
- 展开可写寄存器可输入十六进制或十进制值；只读寄存器不显示写入控件

### 9. CSV 日志

- 点击 Log 区域的 **选择文件**，选择 CSV 保存路径
- 开始采集时自动创建文件并写入 header
- 每帧数据追加写入
- 暂停采集时自动关闭文件

### 10. Debug 调试

- 先通过任一变量树入口加载与目标固件匹配、包含 DWARF 信息的 ELF；Debug 面板也提供同一个共享入口
- 连接 Probe 不会自动启动 DebugPlugin。和其他插件一样，必须先点击全局“开始”；然后在已启用的 DebugPlugin 中选择 `Attach`（保持目标当前状态）或 `Reset`（复位并暂停），再点击调试“启动”。全局停止会同步停止 DebugEngine
- 每个 `MemRWPlugin` 都有统一的“暂停插件/启用插件”按钮。暂停 DebugPlugin 会停止调试状态轮询并卸载硬件断点，但不会擅自改变 MCU 当前运行/暂停状态；重新启用后需要手动再次启动
- Debug 面板会从 DWARF 自动生成工程源码目录树；若 ELF 记录的是构建机路径，可选择本机源码根目录进行映射
- 工程树会自动合并连续的单子目录路径，例如 `/home/liaohy/User/...` 显示为一个目录节点
- 调试工作区采用可拖动的左侧工程/断点、中间源码/汇编、右侧调用栈/栈内存和底部左右分栏布局；底部左侧为局部变量、右侧为寄存器，分隔比例可拖动并随配置保存
- 全局主题禁用控件 hover/active/open 的外扩绘制；DebugPlugin 不再直接使用 egui `selectable_value/selectable_label`，页签、工程文件、断点、调用栈、源码行和汇编行全部使用固定背景/描边的稳定选择控件。滚动区使用不会在悬浮时变宽的实体滚动条
- 工程树支持按文件名或完整路径筛选；点击断点列表项可直接定位到对应源码行或汇编地址
- 源码编辑区使用多缓冲区标签；从工程树、调用栈、断点或汇编打开的文件会保留独立标签，可切换和关闭
- ELF 信息栏同时提供后退、前进、选中源码行跳转汇编以及源码/汇编并排显示工具按钮；按钮使用程序绘制图标，不依赖字体 glyph
- 工具栏同时提供汇编→源码反向跳转；源码/汇编双向跳转都会把目标行滚动到视图中央，Split 模式下只移动对应半栏而不退出并排模式
- 连接目标后可在运行状态下添加源码行断点或指令地址断点；双击源码或汇编行可添加/移除断点
- 断点面板只显示硬件容量和已有断点；新增断点统一通过源码/汇编双击或 `F9`，不再显示手工地址、路径和行号输入框
- 源码断点支持规范化路径、构建路径后缀匹配，并会从空行/注释行向后查找最近的可执行行；断点列表显示最终解析行和硬件地址
- 命中断点或点击“暂停”后，Chart 高频流自动停止，Debug 面板刷新 PC、CPU 寄存器、汇编、调用栈、原始栈内存和当前栈帧局部变量
- 支持单指令、源码级步入、步过和步出；源码步过遇到已验证的直接调用时使用临时硬件断点跨过函数，源码步出优先运行到 unwind 得到的调用者地址，无法可靠加速时自动回退到硬件 single-step。每一步只按 DWARF 规范化文件路径和行号判断源码是否前进，不把同一行的列号变化误判为完成；Fault、watchpoint、软件断点和外部暂停会立即结束步进
- Cortex-M 源码级步进期间会临时设置并回读验证 PRIMASK，结束后恢复原值；无法确认屏蔽生效时直接取消步进，避免普通心跳/SysTick 中断把落点带到 ISR。NMI 和 Fault 仍可能改变实际停止位置
- C++ 局部变量会在 probe-rs-debug 完成位置求值后，由 MemRW3 补充基础整数、浮点、布尔、字符、枚举、指针和位域解码
- 暂停时可直接编辑具有内存地址的标量局部变量并按 Enter 或“写”提交；请求携带 stop/frame/reference 防止旧栈帧写入，C++ 基础类型按 C ABI 编码写入并回读刷新
- 寄存器视图固定名称列并让值列占满检查器剩余宽度，窄窗口中也不会只挤在左侧
- 可执行源码行提供 `+/-` 展开按钮，按需反汇编并缓存该行对应指令；多个已展开行可以同时保留
- 复合局部变量的摘要会折叠换行和连续空白，在一行中截断显示；展开按钮使用可靠的 `+/-`，不再依赖可能缺失的三角形字符
- 源码视图提供轻量级 C/C++/Rust 词法高亮，包括关键字、基础类型、数字、字符串、字符、注释和预处理指令；行号和当前执行行使用独立颜色
- 源码视图按可见行虚拟化渲染，断点、DWARF 可执行行查询、词法高亮和行内汇编只处理当前滚动区域，避免大文件滚动时遍历全部源码行
- 源码行汇编与局部变量的展开控件使用固定 16×16 Painter 图标；展开/收起只切换框内竖线，不改变按钮宽高或触发布局重排
- 汇编视图以 ELF 代码段离线反汇编为常驻内容；只有 PC 不属于 ELF 代码段时才读取目标代码内存。栈读取按目标 RAM 边界逐字进行，局部读取失败作为非致命警告处理
- 单击源码行或汇编指令可移动调试光标；目标暂停时可“运行到光标”，临时断点命中或提前暂停后会自动清理
- 常用快捷键：`F5` 继续、`F6` 暂停、`F9` 切换光标断点、`F10` 源码步过、`F11` 源码步入、`Shift+F11` 源码步出；正在编辑文本时不会触发调试快捷键
- DebugBar 在调试命令等待期间提供手动打断按钮；源码 single-step 循环逐指令检查独立中断标志，可退出因屏蔽中断而无法完成的 Delay/等待循环。全局 Reset 使用更高优先级中断标志，先终止步进再执行复位
- DebugBar 状态行显示最近一次实际采用的步进方式：硬件断点加速或 `SingleStep`；临时硬件断点不可用并回退时显示 `SingleStep`
- Step、运行到光标或断点暂停产生新 stop 时，汇编视图会自动选中真实 PC 并滚动到中央；尚未打开汇编视图时会保留滚动目标，切换后再定位
- Halt 期间 Table 左侧变量以约 1 Hz 更新且不写入 Chart/CSV/FFT 数据流；Table 写入和 SVD 读写仍通过同一 ProbeWorker 正常执行
- 点击“继续”后，如果进入断点前正在采集，Chart 高频采集会自动恢复

当前仅使用 core 0。断点为目标硬件断点，数量由 MCU 决定；优化构建中的局部变量可能显示为不可用或已优化掉。Xtensa 暂不支持汇编显示。

## 项目结构

```
src/
├── main.rs              # 入口
├── app.rs               # 主 App + 采集/连接/插件池/配置编排
├── dwarf/
│   ├── mod.rs           # DWARF 模块入口
│   ├── types.rs         # TreeNode / DwarfState / ExtendConfig / ExtendType
│   └── extract.rs       # ELF + DWARF 解析
├── model/
│   ├── debug.rs         # 调试命令、目标状态、断点/栈/局部变量/汇编快照 DTO
│   ├── mod.rs           # Model 模块入口
│   ├── register_io.rs   # SVD 寄存器批量读请求/结果 DTO
│   ├── state.rs         # AppSession
│   ├── variable_pool.rs # VariablePool (Vec + HashMap)
│   └── ring_buffer.rs   # 有界 lock-free 环形队列
├── probe/
│   ├── mod.rs
│   ├── session.rs       # ProbeSession (probe-rs 连接/采集/断点)
│   └── worker.rs        # 唯一 Session owner；运行/暂停双调度策略和调试引擎
├── svd/
│   └── mod.rs           # CMSIS-SVD 解析与轻量寄存器树
└── ui/
    ├── mod.rs           # UI 模块入口
    ├── control_bar.rs   # 控制栏
    ├── dock.rs          # VS Code 风格左侧插件栏 + 原生 OS 窗口 pop-out/pop-in
    ├── plugin.rs        # MemRWPlugin trait + 统一 PluginAction/FrameData/配置 payload
    ├── debug_plugin/    # IDE 风格调试插件
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
