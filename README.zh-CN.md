![Skyward Recovery](assets/hero.jpg)

# Skyward

[English](README.md) | 简体中文

Skyward 是使用 Rust 编写的 recovery，从零开始编写。

<p align="center">
  <img src="assets/home.png" width="30%" alt="首页">
  <img src="assets/settings-dark.png" width="30%" alt="深色模式下的设置">
  <img src="assets/time-zone.png" width="30%" alt="时区">
</p>

## 功能

### 双系统

![一台手机可以安装两个系统](slides/zh-CN/out/01-dual-boot.png)

![切换系统只改写两个分区](slides/zh-CN/out/02-dual-boot-partitions.png)

第二系统的安装包格式和启动过程见下文的[双系统的工作方式](#双系统的工作方式)。

### 解密

![用锁屏图案或密码解密 data 分区](slides/zh-CN/out/03-decrypt.png)

### 安装

![从三个来源安装四种安装包](slides/zh-CN/out/04-install.png)

刷机包的安装脚本可以使用 `7za` 和 `zstd`，支持刷入 super 镜像使用 zstd 压缩的第三方 ROM；`unzip -p` 边解压边输出镜像。安装脚本还可以使用 `avbctl` 关闭验证启动的校验，使用 `resetprop` 读取和设置属性。

分区镜像按内容识别：boot、init_boot、vendor_boot、recovery、dtbo、vbmeta 和整个 super 的镜像各自刷入对应的分区；包含 erofs、ext4 或 f2fs 文件系统的镜像可以刷入当前槽位在 super 中的任何一个逻辑分区，刷入前这个逻辑分区会调整为镜像的大小，和 fastbootd 的做法相同。内容无法识别的镜像按文件名刷入同名的分区，例如 `modem.img` 刷入 modem，这个分区必须在两个槽位各有一份，或者是 super 中的逻辑分区。sparse 格式的镜像在刷入时展开。系统更新尚未合并的时候，暂不刷入 super 和其中的逻辑分区。

### 在线更新

![系统和 Skyward 都能在线更新](slides/zh-CN/out/05-updates.png)

### 备份和恢复

![系统、应用、照片和 IMEI 都可以备份](slides/zh-CN/out/06-backup.png)

### 文件管理

![在 Skyward 中管理手机里的文件](slides/zh-CN/out/07-files.png)

### 终端

![手机上就能运行 root shell](slides/zh-CN/out/08-terminal.png)

### 连接电脑

![用数据线连接电脑，传文件和刷写分区](slides/zh-CN/out/09-computer.png)

### 清除数据

![每一项都写明会清除哪些内容](slides/zh-CN/out/10-wipe.png)

### 替换原厂 recovery

![系统发来的请求，Skyward 照常处理](slides/zh-CN/out/11-requests.png)

### 搜索

![输入拼音首字母，就能找到功能](slides/zh-CN/out/12-search.png)

### 高级

![系统出了问题，可以先在这里修复](slides/zh-CN/out/13-tools.png)

### 关于本机

![电池剩余容量和充电循环都能查看](slides/zh-CN/out/14-specs.png)

### 其他功能

- 重启到系统、recovery、fastboot、bootloader 和 EDL，切换启动槽位
- 浅色和深色模式
- 15 种语言：简体中文、繁體中文、English、Türkçe、Русский、Українська、Español、Português (Brasil)、Bahasa Indonesia、Tiếng Việt、Deutsch、Français、Italiano、Polski 和日本語。系统语言在这些语言之中时，Skyward 启动后使用系统语言，其他情况使用英文

### 没有系统时

![super 中没有系统，也能装回一个系统](slides/zh-CN/out/15-no-system.png)

## 支持的机型

| 机型 | 代号 | 测试的系统版本 |
| --- | --- | --- |
| 小米 17 Ultra | `nezha` | OS4.0.4.0.XPACNXM（Android 17） |
| 小米 13 Ultra | `ishtar` | OS3.0.308.0.WMACNXM（Android 16） |
| REDMI K60 Pro | `socrates` | OS4.0.0.24.XMKCNXM（Android 17） |

### 🚧 施工中

这些机型的镜像按照各自的原厂固件构建，还没有在真机上试过。屏幕、触摸、WLAN 和振动已经配置好。解密 data 分区所需的程序由 Skyward 在手机上从已安装系统的 init 脚本和服务描述中读出，每个机型的读取结果都用原厂固件核对过，这些程序在这些机型上的运行情况还没有验证。

| 机型 | 代号 | 构建所用的系统版本 |
| --- | --- | --- |
| 🚧 小米 13 | `fuxi` | OS3.0.309.0.WMCCNXM（Android 16） |
| 🚧 小米 13 Pro | `nuwa` | OS3.0.312.0.WMBCNXM（Android 16） |
| 🚧 小米 14 | `houji` | OS3.0.307.0.WNCCNXM（Android 16） |
| 🚧 小米 14 Pro | `shennong` | OS3.0.309.0.WNBCNXM（Android 16） |
| 🚧 小米 14 Ultra | `aurora` | OS3.0.308.0.WNACNXM（Android 16） |
| 🚧 小米 15 | `dada` | OS3.0.307.0.WOCCNXM（Android 16） |
| 🚧 小米 15 Pro | `haotian` | OS3.0.309.0.WOBCNXM（Android 16） |
| 🚧 小米 15 Ultra | `xuanyuan` | OS3.0.314.0.WOACNXM（Android 16） |
| 🚧 小米 17 | `pudding` | OS4.0.4.0.XPCCNXM（Android 17） |
| 🚧 小米 17 Pro | `pandora` | OS4.0.4.0.XBLCNXM（Android 17） |
| 🚧 小米 17 Pro Max | `popsicle` | OS4.0.4.0.XPBCNXM（Android 17） |
| 🚧 小米 17 Max | `byron` | OS3.0.310.0.WAFCNXM（Android 16） |
| 🚧 REDMI K80 | `zorn` | OS3.0.309.0.WOKCNXM（Android 16） |
| 🚧 REDMI K80 Pro | `miro` | OS3.0.305.0.WOMCNXM（Android 16） |
| 🚧 REDMI K90 | `annibale` | OS3.0.307.0.WPKCNXM（Android 16） |
| 🚧 REDMI K90 Pro Max | `myron` | OS4.0.4.0.XPMCNXM（Android 17） |
| 🚧 REDMI K90 Ultra | `warsaw` | OS3.0.309.0.WHPCNXM（Android 16） |
| 🚧 REDMI K100 Pro | `athens` | OS4.0.4.0.XPICNXM（Android 17） |
| 🚧 REDMI K100 Pro Max | `songyuan` | OS4.0.4.0.XGNCNXM（Android 17） |

### 🚧 三星

三星的 recovery 不能在这台手机上启动 Skyward，所以镜像构建在为它构建的第三方 recovery（OrangeFox 或 TWRP）之上：这份基底镜像不由三星分发，构建者自己提供，设备配置的 `[base]` 按 SHA-256 核对它。手机没有 A/B 槽位，不使用 fastboot，用 Odin 或 Heimdall 刷入。配置参考了 [yanzihan/Skyward-Galaxy](https://github.com/yanzihan/Skyward-Galaxy) 的移植，在一台运行移植版 One UI 8.5 的 SC-52B 上试过：屏幕、触摸、adb 和 U 盘可以用，解密 data 分区和恢复出厂设置还不行，WLAN 要另外加入驱动；移植说明、真机测试情况和对参考仓库的安全审计见 [docs/samsung-p3q.zh-CN.md](docs/samsung-p3q.zh-CN.md)。

| 机型 | 代号 | 构建所用的基底 |
| --- | --- | --- |
| 🚧 Galaxy S21 Ultra 5G（骁龙） | `p3q` | 原厂 Android 11 vendor 上的 OrangeFox 或 TWRP recovery |

## 安装 Skyward

Skyward 需要已解锁的 bootloader。从 [Releases](https://github.com/kirklin/skyward/releases) 下载对应机型的 zip，每个文件都标有机型和代号，例如 Xiaomi 17 Ultra (nezha)。使用 `SHA256SUMS` 核对并解压之后，在手机处于 fastboot 模式的时候，把其中的镜像刷入当前运行槽位的 recovery 分区：

```sh
unzip skyward-v0.5.1-nezha-OS4.0.4.0.XPACNXM.zip
fastboot flash recovery skyward-v0.5.1-nezha-OS4.0.4.0.XPACNXM.img
fastboot reboot recovery
```

之后连接 WLAN，就可以在“设置”的“Skyward 更新”页面更新到最新版本，也可以在 Skyward 自己的“安装”页面安装解压出来的新版镜像。打开 **自动重新刷入 Skyward** 之后，系统更新或者刷机包覆盖 recovery 分区的时候，Skyward 会重新刷回这些分区。

三星手机没有 fastboot，在下载模式（Download mode）中刷入。先在开发者选项里打开 OEM 解锁并解锁 bootloader，然后在 Odin 里取消“Auto Reboot”，把 Release 中这台手机的 `.tar.md5` 放进 AP 栏，第一次刷入时把关闭校验的 vbmeta（例如 TWRP 为这台手机提供的 `vbmeta_disabled.tar`）放进 USERDATA 栏，刷入之后按住音量减和电源键直到黑屏，立即改按音量加和电源键进入 recovery。原厂系统启动时会用 vendor 分区里的 `recovery-from-boot.p` 把 recovery 改回原厂的，所以第一次要直接进入 Skyward。用 Heimdall 的话，刷入 zip 里的镜像：

```sh
heimdall flash --RECOVERY skyward-v0.5.1-p3q-base-xxxxxxxxxxxx.img --no-reboot
```

Skyward 中的 adb 是 root shell，手机上无需授权：手机处于 recovery 的时候，任何人拿到手机和数据线都能访问其中的存储。

## 双系统的工作方式

Skyward 可以在手机自身分区中的系统旁边安装第二系统，主系统是 HyperOS 或者第三方 ROM 都可以，并在首页的“双系统”中切换。第二系统的根文件系统是 data 分区中的一个文件，文件系统不能移动或删除它，Skyward 直接通过分区读写其中的内容；两个系统的启动镜像保存在另一个这样的文件中。切换系统只改写下次启动槽位的 boot 和 init_boot 分区，每一步都先记录在 /metadata 中，切换过程中断电的话，Skyward 下次启动时会完成这次切换。vendor_boot、dtbo、recovery、vbmeta、固件和 super 都保持不变，第二系统使用主系统的 vendor_boot 和 dtbo 启动。

其他操作写入启动分区之前，Skyward 会先把主系统的启动镜像写回：安装、切换启动槽位、fastboot、恢复出厂设置，以及备份或恢复启动分区。系统更新尚未合并的时候，暂不切换到第二系统；备份 data 分区的时候不包含第二系统。

第二系统的安装包是 zip 文件，和其他安装包一样在“安装”页面安装：

| 文件 | 内容 |
| --- | --- |
| `system.toml` | `id`、`name`、`version` 和 `filesystem`；`init` 是根文件系统中第一个运行的程序，默认为 `/sbin/init`；`minimum` 是根文件系统至少需要的空间，单位为字节 |
| `boot.img` | bootloader 启动的内核 |
| `rootfs.img` | 根文件系统的镜像，格式为 `filesystem` 指定的文件系统 |
| `init_boot.img` | 只有自带第一阶段 ramdisk 的系统需要 |

没有自带 init_boot 镜像的系统，从 Skyward 生成的 init_boot 镜像启动。其中的程序按照主系统第一阶段的方式加载第一阶段的内核模块，包括模块之间的软依赖和内核命令行中给模块的参数，然后在 data 分区中找到根文件系统，挂载之后运行其中的 `init`。启动过程中出现任何错误，这个程序都会进入 Skyward，和主系统启动失败时进入 recovery 一样：它在 misc 中留下原因，把当前槽位 recovery 分区中的 ramdisk 按照 bootloader 启动 recovery 时内核解包的方式解包到自己的 ramdisk 上，然后交给 Skyward 的第一个程序运行，所以无论 bootloader 怎样处理重启，都能进入 Skyward。Skyward 显示原因；用户在“双系统”中切换之前，下次启动的仍是第二系统。启动长时间没有进展时，这个程序改为重启到 recovery；交给第二系统之前手机重启的话，下次启动时它会在 misc 中看到同样的请求，先进入 Skyward。两种方式都无法进入 Skyward 时，手机停留在出错的地方，不会反复启动同一个系统。自带第一阶段 ramdisk 的系统可以运行 Skyward 放在 `/skyward/map` 的程序，它把根文件系统映射为块设备，并输出设备节点的路径。每次切换到第二系统，Skyward 都用自己带的程序写入第二系统的 init_boot 镜像，所以更新 Skyward 以后，第二系统也用新的程序启动。

正在运行的系统也可以请求切换：在 misc 中写入 `boot-recovery` 和 `--switch_system=installed`，或者 `--switch_system=` 加上第二系统的 `id`，然后重启到 recovery。bootloader 仍然启动这个槽位的话（小米 13 Ultra 连着 USB 线时可能这样），Skyward 的程序会在下次启动时在 misc 中看到这个请求，先进入 Skyward。

## 构建

Skyward 使用 `rust-toolchain.toml` 指定版本的 Rust 和它的 `aarch64-linux-android` 目标、Android NDK r29 和 [cargo-ndk](https://github.com/bbqsrc/cargo-ndk) 构建。recovery 服务针对手机构建，打包工具针对电脑构建：

```sh
cargo ndk -t arm64-v8a --platform 30 build --release -p skyward
cargo build --release -p skyward-pack
```

第二系统启动时运行的第一个程序在任何动态库可用之前运行，因此构建为静态程序：

```sh
RUSTFLAGS="-C target-feature=+crt-static" cargo ndk -t arm64-v8a --platform 30 build --release -p skyward-system-init --target-dir target/system-init
```

打包工具从小米服务器上这个版本的完整更新包中取出原厂 recovery 分区，只读取这个分区所在的部分，并且按照设备配置文件核对 SHA-256：

```sh
target/release/skyward-pack stock --config devices/nezha.toml --out target/stock/nezha/recovery.img
```

然后在它的基础上构建镜像：

```sh
target/release/skyward-pack recovery \
  --stock target/stock/nezha/recovery.img \
  --binary target/aarch64-linux-android/release/skyward \
  --system-init target/system-init/aarch64-linux-android/release/system-init \
  --config devices/nezha.toml \
  --font resources/fonts/MiSansVF.ttf \
  --mono resources/fonts/KirkMono-Regular.ttf \
  --mono-license resources/fonts/KirkMono-OFL.txt \
  --out target/skyward-nezha.img
```

Skyward 在线更新时向 GitHub 询问的是构建它的仓库的最新版本：构建时用 `SKYWARD_RELEASES=<所有者>/<仓库>` 指定，没有指定时是 `kirklin/skyward`。本仓库的 CI 把它设为构建所在的仓库，所以 fork 构建的镜像只从这个 fork 自己的 Release 更新。

测试和代码检查在电脑上运行；只在手机上运行的代码按照手机目标检查：

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo ndk -t arm64-v8a --platform 30 clippy --workspace --all-targets -- -D warnings
```

名称以 `on_the_phone` 开头的测试会创建文件系统、挂载分区、设置加密密钥，这些操作只有手机能够完成。把一台已经进入 Skyward、数据分区已经打开的手机通过 adb 连接到电脑，`cargo ndk` 会把每个测试程序推送到手机上运行，测试逐个执行：

```sh
cargo ndk -t arm64-v8a --platform 30 test -p skyward -p skyward-block -p skyward-files -p skyward-install -p skyward-mtp -p skyward-vab on_the_phone -- --test-threads=1
```

`skyward-preview` 无需手机，在电脑上把页面绘制成 PNG 文件：每个页面完整绘制成长截图的样子，覆盖它可能处于的各种状态，另外还有触摸和动画过程中某些时刻的屏幕。`--lang` 按照地区代码选择语言，例如 `en` 或者 `zh-TW`：

```sh
cargo run --release -p skyward-preview -- target/preview --lang en
```

### 构建在第三方 recovery 上

`[base]` 中 `third_party = true` 的机型构建在第三方 recovery 上。基底镜像是 recovery 分区的内容，`partition_size` 是分区大小；打包工具先按 `sha256` 核对镜像，镜像里没有 TWRP 的文件、或者配置写的是原厂 recovery 而镜像里有，都会停止。`drop` 列出的基底文件不放进镜像，Skyward 替换基底自己的 recovery 程序，基底里和安装脚本要找的命令同名的程序换成指向 Skyward 的链接。基底的内核没有的模块、驱动读取的固件和程序需要的库用 `--add-file <电脑上的文件>=<ramdisk 中的路径>` 加入，已经在 ramdisk 里的文件不能覆盖。`--avb-key` 给出 PEM 格式的 RSA 私钥时，镜像末尾的 vbmeta 按 avbtool 的方式签名。三星手机还需要 Odin 用的 tar：

```sh
target/release/skyward-pack recovery \
  --stock p3q-recovery.img \
  --binary target/aarch64-linux-android/release/skyward \
  --system-init target/system-init/aarch64-linux-android/release/system-init \
  --config devices/p3q.toml \
  --font resources/fonts/MiSansVF.ttf \
  --mono resources/fonts/KirkMono-Regular.ttf \
  --mono-license resources/fonts/KirkMono-OFL.txt \
  --add-file files/lib/modules/dhd.ko=lib/modules/dhd.ko \
  --add-file files/lib/modules/modules.dep=lib/modules/modules.dep \
  --out target/skyward-p3q.img
target/release/skyward-pack odin --image target/skyward-p3q.img --out target/skyward-p3q.tar.md5
```

S21 Ultra 的 WLAN 驱动 `dhd.ko` 要用基底内核自己的源码、按基底内核的配置构建，vermagic 和符号版本才能对上；它读取的 `bcmdhd_sta.bin`、`bcmdhd_clm.blob`、`nvram_net.txt` 来自原厂 vendor 分区的 `etc/wifi`，放在 ramdisk 的 `system/etc/wifi`。

CI 从本仓库标签为 `base-images` 的 Release 中取基底：`<代号>-recovery.img`，以及需要加入的文件 `<代号>-files.zip`，zip 里每个文件放在它在 ramdisk 中的路径。基底按设备配置中的 `[base] sha256` 核对，没有写 `sha256` 时停止；zip 中的文件按仓库里的 `devices/<代号>.files.sha256` 核对，这个文件是在 zip 解压出的目录里运行 `sha256sum` 的输出，只有其中列出的文件会加入镜像。`base-images` 中还没有这台手机的基底时，CI 跳过这台手机。

### 翻译

每种语言的文字是 `crates/skyward-ui/locales` 里的一个文件，以地区代码命名，例如 `tr.toml`、`pt-BR.toml`。`en.toml` 是参照文件，每条文字上方的注释说明它显示在哪里。构建时会读取全部文件，缺少某条文字，或者花括号里的名称和英文不一致，构建就会停止。

修正翻译时，直接修改对应语言文件里的文字。用 `skyward-preview --lang` 把页面按这种语言绘制出来检查效果：每绘制完一个页面，它会列出这个页面上显示不下的文字：`cut short in` 是以省略号结尾的文字，`set at 80% in` 是为了放进按键、按钮或滑动条而缩小了字号的文字，`past the edge in` 是超出屏幕的文字，`overlap in` 是互相重叠的两段文字。

## 许可证

[GPL-3.0-or-later](LICENSE)
