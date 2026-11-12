//! The data directory lock, across threads and real processes.

use bard_core::DataDirLock;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Barrier};

const CHILD_DIR: &str = "BARD_LOCK_TEST_DIR";

/// Takes the lock once it is free. Other tests here spawn processes, and a
/// child holds a copy of every open file between fork and exec, which can
/// keep a just-released lock held for that instant.
fn acquire_once_free(dir: &Path) -> Option<DataDirLock> {
    let end = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        if let Some(l) = DataDirLock::acquire(dir).unwrap() {
            return Some(l);
        }
        if std::time::Instant::now() > end {
            return None;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// Run as a child process (see `spawn`): waits for "go" on stdin, tries the
/// lock, prints `lock:got` or `lock:busy`, then holds it until stdin closes.
/// Does nothing in an ordinary test run.
#[test]
fn child_process_helper() {
    let Some(dir) = std::env::var_os(CHILD_DIR) else { return };
    let mut line = String::new();
    let stdin = std::io::stdin();
    stdin.lock().read_line(&mut line).unwrap();
    let lock = DataDirLock::acquire(Path::new(&dir)).unwrap();
    println!("lock:{}", if lock.is_some() { "got" } else { "busy" });
    std::io::stdout().flush().unwrap();
    line.clear();
    let _ = stdin.lock().read_line(&mut line); // EOF, or killed
    drop(lock);
}

struct Kid {
    child: Child,
    out: BufReader<std::process::ChildStdout>,
}

impl Kid {
    fn spawn(dir: &Path) -> Kid {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "child_process_helper", "--nocapture", "--test-threads=1"])
            .env(CHILD_DIR, dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let out = BufReader::new(child.stdout.take().unwrap());
        Kid { child, out }
    }
    fn go(&mut self) {
        writeln!(self.child.stdin.as_mut().unwrap(), "go").unwrap();
    }
    /// The child's answer.
    fn result(&mut self) -> String {
        let mut line = String::new();
        loop {
            line.clear();
            assert!(self.out.read_line(&mut line).unwrap() > 0, "child exited without answering");
            // The harness prints the test's name on the same line.
            if let Some(at) = line.find("lock:") {
                return line[at + 5..].split_whitespace().next().unwrap_or_default().to_string();
            }
        }
    }
    fn release(mut self) {
        drop(self.child.stdin.take());
        self.child.wait().unwrap();
    }
}

#[test]
fn of_simultaneous_threads_exactly_one_gets_the_lock() {
    let dir = tempfile::tempdir().unwrap();
    let n = 8;
    let barrier = Arc::new(Barrier::new(n));
    let path = Arc::new(dir.path().to_path_buf());
    let handles: Vec<_> = (0..n)
        .map(|_| {
            let (barrier, path) = (barrier.clone(), path.clone());
            std::thread::spawn(move || {
                barrier.wait();
                let lock = DataDirLock::acquire(&path).unwrap();
                // Everyone has tried before anyone lets go.
                barrier.wait();
                lock.is_some()
            })
        })
        .collect();
    let got = handles.into_iter().map(|h| h.join().unwrap()).filter(|g| *g).count();
    assert_eq!(got, 1);
}

#[test]
fn of_simultaneous_processes_exactly_one_becomes_the_writer() {
    let dir = tempfile::tempdir().unwrap();
    let mut kids: Vec<Kid> = (0..5).map(|_| Kid::spawn(dir.path())).collect();
    // All started and waiting; release them together.
    for k in &mut kids {
        k.go();
    }
    let results: Vec<String> = kids.iter_mut().map(|k| k.result()).collect();
    assert_eq!(results.iter().filter(|r| *r == "got").count(), 1, "{results:?}");
    assert_eq!(results.iter().filter(|r| *r == "busy").count(), 4, "{results:?}");
    // And this process cannot have it either while the winner runs.
    assert!(DataDirLock::acquire(dir.path()).unwrap().is_none());
    for k in kids {
        k.release();
    }
    assert!(acquire_once_free(dir.path()).is_some(), "free once they exit");
}

#[test]
fn a_crashed_holder_leaves_no_stale_lock() {
    let dir = tempfile::tempdir().unwrap();
    let mut k = Kid::spawn(dir.path());
    k.go();
    assert_eq!(k.result(), "got");
    assert!(DataDirLock::acquire(dir.path()).unwrap().is_none());
    k.child.kill().unwrap(); // SIGKILL: no cleanup runs
    k.child.wait().unwrap();
    assert!(dir.path().join("bard.lock").exists(), "the file stays; the lock does not");
    assert!(acquire_once_free(dir.path()).is_some());
}

#[test]
fn aliases_of_one_directory_are_one_lock() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("bard");
    std::fs::create_dir(&data).unwrap();
    std::fs::create_dir(data.join("sub")).unwrap();
    let link = dir.path().join("link");
    std::os::unix::fs::symlink(&data, &link).unwrap();

    let held = acquire_once_free(&data).expect("first");
    assert!(DataDirLock::acquire(&link).unwrap().is_none(), "through a symlink");
    assert!(DataDirLock::acquire(&data.join("sub/..")).unwrap().is_none(), "through ..");
    assert!(DataDirLock::acquire(&data.join(".")).unwrap().is_none(), "through .");
    drop(held);
    assert!(acquire_once_free(&link).is_some(), "released on drop");
}

#[test]
fn different_directories_do_not_block_each_other() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let _la = acquire_once_free(a.path()).expect("a");
    assert!(acquire_once_free(b.path()).is_some());
}

#[test]
fn a_missing_directory_is_created() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("new/bard");
    let l = acquire_once_free(&data).expect("lock");
    assert_eq!(l.path(), data.join("bard.lock"));
}
