//! Tests config files that have sometimes caused issues despite being valid.

use std::{io::Read, thread, time::Duration};

use crate::util::spawn_btm_in_pty;

/// Convert a reader implementing [`Read`] into a string.
fn reader_to_string(mut reader: Box<dyn Read>) -> String {
    let mut buf = String::default();
    reader.read_to_string(&mut buf).unwrap();

    buf
}

/// Run bottom with the following args.
fn run_and_kill(args: &[&str]) {
    let (master, mut handle) = spawn_btm_in_pty(args);
    let reader = master.try_clone_reader().unwrap();
    let _ = master.take_writer().unwrap();

    const TIMES_CHECKED: u64 = 6; // Check 6 times, once every 500ms, for 3 seconds total.

    for _ in 0..TIMES_CHECKED {
        thread::sleep(Duration::from_millis(500));
        match handle.try_wait() {
            Ok(Some(exit)) => {
                println!("output: {}", reader_to_string(reader));
                panic!("program terminated unexpectedly (exit status: {exit:?})");
            }
            Err(e) => {
                println!("output: {}", reader_to_string(reader));
                panic!("error while trying to wait: {e}")
            }
            _ => {}
        }
    }

    handle.kill().unwrap();
}

// /// Run the binary with the config path only if the path exists.
// fn run_if_exists(config_path: &str) {
//     if Path::new(config_path).exists() {
//         run_and_kill(&["-C", config_path]);
//     }
// }

#[test]
fn test_basic() {
    run_and_kill(&[]);
}

/// A test to ensure that a bad config will fail the `run_and_kill` function.
#[test]
#[should_panic]
fn test_bad_basic() {
    run_and_kill(&["--this_does_not_exist"]);
}

#[test]
fn test_empty() {
    run_and_kill(&["-C", "./tests/valid_configs/empty_config.toml"]);
}

#[test]
fn test_many_proc() {
    run_and_kill(&["-C", "./tests/valid_configs/many_proc.toml"]);
}

#[test]
fn test_all_proc() {
    run_and_kill(&["-C", "./tests/valid_configs/all_proc.toml"]);
}

#[test]
fn test_cpu_doughnut() {
    run_and_kill(&["-C", "./tests/valid_configs/cpu_doughnut.toml"]);
}
