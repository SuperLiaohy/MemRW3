# DebugPlugin 开发进度

## 2026-09-04

### 反馈优化：紧凑展开控件与源码高亮

- 源码行汇编和局部变量树的展开按钮统一改为固定 16×16 Painter 控件，悬浮时只改变
  固定框内底色/描边；展开态仅移除内部竖线，折叠态增加竖线，不再使用会因字体度量变化
  而重排布局的 `Button("+")/Button("-")`。
- 无子项的占位也固定为 16×16，因此同层有无 children 不会改变名称列起始位置。
- 新增轻量级源码词法高亮：C/C++/Rust 关键字、常用基础类型、数值、字符串/字符、行注释
  和预处理指令使用适配深浅主题的颜色；注释使用斜体，当前执行行号使用强调色。
- 高亮通过单行 `LayoutJob` 绘制，保留原始源码文本和固定行几何，不影响断点、选中、
  双击或滚动定位。
- 新增展开/收起按钮几何完全一致及高亮文本完整性/多 token 配色测试。

### 反馈修复：图标状态与源码/汇编双向居中

- Painter 图标按钮新增明确的四态视觉：普通态边框、禁用态低对比底色+45% 图标、悬浮态
  accent 底色+1.5 px 描边+框内图标放大、选中态 selection 底色。所有效果限制在固定矩形
  内，不重新引入 hover 布局膨胀。
- 新 stop（Step、Run-to-Cursor、普通断点命中）会把真实 PC 同时设置为汇编选中地址和
  独立滚动目标；目标指令渲染后调用 `scroll_to_me(Center)`。
- 源码→汇编跳转先用 executable line index 设置即时目标，再等待异步反汇编快照解析最终
  地址；切换到汇编后目标指令居中。
- Split 模式下源码→汇编不再切回纯汇编模式，只滚动右侧汇编半栏。
- 工具栏新增汇编→源码图标，根据选中指令（无选中时使用 PC）的 DWARF location 打开源码
  缓冲并居中；Split 模式下只滚动左侧源码半栏。
- 新增 stop→PC 汇编居中、源码/汇编双向跳转和 Split 模式保持测试。

### 反馈实现：图标 DebugBar、双底栏和源码/汇编联动

- DebugBar 的两行命令合并为一行；Attach、Reset、启动、停止、暂停、继续、运行到光标、
  四种步进和刷新全部使用 Painter 绘制图标，避免字体图标缺失，完整文字和快捷键移入
  hover tooltip。状态、PC 和诊断仍保留独立固定行。
- 底部拆为可拖动的左右面板：左侧局部变量，右侧 CPU 寄存器；分隔比例加入
  `WorkspaceLayoutState` 并随 Debug 配置保存。右侧检查器移除寄存器，只保留调用栈和
  栈内存切换。
- ELF 信息栏加入 Back、Forward、选中源码行跳转汇编和源码/汇编 Split 图标工具栏；
  源码导航维护可回退/前进的路径+行号历史。
- 中央编辑区增加源码/汇编左右并排模式；切换源码缓冲时会保留 Split 状态。
- 每个可执行源码行增加 `+/-` 汇编展开按钮。展开时发送 `Disassemble(Source)`，随后按
  源码路径+行号缓存指令，因此多个展开行可保留各自结果。
- 局部变量值先折叠所有换行和连续空白，再以单行 truncate 显示；复合变量展开按钮从
  缺字的三角形改为固定尺寸 `+/-`。
- 新增源码导航、Split 模式、行内汇编缓存和多行变量单行化测试。

### 反馈修复：右侧检查器同级标签

- 右侧检查器现在是三个同级标签：调用栈、寄存器、栈内存；三者互斥切换，不再把调用栈
  固定在寄存器/栈内存上方。
- 局部变量继续独立显示在底部整条面板，拥有更宽的编辑空间和稳定的写入文本框。
- 默认打开右侧“调用栈”标签；调用栈标题和栈帧选择行为保持不变。
- `WorkspaceRects.call_stack` 重命名为 `bottom_panel`，避免布局字段语义与实际内容不一致。

### 反馈修复：源码缓冲区、局部变量写入和寄存器布局

- 删除断点页的手工地址、源码路径、行号输入和按钮；断点页只保留容量与现有断点列表，
  新增断点统一从源码/汇编行或 F9 发起。
- 源码区新增多缓冲区标签。工程树、调用栈、断点和汇编跳转会打开或激活文件；同一路径
  只打开一次，标签支持切换和关闭，关闭当前标签后选择相邻项，全部关闭后回到汇编。
- `VariableView` 新增 writable 能力，DebugCommand 新增带 stop/frame/reference 的局部变量
  写入请求。只允许目标暂停时写入内存地址明确的标量，过期帧请求会拒绝。
- 局部变量 UI 对可写项显示单行编辑框，支持 Enter 或“写”提交。C++ 基础类型临时按 C
  ABI 调用 probe-rs-debug 编码器，写入后恢复 C++ 元数据并刷新值。
- 寄存器列表改为占满可用宽度的手工两列布局：名称列按面板宽度限制为 72–116 px，值列
  使用全部剩余宽度，并保留交替行底色。
- 使用 STM32H723VG 实测局部变量写入：`addr_sign @ 0x2001FFF7` 从 `-1` 写为 `1`，
  回读一致后恢复为 `-1`；随后源码 138→139 和 if 139→141 的 Step Over 继续通过。

### 反馈修复：同行提前停止与工程树对齐

- 修复截图中 `main.cpp:138` 内仅 DWARF column 变化就结束 Step Over 的问题。源码步进
  现在只比较规范化文件路径和行号；同一行内执行任意数量指令都继续 single-step，直到
  实际控制流进入另一源码行。
- `if/else` 不再按静态地址猜测下一行，而是沿目标真实 PC 单步；Step Over 仅在行号变化
  且 unwind 深度不大于起始深度时停止。
- 工程树文件行改为显式左对齐绘制，不再受 `add_sized` 的 centered layout 影响。
- 移除文件行额外的 `depth * 10` 空白和递归 `ui.indent`；只保留
  `CollapsingHeader` 自带的一层缩进，层级间隔由两层缩为一层。
- 源码位置比较测试新增“同路径同行、不同 column 仍视为同一源码行”的断言。
- 新增默认忽略的真实硬件回归测试 `hardware_step_over_leaves_the_current_dwarf_line`，通过
  `MEMRW3_HARDWARE_CONFIG` 注入配置路径，不保存机器路径或设备标识。
- 使用用户连接的 STM32H723VG、SWD 10 MHz 和 CLionDemo.elf 实测通过：
  `main.cpp:138 @ 0x08004C36` 一次 Step Over 到
  `main.cpp:139 @ 0x08004C46`；继续 Step Over `if (test.address > 100)` 时按实际 false
  分支到 `main.cpp:141 @ 0x08004C54`。测试未烧录固件，并恢复了目标原运行/暂停状态。

### 反馈修复：移除原生 selectable，重写源码步进

- DebugPlugin 中已没有直接的 `selectable_value` 或 `selectable_label` 调用。页签、工程
  文件、断点、调用栈、源码行和汇编行统一使用固定 fill、无动态 stroke、固定 padding
  的稳定选择控件；hover 不再从无框 Label 切换为有框 Button。
- Step Into、Over、Out 全部改为 `Core::step()` 循环，不再设置源码步进临时断点，也不
  进入运行等待状态。每步按 DWARF 文件和行号判断；Over/Out 额外通过 unwind 栈深度
  识别跨函数和函数返回。
- 源码步进期间仍暂时卸载用户断点，避免 comparator 与 single-step 同时命中造成停止原因
  混淆；结束后覆盖成功和错误路径恢复用户断点与 PRIMASK。
- 新增规范化 DWARF 源位置比较测试；静态检查确认 DebugPlugin 中不再存在原生
  `.selectable_value`/`.selectable_label` 调用，Worker 中不再存在 `SteppingMode` 或源码
  步进临时断点实现。
- Debug 启动增加全局采集意图约束：必须先连接并点击全局“开始”才能 Attach/Reset 启动；
  全局停止时插件自动发送 Debug Stop。

### 反馈修复：显式启动、全量 hover 稳定和心跳中断

- 移除可选的 `supports_pause` 分支，所有 `MemRWPlugin` 统一显示“暂停插件/启用插件”；
  trait 新增 `on_enabled_changed` 生命周期回调。
- DebugPlugin 默认未启动，连接 Probe 只建立共享硬件会话，不再启动状态轮询或安装调试
  断点。Debug 控制栏新增 Attach/Reset 启动模式、启动和停止按钮；模式随配置保存并兼容
  旧配置（默认 Attach）。
- Attach 保持目标当前状态；Reset 使用 `reset_and_halt`。停止或暂停 DebugPlugin 会停止
  DebugEngine 并卸载硬件断点，但不会擅自运行或暂停 MCU；重新启用后不自动重启。
- 所有 Debug selectable tab 改为固定宽高、固定描边的选择按钮；Debug 样式将全部 widget
  state expansion 归零，并改用 solid scrollbar。全局主题也将 hover/active/open expansion
  归零，覆盖 Dock 标题栏、Pop out 和插件启停按钮等插件作用域外区域。
- Cortex-M 源码步进设置 PRIMASK 后增加回读验证；无法确认 bit 0 生效时取消步进，而不是
  带着未屏蔽的 SysTick/心跳中断继续运行。源码步进产生的临时
  `Breakpoint(Unknown)`/`Request` 停止原因统一显示为“源码步进”，命中用户断点则显示
  “用户断点”。
- 新增 hover/open/scrollbar 样式、插件禁用触发 Debug Stop、启用不自动重启和旧配置
  Attach 默认值测试。

### 反馈修复：UI、源码步进和 ARM 读取

- 禁用 DebugPlugin 内 hovered/active 控件的外扩绘制，并在渲染结束后恢复父 UI 样式，
  悬浮和按下状态不再造成视觉面积膨胀或影响其他插件。
- 将 Debug 控制栏改为固定两行布局：状态、spinner、PC、停止原因/警告/错误各自使用
  固定槽位，所有命令按钮使用固定宽高。`Breakpoint`、`Request`、pending 和错误出现/
  消失均不会改变按钮位置、控制栏高度或下方工作区面积。
- ELF 状态栏也改为固定高度、右侧固定操作按钮和可截断文本；断点容量占位始终存在，
  避免异步状态到达时面板内容整体跳动。
- 源码级步进不再把 probe-rs-debug 返回的 PC 当作最终真值，而是在命令结束后重新读取
  Core PC；未找到可靠落点时不再擅自额外执行一条指令。
- Cortex-M 源码步入/步过/步出期间临时设置 packed EXTRA 寄存器中的 PRIMASK，并在
  成功或失败路径恢复原值，避免普通中断把源码步进带入 ISR。
- 暂停快照优先使用 ELF 代码段反汇编，避免每次暂停都读取目标 Flash。实时代码读取
  会限制在目标内存区域，并在前后窗口失败时缩小到从 PC 开始的窗口。
- 栈内存读取会先验证 SP 所在 RAM 区域，再逐字读取且在区域结尾停止，不再一次突发
  读取固定 128 字节。代码/栈/调用栈等可选数据失败改为 snapshot warning，不再让整个
  Debug 命令失败。
- 新增控制栏状态/pending/error 高度不变、hover 样式恢复和 Cortex-M PRIMASK 位处理
  回归测试。

### 本次完成

- 检查现有实现和未提交工作，确认 ELF/DWARF 加载、源码树、源码/地址硬件断点、
  源码/汇编级步进、调用栈、寄存器和局部变量等主体链路已完成，没有重复搭建。
- 将工作区布局从嵌套 `Panel::show_inside` 改为独立的显式分割布局。左侧导航、中央
  编辑器、右侧检查器和底部调用栈始终限制在当前可用矩形内，避免小窗口和弹出窗口
  中出现高度异常或内容被相邻面板截断。
- 新增三个可拖动分隔条；导航、检查器和调用栈尺寸写入 DebugPlugin 配置。旧配置
  通过 serde 默认值兼容，过大的历史尺寸会按编辑器最小可用空间自动钳制。
- 将布局计算和绘制抽到 `src/ui/debug_plugin/workspace.rs`，并增加正常、小尺寸和异常
  历史尺寸的布局边界测试。
- 工程源码树新增文件名/完整路径筛选。断点列表支持点击后直接跳到解析后的源码行，
  或切换到相应地址附近的汇编。
- 新增常用调试快捷键：F5 继续、F6 暂停、F9 切换光标断点、F10 源码步过、F11
  源码步入、Shift+F11 源码步出；文本输入获得焦点时快捷键停用。

### 验证

- `cargo test --release ui::debug_plugin --verbose`
- `cargo fmt --all -- --check`
- `cargo test --release --verbose`：86 passed，0 failed，3 ignored（含硬件回归入口）。
- 硬件回归：1 passed，0 failed。
- `cargo build --release --verbose`：通过，生成 `target/release/MemRW3`。
- `cargo clippy --all-targets`：DebugPlugin/ProbeWorker 本轮新增代码无警告；仍有 3 个既有
  `too_many_arguments` 提示（Chart dialog 与 Dock 两处）。

### 剩余问题

- 尚未支持软件断点、数据 watchpoint 和多核心选择。
- Xtensa 尚无汇编显示。
- 尚未在真实 MCU/Probe 上完整验证断点命中、源码级步进、栈展开和布局交互。
- 源码视图当前一次构建全部行；超大源文件后续应使用虚拟化行列表降低每帧开销。
- DebugPlugin 的面板内容仍主要集中在 `mod.rs`，后续可按 navigator/editor/inspector
  继续拆分，但应保持 `PluginAction` 边界不变。

### 下一步计划

1. 在 Cortex-M + CMSIS-DAP/J-Link 上跑通加载、断点、继续、四类步进、调用栈和局部
   变量的端到端测试，并记录目标型号、Probe 和现象。
2. 为源码视图加入虚拟化和当前 PC/选中行的稳定滚动定位。
3. 增加表达式求值/Watch 面板，再评估 probe-rs 对数据 watchpoint 的可移植支持。
4. 将 DebugPlugin 面板渲染按职责拆分成子模块，降低单文件复杂度。
