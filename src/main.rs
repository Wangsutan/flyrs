use chrono::Local;
use log::{LevelFilter, error, info, warn};
use log4rs::{
    append::file::FileAppender,
    config::{Appender, Config, Root},
    encode::pattern::PatternEncoder,
};
use std::error::Error;
use std::fs;
use std::path::Path;
use std::{
    io,
    process::{Command, Stdio},
};

const RIME_SYSTEM_DIR: &str = "/usr/share/rime-data";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 初始化日志系统
    init_logger()?;

    info!("===== 开始安装小鹤音形输入法 =====");
    info!("时间: {}", Local::now().format("%Y-%m-%d %H:%M:%S"));

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

    let dependencies = ["curl", "7z", "rsync"];

    // 检查并安装依赖
    check_and_install_dependencies(&package_managers, &dependencies)?;

    // 1. 获取配置文件
    let config_dir = match get_config_files(Some("./小鹤音形“鼠须管”for macOS.zip")) {
        Ok(dir) => dir,
        Err(e) => {
            error!("获取配置文件失败: {}", e);
            return Err(e);
        }
    };

    // 2. 复制文件到系统目录 (需要 sudo)
    info!("\n需要管理员权限来复制文件到系统目录");
    info!("请在提示时输入您的密码");
    if let Err(e) = copy_to_system_dir(&config_dir, RIME_SYSTEM_DIR) {
        error!("复制配置文件失败: {}", e);
        return Err(e);
    }

    info!("\n✅ 安装完成！请重新部署 Rime 输入法");
    info!("在任务栏右键点击输入法图标 -> 选择【重新部署】");
    info!("然后按 Ctrl + \\ 或 F4 切换到小鹤音形");

    Ok(())
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
        let status = Command::new("which")
            .arg(dep)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()?;

        if status.success() {
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

/// 查找解压后的配置目录（支持多种策略）
fn find_config_directory(extract_dir: &str) -> Result<String, Box<dyn std::error::Error>> {
    // 策略一：查找第一个子目录
    for entry in fs::read_dir(extract_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            return Ok(path.to_str().unwrap().to_string());
        }
    }

    // 策略二：如果没有子目录，但有文件，说明是平铺结构，直接使用当前目录
    if fs::read_dir(extract_dir)?.next().is_some() {
        info!("ZIP 解压后未找到目录，使用根目录作为配置目录");
        return Ok(extract_dir.to_string());
    }

    error!("解压后未找到配置文件目录或文件");
    Err("解压后未找到配置文件目录".into())
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

    // 执行解压命令
    info!("开始解压文件到目录: {}", output_dir);
    let output = Command::new("7z")
        .env("LANG", "C.UTF-8") // 使用通用的 C.UTF-8 替代
        .arg("x") // 解压命令
        .arg("-y") // 假设所有问题的回答都是 yes
        .arg(format!("-o{}", output_dir)) // 正确的 -o 参数格式
        .arg("-bso0") // 关闭标准输出
        .arg("-bse0") // 关闭错误输出
        .arg(local_path)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        error!("解压失败，错误信息:\n{}", stderr);
        return Err("解压配置文件失败".into());
    }

    // 查找配置目录
    let config_dir = find_config_directory(output_dir)?;

    // rename_files_to_utf8(Path::new(&config_dir))?;

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

/// 复制配置文件到系统目录 (需要sudo权限)
fn copy_to_system_dir(
    config_dir: &str,
    rime_system_dir: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    info!("复制配置文件到系统目录: {}", rime_system_dir);

    // 检查源目录是否存在
    if !Path::new(config_dir).exists() {
        return Err(format!("配置源目录不存在: {}", config_dir).into());
    }

    let target_path = Path::new(rime_system_dir);
    let backup_dir = format!(
        "/usr/share/rime-backup-{}",
        Local::now().format("%Y%m%d_%H%M%S")
    );

    // 备份现有配置（如果存在）
    if target_path.exists() {
        info!("目标目录 {} 已存在", rime_system_dir);

        // 创建备份目录
        create_dir_with_sudo(&backup_dir)?;

        // 检查是否非空
        let is_empty = fs::read_dir(target_path)?.next().is_none();
        if !is_empty {
            info!("开始备份现有配置到 {}", backup_dir);
            run_rsync_with_sudo(target_path.to_str().unwrap(), &backup_dir)?;
        } else {
            info!("目标目录为空，跳过备份");
        }
    } else {
        info!("目标目录 {} 不存在，将创建", rime_system_dir);
    }

    // 确保目标目录存在
    create_dir_with_sudo(rime_system_dir)?;

    // 开始复制新配置
    info!("开始复制新配置文件到 {}", rime_system_dir);
    run_rsync_with_sudo(config_dir, rime_system_dir)?;

    // 设置正确权限
    fix_permissions(rime_system_dir)?;

    info!("✅ 配置文件已成功复制到系统目录");

    Ok(())
}

// 封装创建目录逻辑
fn create_dir_with_sudo(dir: &str) -> Result<(), Box<dyn std::error::Error>> {
    let status = Command::new("sudo")
        .arg("mkdir")
        .arg("-p")
        .arg(dir)
        .status()?;
    if !status.success() {
        return Err(format!("创建目录失败: {}", dir).into());
    }
    Ok(())
}

fn run_rsync_with_sudo(src: &str, dest: &str) -> Result<(), Box<dyn std::error::Error>> {
    info!("复制文件从 {} 到 {}", src, dest);

    // 设置正确的语言环境
    let mut cmd = Command::new("sudo");
    cmd.env("LANG", "zh_CN.UTF-8")
        .env("LC_ALL", "zh_CN.UTF-8")
        .arg("rsync")
        .arg("-a") // 存档模式，保留所有属性
        .arg("--iconv=UTF-8,UTF-8") // 确保编码转换正确
        .arg("--delete") // 删除目标中多余文件，保持一致性
        .arg(format!("{}/", src)) // 结尾斜杠表示复制内容而非目录本身
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
fn fix_permissions(rime_system_dir: &str) -> Result<(), Box<dyn std::error::Error>> {
    info!("修复系统目录权限: {}", rime_system_dir);

    // 使用sudo命令设置目录权限
    let status = Command::new("sudo")
        .arg("find")
        .arg(rime_system_dir)
        .arg("-type")
        .arg("d")
        .arg("-exec")
        .arg("chmod")
        .arg("755")
        .arg("{}")
        .arg(";")
        .status()?;

    if !status.success() {
        return Err("设置目录权限失败".into());
    }

    // 使用sudo命令设置文件权限
    let status = Command::new("sudo")
        .arg("find")
        .arg(rime_system_dir)
        .arg("-type")
        .arg("f")
        .arg("-exec")
        .arg("chmod")
        .arg("644")
        .arg("{}")
        .arg(";")
        .status()?;

    if !status.success() {
        return Err("设置文件权限失败".into());
    }

    // 特殊处理.bin文件
    let status = Command::new("sudo")
        .arg("find")
        .arg(rime_system_dir)
        .arg("-name")
        .arg("*.bin")
        .arg("-exec")
        .arg("chmod")
        .arg("755")
        .arg("{}")
        .arg(";")
        .status()?;

    if !status.success() {
        warn!("未能设置.bin文件的执行权限");
    }

    info!("权限修复完成");
    Ok(())
}
