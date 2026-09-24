use chrono::Local;
use log::{LevelFilter, error, info, warn};
use log4rs::{
    append::file::FileAppender,
    config::{Appender, Config, Root},
    encode::pattern::PatternEncoder,
};
use std::collections::VecDeque;
use std::error::Error;
use std::fs;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::{
    env, io,
    process::{Command, Stdio},
};

// Linux 系统配置目录（默认值，可被 --target-dir / FLYRS_RIME_DIR 覆盖）
const RIME_SYSTEM_DIR: &str = "/usr/share/rime-data";
// macOS 鼠须管配置目录
const MACOS_RIME_USER_DIR: &str = "~/Library/Rime";
const DEFAULT_PACKAGE: &str = "./小鹤音形“鼠须管”for macOS.zip";
// 目标目录覆盖：环境变量
const ENV_TARGET_DIR: &str = "FLYRS_RIME_DIR";
// 可选的 rsync --iconv 参数，例如 FLYRS_RSYNC_ICONV=UTF-8,GBK。
// 默认不加：源与目标的文件名都已是 UTF-8，强制 iconv 反而会在缺 locale、
// 或遇到非法编码文件名时放大成 rsync 报错。
const ENV_RSYNC_ICONV: &str = "FLYRS_RSYNC_ICONV";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 初始化日志系统
    init_logger()?;

    // 解析命令行参数
    let opts = match parse_args(env::args().collect()) {
        Ok(o) => o,
        Err(e) => {
            error!("{}", e);
            return Err(e);
        }
    };
    if opts.help {
        println!("{}", usage());
        return Ok(());
    }

    let package_path = opts
        .package
        .clone()
        .unwrap_or_else(|| DEFAULT_PACKAGE.to_string());
    info!("使用配置包: {}", package_path);

    info!("===== 开始安装小鹤音形输入法 =====");
    info!("时间: {}", Local::now().format("%Y-%m-%d %H:%M:%S"));

    // 检测操作系统
    let target_os = env::consts::OS;
    info!("当前操作系统: {}", target_os);

    // 根据操作系统选择目标目录；命令行 > 环境变量 > 系统默认
    let target_override = resolve_target_override(opts.target_dir.as_deref());
    let (target_dir, is_macos) = if target_os == "macos" {
        // 展开 ~ 为用户目录
        let home_dir = env::var("HOME").map_err(|_| "无法获取用户目录")?;
        let user_dir = MACOS_RIME_USER_DIR.replace('~', &home_dir);
        (target_override.unwrap_or(user_dir), true)
    } else {
        (target_override.unwrap_or_else(|| RIME_SYSTEM_DIR.to_string()), false)
    };

    info!("目标配置目录: {}", target_dir);

    // 非 macOS 系统需要安装依赖
    if !is_macos {
        let package_managers = [
            PackageManager {
                name: "pacman",
                update_cmd: "sudo pacman -Sy",
                install_args: "-S --noconfirm",
            },
            PackageManager {
                name: "apt",
                update_cmd: "sudo apt update",
                install_args: "install -y",
            },
            PackageManager {
                name: "dnf",
                update_cmd: "sudo dnf check-update",
                install_args: "install -y",
            },
        ];

        let dependencies = ["7z", "rsync"];

        // 检查并安装依赖
        check_and_install_dependencies(&package_managers, &dependencies)?;

        // 检查输入法框架
        check_input_method_framework(&package_managers)?;
    } else {
        info!("macOS 系统跳过依赖检查");
    }

    // 1. 获取配置文件
    let config_dir = match get_config_files(Some(&package_path)) {
        Ok(dir) => dir,
        Err(e) => {
            error!("获取配置文件失败: {}", e);
            return Err(e);
        }
    };

    // 2. 复制文件到目标目录
    if is_macos {
        info!("\n正在复制配置文件到用户目录: {}", target_dir);
        copy_to_user_dir_macos(&config_dir, &target_dir)?;
    } else {
        copy_to_system_dir_linux(&config_dir, &target_dir)?;
    }

    info!("\n✅ 安装完成！请重新部署 Rime 输入法");
    if is_macos {
        info!("在任务栏右键点击输入法图标 -> 选择【重新部署】");
        info!("然后按 Ctrl + \\ 或 F4 切换到小鹤音形");
    } else {
        info!("在输入法设置中选择重新部署");
    }

    Ok(())
}

/// 命令行选项
struct Options {
    package: Option<String>,
    target_dir: Option<String>,
    help: bool,
}

fn usage() -> String {
    format!(
        "用法: flyrs [选项] [配置包路径]\n\
         \n\
         选项:\n\
         \x20 --target-dir <目录>   指定 Rime 配置目标目录\n\
         \x20                       也可用环境变量 {env} 指定\n\
         \x20                       默认: {default}\n\
         \x20 -h, --help            显示本帮助\n\
         \n\
         环境变量:\n\
         \x20 {env}          目标目录\n\
         \x20 {iconv} 传给 rsync 的 --iconv 值（默认不加）\n",
        env = ENV_TARGET_DIR,
        default = RIME_SYSTEM_DIR,
        iconv = ENV_RSYNC_ICONV,
    )
}

fn parse_args(args: Vec<String>) -> Result<Options, Box<dyn Error>> {
    let mut opts = Options {
        package: None,
        target_dir: None,
        help: false,
    };

    let mut i = 1;
    while i < args.len() {
        let arg = args[i].clone();
        match arg.as_str() {
            "-h" | "--help" => opts.help = true,
            "--target-dir" => {
                i += 1;
                if i >= args.len() {
                    return Err("--target-dir 需要一个目录参数".into());
                }
                opts.target_dir = Some(args[i].clone());
            }
            a if a.starts_with("--target-dir=") => {
                opts.target_dir = Some(a["--target-dir=".len()..].to_string());
            }
            a if a.starts_with('-') && a.len() > 1 => {
                return Err(format!("未知选项: {}", a).into());
            }
            _ => {
                if opts.package.is_some() {
                    return Err(format!("用法: {} [选项] [配置包路径]", args[0]).into());
                }
                opts.package = Some(arg);
            }
        }
        i += 1;
    }

    Ok(opts)
}

/// 解析目标目录覆盖值：命令行优先，其次环境变量
fn resolve_target_override(cli: Option<&str>) -> Option<String> {
    if let Some(t) = cli {
        if !t.trim().is_empty() {
            return Some(t.to_string());
        }
    }
    if let Ok(t) = env::var(ENV_TARGET_DIR) {
        if !t.trim().is_empty() {
            return Some(t);
        }
    }
    None
}

/// 初始化日志系统
fn init_logger() -> io::Result<()> {
    // 创建日志目录
    let log_dir = "logs";
    fs::create_dir_all(log_dir)?;

    // 生成带时间戳的日志文件名
    let timestamp = Local::now().format("%Y%m%d_%H%M%S");
    let log_file = format!("logs/rime_install_{}.log", timestamp);

    // 配置日志系统
    let logfile = FileAppender::builder()
        .encoder(Box::new(PatternEncoder::new(
            "{d(%Y-%m-%d %H:%M:%S)} | {l} | {t} | {m}{n}",
        )))
        .build(&log_file)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

    let config = Config::builder()
        .appender(Appender::builder().build("logfile", Box::new(logfile)))
        .build(Root::builder().appender("logfile").build(LevelFilter::Info))
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

    log4rs::init_config(config).map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

    info!("日志系统已初始化，日志文件: {}", log_file);

    Ok(())
}

// 包管理器信息
struct PackageManager {
    name: &'static str,
    update_cmd: &'static str,
    install_args: &'static str,
}

// 检查并安装依赖
fn check_and_install_dependencies(
    package_managers: &[PackageManager],
    dependencies: &[&str],
) -> Result<(), Box<dyn Error>> {
    info!("检查系统依赖……");

    let mut missing_deps = Vec::new();

    // 检查每个依赖是否存在
    for &dep in dependencies {
        if command_exists(dep) {
            info!("已安装: {}", dep);
        } else {
            error!("未安装: {}", dep);
            missing_deps.push(dep.to_string());
        }
    }

    // 如果有缺失的依赖，尝试安装
    if !missing_deps.is_empty() {
        info!("尝试安装缺失的依赖: {:?}", missing_deps);

        // 检测包管理器
        let package_manager = package_managers
            .iter()
            .find(|pm| Path::new(&format!("/usr/bin/{}", pm.name)).exists());

        match package_manager {
            Some(pm) => {
                info!("检测到包管理器: {}", pm.name);

                // 构建安装命令
                let deps = missing_deps.join(" ");
                let install_cmd = format!(
                    "{} && sudo {} {} {}",
                    pm.update_cmd, pm.name, pm.install_args, deps
                );

                info!("将执行以下命令安装依赖:");
                info!("{}", install_cmd);
                info!("请在提示时输入您的密码");

                // 执行安装命令
                let status = Command::new("sh")
                    .arg("-c")
                    .arg(&install_cmd)
                    .stdout(Stdio::inherit())
                    .stderr(Stdio::inherit())
                    .status()?;

                if !status.success() {
                    error!("依赖安装失败: {}", install_cmd);
                    return Err(format!("依赖安装失败: {}", install_cmd).into());
                }

                info!("依赖安装成功");
            }
            None => {
                warn!("无法确定包管理器，请手动安装: {:?}", missing_deps);
                return Err(format!("无法确定包管理器，请手动安装: {:?}", missing_deps).into());
            }
        }
    }

    Ok(())
}

/// 检查输入法框架是否安装。
///
/// fcitx5-rime / ibus-rime 是插件包，提供的是 librime.so 之类的共享库，
/// 并不提供同名的可执行文件，因此不能用 `which fcitx5-rime` 判断。
fn check_input_method_framework(
    package_managers: &[PackageManager],
) -> Result<(), Box<dyn Error>> {
    info!("检查输入法框架依赖……");

    if rime_framework_installed() {
        return Ok(());
    }

    warn!("未检测到 Rime 输入法框架，默认尝试安装 fcitx5-rime...");

    // 尝试安装 fcitx5-rime
    if let Some(pm) = package_managers.iter().find(|pm| command_exists(pm.name)) {
        let install_cmd = format!(
            "{} && sudo {} {} {}",
            pm.update_cmd, pm.name, pm.install_args, "fcitx5-rime"
        );

        info!("将执行以下命令安装输入法框架: {}", install_cmd);

        let status = Command::new("sh")
            .arg("-c")
            .arg(&install_cmd)
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()?;

        if status.success() {
            info!("输入法框架安装成功");
            return Ok(());
        }
    }

    error!("输入法框架安装失败，请手动安装 fcitx5-rime 或 ibus-rime");
    Err("输入法框架安装失败".into())
}

/// 通过 librime 共享库或所属软件包判断 Rime 框架是否已安装
fn rime_framework_installed() -> bool {
    const LIB_PATHS: [&str; 5] = [
        "/usr/lib/fcitx5/librime.so",
        "/usr/lib/ibus/librime.so",
        "/usr/lib64/fcitx5/librime.so",
        "/usr/local/lib/fcitx5/librime.so",
        "/usr/lib/librime.so",
    ];
    for p in LIB_PATHS {
        if Path::new(p).exists() {
            info!("已安装输入法组件: {} (librime)", p);
            return true;
        }
    }

    // 通用搜索插件目录里的 librime.so*
    for dir in ["/usr/lib/fcitx5", "/usr/lib/ibus", "/usr/lib64/fcitx5"] {
        if lib_exists_in(dir, "librime.so") {
            info!("已安装输入法组件: {}/librime.so", dir);
            return true;
        }
    }

    // 退一步：用包管理器确认插件包已安装
    if package_installed("fcitx5-rime") || package_installed("ibus-rime") {
        return true;
    }

    false
}

/// 目录下是否存在以 prefix 开头的共享库文件（如 librime.so / librime.so.1）
fn lib_exists_in(dir: &str, prefix: &str) -> bool {
    let path = Path::new(dir);
    if !path.is_dir() {
        return false;
    }
    fs::read_dir(path)
        .map(|rd| {
            rd.filter_map(|e| e.ok()).any(|e| {
                e.file_name()
                    .to_str()
                    .map(|n| n.starts_with(prefix))
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

/// 用包管理器查询某个包是否已安装
fn package_installed(pkg: &str) -> bool {
    let queries: [(&str, &[&str]); 3] = [
        ("pacman", &["-Q", pkg]),
        ("dpkg", &["-s", pkg]),
        ("rpm", &["-q", pkg]),
    ];
    for (bin, args) in queries {
        if !command_exists(bin) {
            continue;
        }
        let ok = Command::new(bin)
            .args(args)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if ok {
            info!("已安装输入法组件: {} (由 {} 确认)", pkg, bin);
            return true;
        }
    }
    false
}

/// 检查命令是否存在
fn command_exists(cmd: &str) -> bool {
    Command::new("which")
        .arg(cmd)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// 探测可用的 UTF-8 locale，优先 zh_CN.UTF-8，其次 C.UTF-8 / C.utf8 等
fn detect_utf8_locale() -> Option<String> {
    static CACHE: OnceLock<Option<String>> = OnceLock::new();
    CACHE
        .get_or_init(|| {
            let preferred = [
                "zh_CN.UTF-8",
                "zh_CN.utf8",
                "C.UTF-8",
                "C.utf8",
                "en_US.UTF-8",
                "en_US.utf8",
            ];
            let output = Command::new("locale").arg("-a").output().ok()?;
            if !output.status.success() {
                return None;
            }
            let text = String::from_utf8_lossy(&output.stdout);
            let available: Vec<&str> = text
                .lines()
                .map(|l| l.trim())
                .filter(|l| !l.is_empty())
                .collect();
            for p in preferred {
                if let Some(found) = available.iter().find(|a| a.eq_ignore_ascii_case(p)) {
                    return Some((*found).to_string());
                }
            }
            // 兜底：任意 UTF-8 locale
            available
                .iter()
                .find(|a| {
                    let l = a.to_lowercase();
                    l.ends_with(".utf-8") || l.ends_with(".utf8")
                })
                .map(|a| (*a).to_string())
        })
        .clone()
}

/// 给命令设置探测到的 UTF-8 locale；探测不到则不设置，沿用系统默认
fn apply_locale(cmd: &mut Command) {
    if let Some(loc) = detect_utf8_locale() {
        cmd.env("LANG", &loc).env("LC_ALL", &loc);
    }
}

/// 组装命令：需要提权时以 sudo 前缀执行
fn command_with_sudo(program: &str, use_sudo: bool) -> Command {
    if use_sudo {
        let mut cmd = Command::new("sudo");
        cmd.arg(program);
        cmd
    } else {
        Command::new(program)
    }
}

/// 目标路径是否可直接写入（含“父目录可写、目标尚不存在”的情况）
fn path_writable_or_creatable(target: &Path) -> bool {
    if target.exists() {
        return probe_write(target);
    }
    let mut cur = target.parent();
    while let Some(p) = cur {
        if p.exists() {
            return probe_write(p);
        }
        cur = p.parent();
    }
    false
}

/// 在目录中实际创建一个临时文件来测试可写性
fn probe_write(dir: &Path) -> bool {
    let probe = dir.join(format!(".flyrs-write-probe-{}", std::process::id()));
    match fs::File::create(&probe) {
        Ok(_) => {
            let _ = fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

/// 需要 sudo 时提前确认 sudo 可用，避免非交互环境下留下含糊的失败信息
fn ensure_sudo_available(use_sudo: bool) -> Result<(), Box<dyn Error>> {
    if !use_sudo {
        return Ok(());
    }
    let non_interactive_ok = Command::new("sudo")
        .arg("-n")
        .arg("true")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if non_interactive_ok {
        return Ok(());
    }
    if io::stdin().is_terminal() {
        info!("系统目录需要管理员权限，sudo 稍后将提示您输入密码");
        return Ok(());
    }
    Err(format!(
        "目标目录需要管理员权限（sudo），但当前不是交互式终端、sudo 也无法免密执行。\n\
         请在终端中交互式运行，或使用 --target-dir / {env} 指定一个当前用户可写的目录。",
        env = ENV_TARGET_DIR
    )
    .into())
}

/// 计算备份目录：与目标目录同级，保持原有 “rime-backup-<时间戳>” 命名习惯
fn backup_dir_for(target: &str) -> String {
    let path = Path::new(target);
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "rime-data".to_string());
    let stem = if name == "rime-data" {
        "rime-backup".to_string()
    } else {
        format!("{}-backup", name)
    };
    parent
        .join(format!("{}-{}", stem, Local::now().format("%Y%m%d_%H%M%S")))
        .to_string_lossy()
        .into_owned()
}

/// 查找解压后的 rime 配置目录（支持多种策略）
fn find_config_directory(extract_dir: &str) -> Result<String, Box<dyn std::error::Error>> {
    // 策略一（首选）：找到含有 *.schema.yaml 的目录，即真正的 rime 配置目录。
    // 官方压缩包解压后顶层是「小鹤音形Rime平台鼠须管for macOS」，
    // 其下才是 rime/，配置必须取 rime/ 这一层，否则会多出一层 rime/。
    if let Some(dir) = find_dir_with_schema(Path::new(extract_dir)) {
        info!("按 *.schema.yaml 定位到 rime 配置目录: {}", dir.display());
        return Ok(dir.to_string_lossy().into_owned());
    }

    // 策略二：查找第一个子目录
    for entry in fs::read_dir(extract_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            warn!(
                "未找到 *.schema.yaml，回退使用第一个子目录: {}",
                path.display()
            );
            return Ok(path.to_str().unwrap().to_string());
        }
    }

    // 策略三：如果没有子目录，但有文件，说明是平铺结构，直接使用当前目录
    if fs::read_dir(extract_dir)?.next().is_some() {
        info!("ZIP 解压后未找到目录，使用根目录作为配置目录");
        return Ok(extract_dir.to_string());
    }

    error!("解压后未找到配置文件目录或文件");
    Err("解压后未找到配置文件目录".into())
}

/// 广度优先查找含 *.schema.yaml 的目录（保证取最浅的一层）
fn find_dir_with_schema(root: &Path) -> Option<PathBuf> {
    let mut queue: VecDeque<PathBuf> = VecDeque::new();
    queue.push_back(root.to_path_buf());
    while let Some(dir) = queue.pop_front() {
        if dir_has_schema(&dir) {
            return Some(dir);
        }
        if let Ok(rd) = fs::read_dir(&dir) {
            let mut subdirs: Vec<PathBuf> = rd
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect();
            subdirs.sort();
            for s in subdirs {
                queue.push_back(s);
            }
        }
    }
    None
}

/// 目录内是否直接包含 *.schema.yaml
fn dir_has_schema(dir: &Path) -> bool {
    fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok()).any(|e| {
                let p = e.path();
                p.is_file()
                    && p.file_name()
                        .map(|n| n.to_string_lossy().ends_with(".schema.yaml"))
                        .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

/// 从本地 ZIP 文件提取配置并返回配置目录路径
fn get_config_from_local(
    local_path: &str,
    output_dir: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    info!("尝试从本地路径获取配置文件: {}", local_path);

    // 确保输出目录存在
    if !Path::new(output_dir).exists() {
        fs::create_dir_all(output_dir)?;
    }

    // 清空目录内容（可选）
    for entry in fs::read_dir(output_dir)? {
        let path = entry?.path();
        if path.is_dir() {
            fs::remove_dir_all(&path)?;
        } else {
            fs::remove_file(&path)?;
        }
    }

    info!("开始解压文件到目录: {}", output_dir);
    // 获取操作系统类型
    let target_os = env::consts::OS;
    // 根据操作系统选择解压工具
    let output;
    if target_os == "macos" {
        info!("使用 unzip 解压");
        output = Command::new("unzip")
            .arg("-o") // 覆盖已存在的文件
            .arg("-q") // 静默模式，减少输出
            .arg("-d")
            .arg(output_dir) // 指定解压目录
            .arg(local_path)
            .output()?;
    } else {
        info!("使用 7z 解压");
        let mut cmd = Command::new("7z");
        apply_locale(&mut cmd); // 使用探测到的可用 locale，避免写死不存在的 locale
        output = cmd
            .arg("x") // 解压命令
            .arg("-y") // 假设所有问题的回答都是 yes
            .arg(format!("-o{}", output_dir)) // 正确的 -o 参数格式
            .arg("-bso0") // 关闭标准输出
            .arg("-bse0") // 关闭错误输出
            .arg(local_path)
            .output()?;
    }

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        error!("解压失败，错误信息:\n{}", stderr);
        return Err("解压配置文件失败".into());
    }

    // 查找配置目录
    let config_dir = find_config_directory(output_dir)?;

    info!("找到配置目录: {}", config_dir);

    Ok(config_dir)
}

/// 获取配置文件：使用本地路径
fn get_config_files(local_path: Option<&str>) -> Result<String, Box<dyn Error>> {
    info!("获取小鹤音形配置文件……");

    // 尝试从本地获取
    if let Some(path) = local_path {
        match get_config_from_local(path, "./extracted") {
            Ok(config_dir) => return Ok(config_dir),
            Err(err) => error!("从本地获取配置文件失败: {}", err),
        }
    }

    Err("无法获取配置文件，请检查本地路径".into())
}

/// 复制配置文件到系统目录（必要时使用 sudo）
fn copy_to_system_dir_linux(
    config_dir: &str,
    rime_system_dir: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    info!("复制配置文件到目标目录: {}", rime_system_dir);

    // 检查源目录是否存在
    if !Path::new(config_dir).exists() {
        return Err(format!("配置源目录不存在: {}", config_dir).into());
    }

    let target_path = Path::new(rime_system_dir);
    // 只在目标确实不可写时才提权；指向临时目录自测时全程无需 sudo
    let use_sudo = !path_writable_or_creatable(target_path);
    if use_sudo {
        info!("目标目录当前用户不可写，将使用 sudo 提权");
    } else {
        info!("目标目录当前用户可写，无需 sudo");
    }
    ensure_sudo_available(use_sudo)?;

    let backup_dir = backup_dir_for(rime_system_dir);

    // 备份现有配置（如果存在）
    if target_path.exists() {
        info!("目标目录 {} 已存在", rime_system_dir);

        // 检查是否非空
        let is_empty = fs::read_dir(target_path)
            .map(|mut it| it.next().is_none())
            .unwrap_or(false);
        if !is_empty {
            match create_dir(&backup_dir, use_sudo) {
                Ok(()) => {
                    info!("开始备份现有配置到 {}", backup_dir);
                    if let Err(e) =
                        run_rsync(target_path.to_str().unwrap(), &backup_dir, use_sudo)
                    {
                        warn!("备份现有配置失败（不中止安装）: {}", e);
                    }
                }
                Err(e) => warn!("创建备份目录 {} 失败，跳过备份: {}", backup_dir, e),
            }
        } else {
            info!("目标目录为空，跳过备份");
        }
    } else {
        info!("目标目录 {} 不存在，将创建", rime_system_dir);
    }

    // 确保目标目录存在
    create_dir(rime_system_dir, use_sudo)?;

    // 开始复制新配置（合并写入，不删除目标中既有文件）
    info!("开始复制新配置文件到 {}", rime_system_dir);
    run_rsync(config_dir, rime_system_dir, use_sudo)?;

    // 设置正确权限
    fix_permissions(rime_system_dir, use_sudo)?;

    info!("✅ 配置文件已成功复制到目标目录");

    Ok(())
}

/// 创建目录；use_sudo 为真时通过 sudo mkdir -p
fn create_dir(dir: &str, use_sudo: bool) -> Result<(), Box<dyn std::error::Error>> {
    if use_sudo {
        let status = command_with_sudo("mkdir", true)
            .arg("-p")
            .arg(dir)
            .status()?;
        if !status.success() {
            return Err(format!("创建目录失败: {}", dir).into());
        }
    } else {
        fs::create_dir_all(dir)?;
    }
    Ok(())
}

/// 用 rsync 把 src 的内容合并进 dest。
///
/// 刻意不使用 --delete：目标目录里既有文件一律保留，避免误删系统配置。
/// locale 使用探测到的可用值；--iconv 默认不加，仅在显式设置
/// FLYRS_RSYNC_ICONV 时才启用。
fn run_rsync(src: &str, dest: &str, use_sudo: bool) -> Result<(), Box<dyn std::error::Error>> {
    info!(
        "复制文件从 {} 到 {}{}",
        src,
        dest,
        if use_sudo { "（sudo）" } else { "" }
    );

    let mut cmd = command_with_sudo("rsync", use_sudo);
    apply_locale(&mut cmd);
    cmd.arg("-a"); // 存档模式，保留所有属性

    if let Ok(v) = env::var(ENV_RSYNC_ICONV) {
        if !v.trim().is_empty() {
            info!("启用 rsync --iconv={}", v.trim());
            cmd.arg(format!("--iconv={}", v.trim()));
        }
    }

    cmd.arg(format!("{}/", src)) // 结尾斜杠表示复制内容而非目录本身
        .arg(format!("{}/", dest));

    let output = cmd.output()?;

    if !output.status.success() {
        error!(
            "rsync 失败 stdout: {:?}",
            String::from_utf8_lossy(&output.stdout)
        );
        error!(
            "rsync 错误 stderr: {:?}",
            String::from_utf8_lossy(&output.stderr)
        );
        return Err("rsync 失败".into());
    }

    Ok(())
}

/// 设置文件和目录权限
fn fix_permissions(
    rime_system_dir: &str,
    use_sudo: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    info!("修复目标目录权限: {}", rime_system_dir);

    // 使用 sudo 命令设置目录权限
    let status = command_with_sudo("find", use_sudo)
        .arg(rime_system_dir)
        .args(["-type", "d", "-exec", "chmod", "755", "{}", ";"])
        .status()?;

    if !status.success() {
        return Err("设置目录权限失败".into());
    }

    // 使用 sudo 命令设置文件权限
    let status = command_with_sudo("find", use_sudo)
        .arg(rime_system_dir)
        .args(["-type", "f", "-exec", "chmod", "644", "{}", ";"])
        .status()?;

    if !status.success() {
        return Err("设置文件权限失败".into());
    }

    // 特殊处理.bin文件
    let status = command_with_sudo("find", use_sudo)
        .arg(rime_system_dir)
        .args(["-name", "*.bin", "-exec", "chmod", "755", "{}", ";"])
        .status()?;

    if !status.success() {
        warn!("未能设置.bin文件的执行权限");
    }

    info!("权限修复完成");
    Ok(())
}

/// macOS 专用：复制配置文件到用户目录
fn copy_to_user_dir_macos(
    config_dir: &str,
    user_dir: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    info!("复制配置文件到用户目录: {}", user_dir);

    // 确保目标目录存在
    fs::create_dir_all(user_dir)?;

    // 创建备份目录
    let backup_dir = format!(
        "{}/backup-{}",
        user_dir,
        Local::now().format("%Y%m%d_%H%M%S")
    );

    // 备份现有配置（如果存在）
    if Path::new(user_dir).exists() {
        let is_empty = fs::read_dir(user_dir)?.next().is_none();
        if !is_empty {
            info!("备份现有配置到: {}", backup_dir);
            fs::create_dir_all(&backup_dir)?;
            run_copy_cmd(user_dir, &backup_dir)?;
        }
    }

    // 复制新配置
    info!("复制新配置文件到: {}", user_dir);
    run_copy_cmd(config_dir, user_dir)?;

    info!("✅ 配置文件已成功复制到用户目录");
    Ok(())
}

/// 苹果系统通用复制命令（不需要 sudo）
fn run_copy_cmd(src: &str, dest: &str) -> Result<(), Box<dyn std::error::Error>> {
    info!("复制文件从 {} 到 {}", src, dest);

    // 使用 cp 命令进行复制，使用 -R 选项来递归复制目录
    let status = Command::new("cp")
        .arg("-R") // 递归复制
        .arg(src) // 源目录
        .arg(dest) // 目标目录
        .status()?;

    if !status.success() {
        error!("cp 复制失败");
        return Err("文件复制失败".into());
    }

    Ok(())
}
