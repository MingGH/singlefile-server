//! 子进程执行与回收。
//!
//! singlefile-server 在容器里是 PID 1，Linux 内核会把 single-file 派生的
//! chromium / chrome_crashpad 这类孤儿进程 re-parent 给它。Rust 的
//! `Command::output()` 只回收自己的直接子进程，被收养的孤儿退出后无人
//! waitpid，会永久堆积成僵尸（Z 态，占用 PID 且无法 kill）。
//!
//! 本模块用一个独立线程作为唯一的 waitpid 调用方：所有已退出的子进程
//! 统一在此回收，退出状态经共享表交给等待者，孤儿的状态直接丢弃。
//!
//! 注意：本进程只能有这一个 waitpid 调用方。若将来改用 `tokio::process`，
//! tokio 的 SIGCHLD driver 会抢走子进程，需要重新设计。

use std::collections::HashMap;
use std::io::{self, Read};
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

/// 无人认领的退出状态在表中保留的时长，超过即丢弃，避免表无限增长。
const ORPHAN_TTL: Duration = Duration::from_secs(60);

/// 子进程的退出方式。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ChildExit {
    /// 正常退出，携带退出码
    Code(i32),
    /// 被信号终止，携带信号编号
    Signal(i32),
}

type Exits = Mutex<HashMap<i32, (ChildExit, Instant)>>;

fn exits() -> &'static Exits {
    static EXITS: OnceLock<Exits> = OnceLock::new();
    EXITS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 启动回收线程。进程内只需调用一次，且必须在派生任何子进程之前调用。
pub fn spawn_reaper() {
    static STARTED: OnceLock<()> = OnceLock::new();
    STARTED.get_or_init(|| {
        thread::spawn(|| loop {
            if reap_one().is_none() {
                thread::sleep(Duration::from_millis(100));
            }
            prune();
        });
    });
}

/// 回收一个已退出的子进程。
fn reap_one() -> Option<(i32, ChildExit)> {
    let mut status: libc::c_int = 0;
    // SAFETY: status 指向合法的 c_int；WNOHANG 保证不阻塞
    let pid = unsafe { libc::waitpid(-1, &mut status, libc::WNOHANG) };
    if pid <= 0 {
        return None;
    }

    let exit = decode(status);
    exits().lock().unwrap().insert(pid, (exit, Instant::now()));
    Some((pid, exit))
}

/// 丢弃无人认领且已过期的退出状态，避免表无限增长。
fn prune() {
    exits()
        .lock()
        .unwrap()
        .retain(|_, (_, at)| at.elapsed() < ORPHAN_TTL);
}

/// 解析 waitpid 的状态字。
fn decode(status: libc::c_int) -> ChildExit {
    if libc::WIFEXITED(status) {
        ChildExit::Code(libc::WEXITSTATUS(status))
    } else if libc::WIFSIGNALED(status) {
        ChildExit::Signal(libc::WTERMSIG(status))
    } else {
        ChildExit::Signal(0)
    }
}

/// 等待指定子进程退出，超时返回 None。
pub fn wait_for(pid: i32, timeout: Duration) -> Option<ChildExit> {
    let deadline = Instant::now() + timeout;
    loop {
        {
            let mut table = exits().lock().unwrap();
            if let Some((exit, _)) = table.remove(&pid) {
                return Some(exit);
            }
        }
        if Instant::now() >= deadline {
            return None;
        }
        thread::sleep(Duration::from_millis(20));
    }
}

/// 子进程的执行结果。
pub struct Captured {
    pub exit: ChildExit,
    pub stderr: Vec<u8>,
}

/// 执行子进程并收集其输出。
///
/// 子进程被放进独立进程组，超时时整组 kill，避免 chromium 等孙进程变成
/// 长期存活的孤儿进程。
pub fn run_capture(cmd: &str, args: &[&str], timeout: Duration) -> io::Result<Captured> {
    let mut child = Command::new(cmd)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()?;

    let pid = child.id() as i32;
    let stdout_pipe = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("stdout pipe missing"))?;
    let stderr_pipe = child
        .stderr
        .take()
        .ok_or_else(|| io::Error::other("stderr pipe missing"))?;

    // 先起线程抽干管道，否则子进程写满管道缓冲区后会与等待方死锁。
    // stdout 用不到，但必须读掉，否则管道写满同样会卡死子进程
    let out_reader = thread::spawn(move || read_all(stdout_pipe));
    let err_reader = thread::spawn(move || read_all(stderr_pipe));

    let exit = match wait_for(pid, timeout) {
        Some(exit) => Some(exit),
        None => {
            // 超时：连同进程组里的 chromium 一起杀掉
            // SAFETY: 子进程是自己进程组的组长，负 pid 即整个组
            unsafe { libc::kill(-pid as libc::pid_t, libc::SIGKILL) };
            let _ = wait_for(pid, Duration::from_secs(5));
            None
        }
    };

    let _ = out_reader.join();
    let stderr = err_reader.join().unwrap_or_default();

    match exit {
        Some(exit) => Ok(Captured { exit, stderr }),
        None => Err(io::Error::new(
            io::ErrorKind::TimedOut,
            format!("{} 超时 {}s", cmd, timeout.as_secs()),
        )),
    }
}

fn read_all(mut reader: impl Read) -> Vec<u8> {
    let mut buf = Vec::new();
    let _ = reader.read_to_end(&mut buf);
    buf
}
