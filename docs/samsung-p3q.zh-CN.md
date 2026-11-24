# Galaxy S21 Ultra（p3q）移植说明与安全审计

本仓库在上游 [kirklin/skyward](https://github.com/kirklin/skyward) v0.5.1（`43ec6da`）上加入了三星 Galaxy S21 Ultra 5G 骁龙版（SM-G9980/G998U/G998U1/G998W，代号 `p3q`）的支持。移植参考了：

- [yanzihan/Skyward-Galaxy](https://github.com/yanzihan/Skyward-Galaxy)（审计时为 `c950718`，2026-11-23）
- [yanzihan/android_kernel_samsung_sm8350-p3q](https://github.com/yanzihan/android_kernel_samsung_sm8350-p3q)（`sixteen` 与 `lineage-23.2` 分支为 `fffbecbe`，`oneui-8.5` 分支为 `3af90cf2`）

代码按上游的写法重新实现，没有从参考仓库复制任何二进制文件：基底 recovery 镜像、`dhd.ko`、WLAN 固件、VNDK 库、super 分区元数据、Magisk 程序都不在本仓库里。`p3q` 的测试数据只有原厂 vendor 分区的 init 脚本和 VINTF 清单（文本），以及代替程序的空文件。

## 一、安全审计

### 应用仓库 Skyward-Galaxy

**结论：没有发现恶意代码。** 检查了它相对上游的全部改动（约 6400 行新增）、源码中出现的每一个网络地址、每一处启动外部程序的代码，以及仓库中的二进制文件。

- 网络：新增的地址只有 GitHub 的 API 和 Release 下载（在线更新）以及 `connectivitycheck.gstatic.com`（判断是否联网），其余都是测试用例里的示例地址。没有统计、上报或远程下发命令。
- 进程：新增的外部程序调用只有按 vendor 的服务描述启动 `servicemanager`。
- 二进制：测试里的 `vendor/bin/*` 是空文件；`p3q-super-metadata.bin` 是他手机上的分区表元数据（本仓库没有收入，改用构造的测试数据）。

需要注意的风险（不是恶意，但本仓库已经改掉）：

| 问题 | 影响 | 本仓库的处理 |
| --- | --- | --- |
| 在线更新写死为 `yanzihan/Skyward-Galaxy` 的 Release | 装了他构建的镜像后，Skyward（在 recovery 中以 root 运行）会从他的发布渠道更新自己 | 更新来源在构建时由 `SKYWARD_RELEASES=<所有者>/<仓库>` 指定，CI 设为构建所在的仓库，未指定时是上游；编译时检查格式 |
| 基底镜像和附加文件（OrangeFox 镜像、`dhd.ko`、固件、VNDK 库）从他的 `base-images` Release 下载，没有固定哈希 | 发布里的文件被替换后，构建会悄悄用上新的二进制 | 基底按 `[base] sha256` 核对；附加文件按仓库中的 `devices/<代号>.files.sha256` 核对，只加入列出的文件，不能覆盖 ramdisk 中已有的文件 |
| `[wipe] logs` 含 `/data/vendor/wifi` | “清除日志”会删除已保存的 WLAN 网络和 supplicant 配置 | 去掉，换成真正的日志目录 |
| README 让三星用户 `fastboot flash recovery` | 三星手机没有 fastboot | 改为 Odin/Heimdall 的刷入方法，并生成 Odin 用的 `.tar.md5` |
| 构建脚本写死个人路径，依赖本地 avbtool 和个人私钥 | 别人无法复现构建 | 打包工具自己写 AVB footer（与 avbtool 字节一致），可选 `--avb-key` 签名；`odin` 子命令生成 `.tar.md5` |

另外，Skyward 上游本身的设计是 recovery 中 `ro.secure=0`、`ro.adb.secure=0`：adb 是无需授权的 root shell，拿到手机和数据线的人在 recovery 中就能读取存储。这一点没有改变，使用时需要知道。

### 内核仓库 android_kernel_samsung_sm8350-p3q

**结论：没有发现针对用户的恶意代码，但有几处改动明显降低了安全性，不建议直接使用这个内核或它的编译产物，尤其是 `oneui-8.5` 分支。**

1. **高危：KernelSU 的 root 控制 fd 泄露给子进程**（`oneui-8.5`：`a3ef6964`、`540485ef`、`1fb6c5ea`）。驱动 fd 去掉了 `O_CLOEXEC`，并且在管理器每次 `execve`/`execveat` 和 `bprm_committing_creds` 时再装一个可继承的新 fd。管理器 UID 下执行的任何程序，以及它们的所有子孙进程，都拿着能控制 root 的 fd。上游 KernelSU 刻意用 `O_CLOEXEC` 防止这种泄露。
2. **高危：内核身份覆盖任意文件、泄露内核地址**（`oneui-8.5`：`5364dc4f`、`1fb6c5ea`）。这段调试代码没有任何编译开关，总是启用：内核线程每 10 秒用 `filp_open(O_WRONLY | O_CREAT | O_TRUNC)` 写 `/data/local/tmp/ksu_dbg.log` 和 `/cache/ksu_dbg.log`，没有 `O_NOFOLLOW`；`/data/local/tmp` 对 adb shell 可写，放一个符号链接就能让内核截断并覆盖链接指向的文件。日志里用 `%px` 记录了内核指针，经 0444 的 `/proc/ksu_dbg` 和 0644 的日志文件都能读到，可以绕过 KASLR。
3. **中：运行时改写系统调用表，关闭 FIPS**（`oneui-8.5`：`ddeaff7c`）。`CONFIG_KSU_TAMPER_SYSCALL_TABLE=y`，`CONFIG_CRYPTO_FIPS` 关闭。
4. **中：移除三星内核防护、伪装解锁状态**（所有分支）。彻底删除 DEFEX、HDM、KNOX_NCM、ngksm 和 uH/RKP（fastuh）；`PROC_SPOOF_CMDLINE` 和伪装 `vbmeta.device_state`（`faad7cb5`、`2130a2a7`）对应用隐藏 bootloader 已解锁；`/sys/fs/pstore` 目录权限从 0750 放宽到 0755（`435eba96`）。
5. **供应链：提交了无法审计的预编译程序**（`fffbecbe`）。`magisk_pc/` 下的 `magisk`、`magiskboot`、`magiskinit`、`init-ld`、`stub.apk` 和 `tools/magiskboot`，以及一份原厂 ramdisk。
6. **低**：`3af90cf2` 让 KernelSU 识别三星预装应用的 APK 路径，只影响取包名，签名校验没有改变。

建议：p3q 的 recovery 只需要基底内核缺少的 WLAN 驱动 `dhd.ko`。用基底 recovery 自己的内核源码（三星开源的 SM-G9980 内核，或者基底作者公开的内核树）按它的配置只编译这个模块，不要使用上面这个内核仓库的产物和预编译程序。

### 依赖（cargo audit）

用 `cargo audit`（RustSec 数据库，2026-11-24）检查整个依赖树，其中大部分来自上游原有的依赖。处理结果：

| 依赖 | 公告 | 处理 |
| --- | --- | --- |
| `rsa` 0.9.10 | RUSTSEC-2023-0071，Marvin 时序侧信道 | 升到 0.10.0 |
| `libsqlite3-sys` 0.38.2（`rusqlite` 0.40） | RUSTSEC-2026-0459，内置 SQLite 的漏洞 | `rusqlite` 升到 0.43.0，内置 SQLite 随之更新；读取 locksettings 数据库的测试通过 |
| `url` 2.5.8、`rustix` 1.1.5 | RUSTSEC-2026-0592、RUSTSEC-2026-0486 | 兼容版本内更新到 2.5.9、1.1.6 |
| `zip` 9.1.0（`unpayload` 使用） | RUSTSEC-2026-0548 等 | 兼容版本内更新到 9.8.2 |
| `zip` 8.6.0 | `ZipArchive::extract`、`ZipStreamReader`、AES 和旧压缩算法的 8 条公告 | 保留上游的版本：Skyward 不调用 `extract` 和 `ZipStreamReader`，编译时没有打开 AES 和旧压缩算法；它自己的 `unzip` 拒绝绝对路径和 `..`，`7za` 用 `enclosed_name`。升到 9.x 要改上游安装器多处接口，等上游升级时一起合并 |
| `crypto-bigint` 0.5.5（`p256` 0.13） | RUSTSEC-2026-0387，常数时间运算的时序差异 | 保留：只用来以公钥验证刷机包的 ECDSA 签名，不涉及私钥 |
| `ruzstd` 0.8.3、`lru` 0.12.5（`rust-fs-erofs` 0.6.0） | RUSTSEC-2026-0344 解压炸弹，以及两条 unsound 警告 | 保留：只在电脑上的打包工具读取官方完整包里的 EROFS 分区时使用，读取的数据按包内自带的校验和核对；`rust-fs-erofs` 已是最新版本 |
| `ttf-parser` 0.25.1 | RUSTSEC-2026-0192，不再维护 | 保留：只解析镜像里自带的字体 |

## 二、本仓库的移植

小米机型的行为保持不变：三星相关的行为只在设备配置 `[base] third_party = true`，或者手机本身的数据（没有槽位后缀的分区、super 元数据中不带后缀的分区名、没有 boot control）需要时才生效。

- **打包工具**：设备配置的 `[base]`（基底镜像的 SHA-256、分区大小、去掉的基底文件、是否加入 MTP 规则）；boot 镜像 v2 头和 dtb、gzip 和 lz4 ramdisk；`--add-file`（检查 ramdisk 内路径，不覆盖已有文件）；`--avb-key` 按 avbtool 的格式签名 vbmeta（用 AOSP 测试密钥和 openssl 验证过）；`odin` 子命令生成内容固定、带 MD5 行的 `.tar.md5`。
- **没有槽位的分区**：安装、备份恢复和在线更新按分区实际名称工作，不再假定 `_a`/`_b`；从 super 元数据读出不带后缀的分区名；复用已有的 dm 映射；备份恢复前先发现只读分区（三星的 vbmeta）。
- **安全世界**：Keymaster 4.0 和 Gatekeeper 1.0 的 HIDL 接口（binder scatter-gather 缓冲区、libbinder 的分区标记）；按原厂 init 脚本启动 qseecomd、ssgtzd，然后是密钥和凭据服务；没有 boot control 的手机不启动它；weaver 支持 HIDL 版本。
- **第三方 recovery 基底**：从已安装系统补上 `ro.hardware.*`；硬件服务先用 vendor 自己的库并预加载 recovery 的 libc++；按需要启动 servicemanager 并重启 hwservicemanager；让 adb 在基底自己设置的 USB 端口上起来。
- **硬件**：timed_output 振动；Broadcom dhd WLAN（HIDL 版 supplicant、VNDK 库目录 `$bundle`、可配置等待时间）；按 `androidboot.fstab_suffix` 找到 `fstab.default`；挂载 apnhlos（固件）和 persist。
- **依赖**：见上面的依赖审计。打包工具和安装器使用的 `rsa` 从 0.9 升到 0.10，修复 RUSTSEC-2023-0071（Marvin 时序侧信道）。安装器只用公钥验证刷机包签名，打包工具只在构建机上离线签名，原来的用法都不能被远程利用，升级是为了不带已知漏洞的依赖，也让整个项目只用一套 RSA 实现；签名结果用 openssl 和按 avbtool 算法独立算出的公钥数据核对过。
- **其他修复**：DHCP 经过中继时带回中继信息、只接受对应的应答；supplicant 控制接口在 EINTR 时继续等待；按 `LOOP_GET_STATUS64` 找回已有的 loop 设备；时间守护进程的偏移文件按三星的布局读取；清除数据时 cache 和 /metadata 按手机自己的文件系统重新格式化；系统名称显示 One UI 版本。

## 三、构建和刷入

1. 准备基底：p3q 的 OrangeFox 或 TWRP `recovery.img`（recovery 分区的内容）。算出 SHA-256，写进 `devices/p3q.toml` 的 `[base] sha256`。现在写在里面的是下面“编译 OrangeFox 基底”得到的镜像；换用别的基底时改成那份镜像的值。
2. 准备附加文件（目录结构就是它们在 ramdisk 中的位置）：
   - `lib/modules/dhd.ko` 和对应的 `lib/modules/modules.dep`（按上面的建议自己编译）；
   - `system/etc/wifi/bcmdhd_sta.bin`、`bcmdhd_clm.blob`、`nvram_net.txt`，来自原厂 vendor 分区的 `etc/wifi`；
   - 基底内核在 init 之前就请求的触摸固件（参考移植加入了 `lib/firmware/y792_p3.bin`，缺少时启动会等满 60 秒的固件超时）；
   - 只有 vendor 分区被改过的 ROM 才需要：`system/wifi-vndk/lib64/` 下原厂 VNDK 30 的 `android.hardware.wifi*` 库、`libssl.so`、`libcrypto.so`。
3. 本地构建见 README 的“构建在第三方 recovery 上”。用 CI 的话，在仓库里建一个标签为 `base-images` 的 Release，上传 `p3q-recovery.img` 和 `p3q-files.zip`，并提交 `devices/p3q.files.sha256`（在 zip 解压出的目录里运行 `sha256sum <每个文件>` 的输出）。
4. 刷入：Odin 取消 Auto Reboot，`.tar.md5` 放 AP 栏，第一次同时把关闭校验的 vbmeta 放 USERDATA 栏；刷完直接按键进入 recovery，因为原厂系统启动时会把 recovery 改回原厂的。

### 编译 OrangeFox 基底

`devices/p3q.toml` 固定的基底 `OrangeFox-R12.0-Unofficial-p3q.img`（SHA-256 `5d8083ac7805df7241beb91cdee016b99b5a969c5807c4d23ceba1777be8e31c`）是这样编译的：

- **源码**：按 OrangeFox 官方 sync 脚本（`gitlab.com/OrangeFox/sync`）对 14.1 的步骤：`repo init --depth=1 -u https://github.com/nebrassy/platform_manifest_twrp_aosp.git -b twrp-14`（AOSP `android-14.0.0_r67`），打上脚本自带的四个补丁，同步 `external/guava`、`external/gflags`、`hardware/google/interfaces`、`hardware/google/pixel`，再克隆 OrangeFox 的 `system/update_engine`、`external/se_omapi`（`fox_14.1`）、`bootable/recovery`（`fox_14.1`，`1d94174`）和 `vendor/recovery`（`main`，`1998cfc`）。
- **设备树**：[yanzihan/android_device_samsung_p3q-fox-14.1](https://github.com/yanzihan/android_device_samsung_p3q-fox-14.1) 的 `bb194c2`，放在 `device/samsung/p3q`。它的 `prebuilt/Image.gz` 是三星原厂内核 `5.4.274-qgki-2370012-abG9980ZCSDHYF2`（三星编译服务器 `dpi@21DK7915` 编译，SHA-256 `7ea3b698…`，没有 KernelSU），触摸模块的 vermagic 与它一致。设备树里的脚本、init 文件和 fstab 都看过，没有联网或可疑的命令。只改了三处：删掉 `vendorsetup.sh` 里指向作者本机的 `FOX_USE_SPECIFIC_MAGISK_ZIP`（OrangeFox 改用自带的 Magisk，它只进安装 zip，不进 `recovery.img`），维护者 `OF_MAINTAINER` 和 `TW_DEVICE_VERSION` 改成中性的名字。
- **编译**：`export ALLOW_MISSING_DEPENDENCIES=true FOX_BUILD_DEVICE=p3q LC_ALL=C`，`source build/envsetup.sh`，`lunch twrp_p3q-ap2a-eng`（Android 14 QPR 的 lunch 要带 release，这棵树里只有 `ap2a`），`mka adbd recoveryimage`。
- **内存不到 16 GB 时**：Android 14 的 soong 分析在 15 GB 内存上会被 OOM 杀掉，这台构建机上做了两件只影响编译环境、不影响镜像内容的事。一是在 `build/blueprint/bootstrap/command.go` 给 `soong_build` 加 `debug.SetMemoryLimit(11 << 30)`（它在 `env -i` 下运行，`GOMEMLIMIT` 传不进去）。二是在 recovery 用不到的目录里放 `.find-ignore`，让 soong 不分析它们：`cts`、`development`（`development/sdk` 除外）、`test`（`test/app_compat` 除外，里面有 `csuite_test` 插件）、`external/noto-fonts`、`external/cronet`、`tools/` 下的测试工具、Java/应用类的主线模块，以及名为 `tests`、`test`、`vts` 的目录。下面这些被其他模块用到，必须保留：`packages/modules/Media`、`external/avb/test`、ART 的测试目录、`system/sepolicy/tests`（`fc_sort`）、`test/vts-testcase/hal/treble/vintf/libvts_vintf_test_common`、`packages/modules/Connectivity/staticlibs/netd/libnetdutils` 和 `.../staticlibs/native/bpf_headers`。磁盘不够时还删了用不到的预编译件：另外四个 clang 版本、rust 的 1.73/1.74、macOS/musl/Windows 版本、ABI dump、VNDK 快照、JDK 11/21。
- **检验**：镜像是 boot header v2，大小正好是 recovery 分区（81788928 字节），末尾有 AVB footer；里面的内核与设备树的原厂内核逐字节相同。用它和 `devices/p3q.toml` 打包 Skyward 成功，镜像内容 72.0 MB，还剩 9.8 MB 放 WLAN 驱动、固件和库。同时得到的还有 Odin 用的 `OrangeFox-R12.0-Unofficial-p3q.img.tar` 和 OrangeFox 的安装 zip。

## 四、没有验证的部分

本移植没有在真机上运行过，下面这些需要在手机上确认：

- OrangeFox 基底本身也没有在手机上启动过：先单独刷入它，确认能进入 OrangeFox、触摸和 adb 正常，再刷构建在它上面的 Skyward。

- 显示密度按同分辨率、尺寸相近的小米 13 Ultra 取 560，没有设置圆角半径。
- WLAN：`dhd.ko` 如果按 `DHD_LINUX_STD_FW_API` 编译（PCIe 默认如此），会通过内核的固件加载器请求 `bcmdhd_sta.bin`、`nvram.txt`、`bcmdhd_clm.blob`；如果 `dmesg` 里有 `request_firmware` 失败，就在 `[wlan]` 里用 `firmware` 把这些名字映射到 `system/etc/wifi` 下的文件。
- 解密：从原厂 init 脚本读出要启动的程序已经用测试核对过，Keymaster/Gatekeeper 在这台手机上的实际运行没有验证；`[firmware]` 的 apnhlos 和 `[partitions]` 的 persist 分区名按三星骁龙机型的惯例填写。
- 备份 EFS 时包含 efs、sec_efs、modemst1、modemst2、fsg、fsc、persist；手机上缺少其中任何一个分区时备份会停止，需要按 `/dev/block/by-name` 调整。
