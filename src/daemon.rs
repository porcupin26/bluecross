use std::fs;
use std::io::Write;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub fn get_log_dir() -> PathBuf {
    let xdg_state = std::env::var("XDG_STATE_HOME").unwrap_or_else(|_| {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
        format!("{}/.local/state", home)
    });
    let log_dir = PathBuf::from(xdg_state).join("bluecross").join("logs");
    let _ = fs::create_dir_all(&log_dir);
    log_dir
}

pub fn get_run_dir() -> PathBuf {
    // Prefer the per-user XDG runtime dir. When absent, fall back to a
    // UID-namespaced directory under /tmp (never the shared /tmp root, where
    // another user could plant a PID file and trick `ctl stop` into killing an
    // arbitrary process).
    let run_dir = match std::env::var("XDG_RUNTIME_DIR") {
        Ok(dir) if !dir.is_empty() => PathBuf::from(dir).join("bluecross"),
        _ => {
            let uid = unsafe { libc::getuid() };
            PathBuf::from(format!("/tmp/bluecross-{}", uid))
        }
    };
    let _ = fs::create_dir_all(&run_dir);
    // Owner-only access on the run directory.
    let _ = fs::set_permissions(
        &run_dir,
        std::os::unix::fs::PermissionsExt::from_mode(0o700),
    );
    run_dir
}

fn get_pid_file(name: &str) -> PathBuf {
    get_run_dir().join(format!("{}.pid", name))
}

pub fn write_pid_file(name: &str) -> anyhow::Result<()> {
    let pid_file = get_pid_file(name);
    fs::write(&pid_file, format!("{}", std::process::id()))?;
    Ok(())
}

pub fn read_pid(name: &str) -> Option<i32> {
    let pid_file = get_pid_file(name);
    if !pid_file.exists() {
        return None;
    }
    let content = fs::read_to_string(&pid_file).ok()?;
    let pid: i32 = content.trim().parse().ok()?;
    // Check if process is running
    if unsafe { libc::kill(pid, 0) } == 0 {
        Some(pid)
    } else {
        let _ = fs::remove_file(&pid_file);
        None
    }
}

pub fn is_running(name: &str) -> bool {
    read_pid(name).is_some()
}

pub fn stop_daemon(name: &str) -> bool {
    let pid = match read_pid(name) {
        Some(p) => p,
        None => return false,
    };

    unsafe {
        libc::kill(pid, libc::SIGTERM);
    }

    // Wait up to 5 seconds for termination
    for _ in 0..50 {
        std::thread::sleep(std::time::Duration::from_millis(100));
        if unsafe { libc::kill(pid, 0) } != 0 {
            let _ = fs::remove_file(get_pid_file(name));
            return true;
        }
    }

    // Force kill
    unsafe {
        libc::kill(pid, libc::SIGKILL);
    }
    let _ = fs::remove_file(get_pid_file(name));
    true
}

pub fn daemonize() -> anyhow::Result<()> {
    // First fork
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        anyhow::bail!("First fork failed");
    }
    if pid > 0 {
        std::process::exit(0);
    }

    // New session
    if unsafe { libc::setsid() } < 0 {
        anyhow::bail!("setsid failed");
    }

    std::env::set_current_dir("/")?;
    unsafe {
        libc::umask(0);
    }

    // Second fork
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        anyhow::bail!("Second fork failed");
    }
    if pid > 0 {
        std::process::exit(0);
    }

    // Redirect stdio to /dev/null
    let devnull = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/null")?;
    unsafe {
        libc::dup2(devnull.as_raw_fd(), libc::STDIN_FILENO);
        libc::dup2(devnull.as_raw_fd(), libc::STDOUT_FILENO);
        libc::dup2(devnull.as_raw_fd(), libc::STDERR_FILENO);
    }

    Ok(())
}

pub fn cleanup_pid(name: &str) {
    let _ = fs::remove_file(get_pid_file(name));
}

pub fn get_default_config() -> PathBuf {
    // Check current directory
    let cwd_config = Path::new("bluecross.json").to_path_buf();
    if cwd_config.exists() {
        return cwd_config;
    }

    // Check XDG_CONFIG_HOME
    let xdg_config = std::env::var("XDG_CONFIG_HOME").unwrap_or_else(|_| {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
        format!("{}/.config", home)
    });
    let config_file = PathBuf::from(xdg_config)
        .join("bluecross")
        .join("bluecross.json");
    if config_file.exists() {
        return config_file;
    }

    cwd_config
}

// --- Logging ---

struct BlueCrossLogger {
    log_file: Mutex<fs::File>,
    error_file: Mutex<fs::File>,
    foreground: bool,
    level: log::LevelFilter,
}

impl log::Log for BlueCrossLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= self.level
    }

    fn log(&self, record: &log::Record) {
        if !self.enabled(record.metadata()) {
            return;
        }

        let timestamp = format_timestamp();
        let msg = format!(
            "{} - {} - {} - {}\n",
            timestamp,
            record.target(),
            record.level(),
            record.args()
        );

        if let Ok(mut f) = self.log_file.lock() {
            let _ = f.write_all(msg.as_bytes());
            let _ = f.flush();
        }

        if record.level() <= log::Level::Error {
            if let Ok(mut f) = self.error_file.lock() {
                let _ = f.write_all(msg.as_bytes());
                let _ = f.flush();
            }
        }

        if self.foreground {
            print!("{}", msg);
        }
    }

    fn flush(&self) {
        if let Ok(mut f) = self.log_file.lock() {
            let _ = f.flush();
        }
    }
}

fn format_timestamp() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe {
        libc::localtime_r(&now, &mut tm);
    }
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec,
    )
}

pub fn setup_logging(name: &str, debug: bool, foreground: bool) -> anyhow::Result<()> {
    let log_dir = get_log_dir();
    let level = if debug {
        log::LevelFilter::Debug
    } else {
        log::LevelFilter::Info
    };

    let log_file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_dir.join(format!("{}.log", name)))?;

    let error_file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_dir.join(format!("{}.error.log", name)))?;

    let logger = BlueCrossLogger {
        log_file: Mutex::new(log_file),
        error_file: Mutex::new(error_file),
        foreground,
        level,
    };

    log::set_boxed_logger(Box::new(logger)).map_err(|e| anyhow::anyhow!("{}", e))?;
    log::set_max_level(level);

    Ok(())
}

// --- Ctl commands ---

pub fn handle_ctl(command: &str, args: &[String]) -> anyhow::Result<()> {
    match command {
        "start" => cmd_start(args),
        "stop" => cmd_stop(args),
        "restart" => cmd_restart(args),
        "status" => cmd_status(args),
        "logs" => cmd_logs(args),
        _ => {
            eprintln!("Unknown command: {}", command);
            std::process::exit(1);
        }
    }
}

fn cmd_start(args: &[String]) -> anyhow::Result<()> {
    let component = args.first().map(|s| s.as_str()).unwrap_or("server");
    if component != "server" && component != "client" {
        anyhow::bail!("Component must be 'server' or 'client'");
    }

    if is_running(component) {
        let pid = read_pid(component).unwrap_or(0);
        println!("BlueCross {} is already running (PID: {})", component, pid);
        return Ok(());
    }

    let config = get_default_config();
    let exe = std::env::current_exe()?;

    let mut cmd = std::process::Command::new(&exe);
    cmd.arg(component);
    cmd.args(["-c", config.to_str().unwrap_or("bluecross.json")]);

    // Check for flags in remaining args
    if args.iter().any(|a| a == "-f" || a == "--foreground") {
        cmd.arg("-f");
        // Foreground mode: replace current process
        use std::os::unix::process::CommandExt;
        let err = cmd.exec();
        return Err(err.into());
    }

    if args.iter().any(|a| a == "-d" || a == "--debug") {
        cmd.arg("-d");
    }

    cmd.stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    cmd.spawn()?;

    std::thread::sleep(std::time::Duration::from_millis(500));

    if is_running(component) {
        let pid = read_pid(component).unwrap_or(0);
        println!("BlueCross {} started (PID: {})", component, pid);
    } else {
        println!("Failed to start BlueCross {}", component);
        println!(
            "Check logs at: {}",
            get_log_dir().join(format!("{}.log", component)).display()
        );
    }

    Ok(())
}

fn cmd_stop(args: &[String]) -> anyhow::Result<()> {
    let component = args.first().map(|s| s.as_str()).unwrap_or("server");

    if !is_running(component) {
        println!("BlueCross {} is not running", component);
        return Ok(());
    }

    let pid = read_pid(component).unwrap_or(0);
    println!("Stopping BlueCross {} (PID: {})...", component, pid);

    if stop_daemon(component) {
        println!("BlueCross {} stopped", component);
    } else {
        println!("Failed to stop BlueCross {}", component);
    }

    Ok(())
}

fn cmd_restart(args: &[String]) -> anyhow::Result<()> {
    let component = args.first().map(|s| s.as_str()).unwrap_or("server");

    if is_running(component) {
        cmd_stop(args)?;
    }

    cmd_start(args)
}

fn cmd_status(args: &[String]) -> anyhow::Result<()> {
    let component = args.first().map(|s| s.as_str()).unwrap_or("all");

    let components: Vec<&str> = if component == "all" {
        vec!["server", "client"]
    } else {
        vec![component]
    };

    for c in components {
        if let Some(pid) = read_pid(c) {
            println!("BlueCross {}: running (PID: {})", c, pid);
        } else {
            println!("BlueCross {}: stopped", c);
        }
    }

    Ok(())
}

fn cmd_logs(args: &[String]) -> anyhow::Result<()> {
    let component = args.first().map(|s| s.as_str()).unwrap_or("server");
    let log_dir = get_log_dir();
    let is_error = args.iter().any(|a| a == "-e" || a == "--error");
    let follow = args.iter().any(|a| a == "-f" || a == "--follow");

    let log_file = if is_error {
        log_dir.join(format!("{}.error.log", component))
    } else {
        log_dir.join(format!("{}.log", component))
    };

    if !log_file.exists() {
        println!("No logs found at {}", log_file.display());
        return Ok(());
    }

    let lines = args
        .iter()
        .position(|a| a == "-n" || a == "--lines")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(50);

    if follow {
        use std::os::unix::process::CommandExt;
        let err = std::process::Command::new("tail")
            .args(["-f", log_file.to_str().unwrap()])
            .exec();
        return Err(err.into());
    }

    use std::os::unix::process::CommandExt;
    let err = std::process::Command::new("tail")
        .args(["-n", &lines.to_string(), log_file.to_str().unwrap()])
        .exec();
    Err(err.into())
}
