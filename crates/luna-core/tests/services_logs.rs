//! What a service prints is kept, rotated, shown live and sent in throttled batches (FR-SV-05, D-025). Real
//! processes; every data folder has a space and a non-ASCII letter in its path (NFR-07). The stand-in service is in
//! `stub_service/`. Its own startup chatter (the test harness prints "running 1 test") is in the output too, so the
//! tests look for their own lines by prefix.

// Test code: a panic with a message is the desired failure mode.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod stub_service;

use std::fs;
use std::io::Write;
use std::path::Path;
use std::time::{Duration, Instant};

use luna_core::ErrorCode;
use luna_core::services::{LogConfig, LogFile, OUTPUT, ReadyCheck, ServiceState};
use stub_service::*;

/// Fast reading, a quick "this line is not going to get an ending", and everything else as shipped.
fn quick() -> LogConfig {
    LogConfig {
        interval: Duration::from_millis(20),
        partial_after: Duration::from_millis(150),
        ..LogConfig::default()
    }
}

/// The lines of a source that start with `prefix`.
fn starting_with(lines: Vec<luna_core::services::LogLine>, prefix: &str) -> Vec<String> {
    lines
        .into_iter()
        .map(|l| l.line)
        .filter(|line| line.starts_with(prefix))
        .collect()
}

/// Wait until `source` has a line that starts with `prefix`.
async fn line_appears(
    supervisor: &luna_core::services::Supervisor,
    id: &str,
    source: &str,
    prefix: &str,
) -> Vec<String> {
    for _ in 0..800 {
        let lines = starting_with(supervisor.log_lines(id, source, 5_000), prefix);
        if !lines.is_empty() {
            return lines;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("no line starting with {prefix:?} ever showed up in {source}");
}

fn log_file(bench: &Shared, id: &str) -> std::path::PathBuf {
    bench.home.logs_dir().join(format!("{id}.log"))
}

fn names_in_logs(bench: &Shared) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(bench.home.logs_dir())
        .map(|dir| {
            dir.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

// ---------------------------------------------------------------------------------------------
// Capturing
// ---------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn what_a_service_prints_is_kept_in_the_order_it_was_written_with_both_streams_together() {
    let bench = private("logs-output");
    let (supervisor, _events, _batches) = bench.supervisor_with_logs(quick());
    supervisor
        .start(bench.serving(
            "web",
            free_port(),
            &[("LUNA_STUB_OUT_COUNT", "3"), ("LUNA_STUB_ERR_COUNT", "3")],
        ))
        .unwrap();
    settled(&supervisor, "web").await;
    line_appears(&supervisor, "web", OUTPUT, "err 2").await;

    let lines = supervisor.log_lines("web", OUTPUT, 5_000);
    let mine: Vec<&str> = lines
        .iter()
        .map(|l| l.line.as_str())
        .filter(|l| l.starts_with("out ") || l.starts_with("err "))
        .collect();
    assert_eq!(mine, ["out 0", "err 0", "out 1", "err 1", "out 2", "err 2"]);
    assert!(lines.iter().all(|l| l.source == OUTPUT));
    assert!(
        lines.iter().all(|l| l.ts > 1_700_000_000_000),
        "milliseconds since 1970"
    );
    let position = |text: &str| lines.iter().position(|l| l.line == text);
    assert!(
        position("[lunaRun] starting").unwrap() < position("out 0").unwrap(),
        "the log says when it started, before anything the service prints"
    );

    supervisor.stop("web").await.unwrap();
    let after = supervisor.log_lines("web", OUTPUT, 5_000);
    assert_eq!(
        after.last().unwrap().line,
        "[lunaRun] stopped",
        "and when it was stopped"
    );
    // The same lines are in the file, and the log folder was there to hold it.
    let text = fs::read_to_string(log_file(&bench, "web")).unwrap();
    assert!(text.contains("out 0\n") || text.contains("out 0\r\n"));
    assert!(text.trim_end().ends_with("[lunaRun] stopped"));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_log_folder_is_created_when_it_is_missing() {
    let bench = private("logs-folder");
    fs::remove_dir_all(bench.home.logs_dir()).unwrap();
    let (supervisor, _events, _batches) = bench.supervisor_with_logs(quick());

    supervisor
        .start(bench.serving("web", free_port(), &[("LUNA_STUB_OUT_COUNT", "1")]))
        .unwrap();
    settled(&supervisor, "web").await;
    line_appears(&supervisor, "web", OUTPUT, "out 0").await;

    assert!(log_file(&bench, "web").exists());
    supervisor.stop("web").await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn the_ring_keeps_only_the_last_lines() {
    let bench = private("logs-ring");
    let config = LogConfig {
        ring_lines: 50,
        ..quick()
    };
    let (supervisor, _events, _batches) = bench.supervisor_with_logs(config);
    supervisor
        .start(bench.serving("web", free_port(), &[("LUNA_STUB_OUT_COUNT", "500")]))
        .unwrap();
    settled(&supervisor, "web").await;
    line_appears(&supervisor, "web", OUTPUT, "out 499").await;

    let all = supervisor.log_lines("web", OUTPUT, 10_000);
    assert_eq!(all.len(), 50, "the cap");
    let numbered = starting_with(all, "out ");
    assert_eq!(numbered.last().unwrap(), "out 499");
    assert!(
        numbered.len() >= 40,
        "the last of the 500 lines, not the first: {}",
        numbered.len()
    );
    assert!(!numbered.contains(&"out 0".to_owned()));
    assert_eq!(
        supervisor.log_lines("web", OUTPUT, 3).len(),
        3,
        "the viewer can ask for fewer"
    );
    supervisor.stop("web").await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn odd_output_never_breaks_the_log() {
    let bench = private("logs-odd");
    let (supervisor, _events, _batches) = bench.supervisor_with_logs(quick());
    supervisor
        .start(bench.serving(
            "odd",
            free_port(),
            &[
                // "ok" and a Windows line ending, then two bytes that are not UTF-8 and a line ending.
                ("LUNA_STUB_RAW_HEX", "6f 6b 0d 0a ff fe 0a"),
                ("LUNA_STUB_LONG", "100000"),
            ],
        ))
        .unwrap();
    settled(&supervisor, "odd").await;
    line_appears(&supervisor, "odd", OUTPUT, "after long").await;

    let lines: Vec<String> = supervisor
        .log_lines("odd", OUTPUT, 5_000)
        .into_iter()
        .map(|l| l.line)
        .collect();
    assert!(
        lines.contains(&"ok".to_owned()),
        "the carriage return is not part of the line"
    );
    assert!(
        lines.contains(&"\u{fffd}\u{fffd}".to_owned()),
        "bytes that are not text are shown as such"
    );
    let long = lines.iter().find(|l| l.starts_with("xxxx")).unwrap();
    assert!(
        long.ends_with("...[line cut]") && long.len() < 17_000,
        "{}",
        long.len()
    );
    assert!(
        !lines.iter().any(|l| l == "x"),
        "the rest of the long line was dropped, not shown as more lines"
    );
    supervisor.stop("odd").await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_last_line_that_never_gets_its_ending_is_shown() {
    let bench = private("logs-partial");
    let (supervisor, _events, _batches) = bench.supervisor_with_logs(quick());
    supervisor
        .start(bench.serving("web", free_port(), &[("LUNA_STUB_NO_NEWLINE", "waiting> ")]))
        .unwrap();
    settled(&supervisor, "web").await;

    let seen = line_appears(&supervisor, "web", OUTPUT, "waiting>").await;

    assert_eq!(seen, ["waiting> "]);
    supervisor.stop("web").await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn the_last_words_are_there_when_the_stop_returns() {
    let bench = private("logs-last-words");
    let (supervisor, _events, _batches) = bench.supervisor_with_logs(quick());
    let dir = bench.scratch("t");
    let stop_file = dir.join("stop");
    let mut spec = bench.serving(
        "web",
        free_port(),
        &[("LUNA_STUB_STOPFILE", stop_file.to_str().unwrap())],
    );
    spec.stop.method = luna_core::services::StopMethod::Command(
        bench.helper("touch", &[("LUNA_STUB_FILE", stop_file.to_str().unwrap())]),
    );
    supervisor.start(spec).unwrap();
    settled(&supervisor, "web").await;

    supervisor.stop("web").await.unwrap();

    // Right now, without waiting for another tick: it said goodbye and lunaRun noted that it stopped.
    let lines: Vec<String> = supervisor
        .log_lines("web", OUTPUT, 5_000)
        .into_iter()
        .map(|l| l.line)
        .collect();
    let tail: Vec<&str> = lines.iter().rev().take(2).map(String::as_str).collect();
    assert_eq!(tail, ["[lunaRun] stopped", "goodbye"]);
}

// ---------------------------------------------------------------------------------------------
// Sending
// ---------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn events_come_in_batches_never_more_than_ten_a_second() {
    let bench = private("logs-batches");
    // The shipped interval: ten reads a second.
    let config = LogConfig {
        partial_after: Duration::from_millis(150),
        ..LogConfig::default()
    };
    let (supervisor, _events, batches) = bench.supervisor_with_logs(config);
    supervisor
        .start(bench.serving(
            "web",
            free_port(),
            &[("LUNA_STUB_FLOOD", "20000"), ("LUNA_STUB_CHATTER_MS", "30")],
        ))
        .unwrap();
    settled(&supervisor, "web").await;
    tokio::time::sleep(Duration::from_millis(1_500)).await;
    // Before stopping: the stop itself delivers one last batch on request.
    let during: Vec<(Instant, usize)> = batches
        .lock()
        .unwrap()
        .iter()
        .map(|(at, batch)| (*at, batch.lines.len()))
        .collect();
    supervisor.stop("web").await.unwrap();

    assert!(!during.is_empty());
    for pair in during.windows(2) {
        let gap = pair[1].0.duration_since(pair[0].0);
        assert!(
            gap >= Duration::from_millis(60),
            "two batches {gap:?} apart: more than ten a second"
        );
    }
    let span = during.last().unwrap().0.duration_since(during[0].0);
    assert!(
        during.len() as u128 <= span.as_millis() / 90 + 2,
        "{} batches in {span:?}",
        during.len()
    );
    // All of it arrived, in order, in a few big batches rather than in one event per line.
    let all = batches.lock().unwrap();
    let flood: Vec<&str> = all
        .iter()
        .flat_map(|(_, b)| b.lines.iter().map(|l| l.line.as_str()))
        .filter(|l| l.starts_with("flood "))
        .collect();
    assert_eq!(flood.len(), 20_000);
    assert!(flood.windows(2).all(|w| w[0] < w[1]), "in order");
    assert!(
        all.iter()
            .all(|(_, b)| b.id == "web" && !b.lines.is_empty())
    );
    assert!(all.len() < 500, "{} batches for 20,000 lines", all.len());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_service_that_prints_a_lot_is_never_slowed_by_a_slow_reader() {
    let bench = private("logs-slow-reader");
    let done = bench.scratch("t").join("flood-done");
    // A reader that takes a fifth of a second per batch would fill a pipe in no time and block the service. The
    // service writes to a file, so it does not even notice.
    let (supervisor, _events, _batches) = bench.supervisor_with_log_sink(quick(), |_| {
        std::thread::sleep(Duration::from_millis(200));
    });
    supervisor
        .start(bench.serving(
            "web",
            free_port(),
            &[
                ("LUNA_STUB_FLOOD", "200000"),
                ("LUNA_STUB_FLOOD_DONE", done.to_str().unwrap()),
            ],
        ))
        .unwrap();

    let begun = Instant::now();
    while !done.exists() {
        assert!(
            begun.elapsed() < Duration::from_secs(30),
            "the service was blocked by the reader"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    supervisor.stop("web").await.unwrap();
}

// ---------------------------------------------------------------------------------------------
// Files: a new run, a restart, the size limit
// ---------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn each_new_start_moves_the_old_logs_up_and_starts_an_empty_viewer() {
    let bench = private("logs-new-runs");
    let (supervisor, _events, _batches) = bench.supervisor_with_logs(quick());
    let port = free_port();

    for run in 1..=4 {
        let prefix = format!("run{run}");
        supervisor
            .start(bench.serving(
                "web",
                port,
                &[("LUNA_STUB_OUT_COUNT", "1"), ("LUNA_STUB_PREFIX", &prefix)],
            ))
            .unwrap();
        settled(&supervisor, "web").await;
        line_appears(&supervisor, "web", OUTPUT, &prefix).await;
        let earlier: Vec<String> = (1..run).map(|n| format!("run{n} 0")).collect();
        let shown: Vec<String> = supervisor
            .log_lines("web", OUTPUT, 5_000)
            .into_iter()
            .map(|l| l.line)
            .collect();
        assert!(
            earlier.iter().all(|old| !shown.contains(old)),
            "the viewer starts empty: {shown:?}"
        );
        supervisor.stop("web").await.unwrap();
    }

    assert_eq!(
        names_in_logs(&bench),
        ["web.log", "web.log.1", "web.log.2"],
        "three files, the oldest is gone"
    );
    let read = |name: &str| fs::read_to_string(bench.home.logs_dir().join(name)).unwrap();
    assert!(read("web.log").contains("run4 0"));
    assert!(read("web.log.1").contains("run3 0"));
    assert!(read("web.log.2").contains("run2 0"));
    assert!(!read("web.log.2").contains("run1 0"));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_automatic_restart_keeps_adding_to_the_same_log() {
    let bench = private("logs-restart");
    let (supervisor, _events, _batches) = bench.supervisor_with_logs(quick());
    let counter = bench.scratch("t").join("launches");
    // Crashes on its first launch only, and says "banner" every time it starts.
    supervisor
        .start(bench.serving(
            "web",
            free_port(),
            &[
                ("LUNA_STUB_COUNTER", counter.to_str().unwrap()),
                ("LUNA_STUB_EXIT_AFTER_MS", "200"),
                ("LUNA_STUB_LEAVE_FIRST", "1"),
                ("LUNA_STUB_EXIT_CODE", "7"),
                ("LUNA_STUB_OUT_COUNT", "1"),
                ("LUNA_STUB_PREFIX", "banner"),
            ],
        ))
        .unwrap();
    supervisor
        .wait_until("web", WAIT, |s| {
            s.restarts == 1 && s.state == ServiceState::Running
        })
        .await
        .unwrap();
    // Wait for the second banner: the first one came from the crashed run.
    for _ in 0..400 {
        if starting_with(supervisor.log_lines("web", OUTPUT, 5_000), "banner").len() == 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    let lines: Vec<String> = supervisor
        .log_lines("web", OUTPUT, 5_000)
        .into_iter()
        .map(|l| l.line)
        .filter(|l| l.starts_with("banner") || l.starts_with("[lunaRun]"))
        .collect();
    let kinds: Vec<String> = lines
        .iter()
        .map(|l| l.split(" (").next().unwrap().to_owned())
        .collect();
    assert_eq!(
        kinds,
        [
            "[lunaRun] starting",
            "banner 0",
            "[lunaRun] exited with code 7",
            "[lunaRun] starting",
            "banner 0"
        ],
        "{lines:?}"
    );
    assert_eq!(
        names_in_logs(&bench),
        ["web.log"],
        "a restart does not rotate"
    );
    supervisor.stop("web").await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_running_service_is_rotated_at_the_size_limit_and_keeps_writing() {
    let bench = private("logs-live-rotation");
    let config = LogConfig {
        max_bytes: 20_000,
        files: 3,
        ..quick()
    };
    let (supervisor, _events, _batches) = bench.supervisor_with_logs(config);
    supervisor
        .start(bench.serving(
            "web",
            free_port(),
            // 2,000 lines of about 55 bytes: far more than the limit; then a line every 30 ms.
            &[("LUNA_STUB_FLOOD", "2000"), ("LUNA_STUB_CHATTER_MS", "30")],
        ))
        .unwrap();
    settled(&supervisor, "web").await;
    // Wait until some chatter written after the flood has been seen.
    line_appears(&supervisor, "web", OUTPUT, "tick 15").await;
    let size_now = fs::metadata(log_file(&bench, "web")).unwrap().len();

    let names = names_in_logs(&bench);
    assert!(
        names.contains(&"web.log.1".to_owned()),
        "a copy was made: {names:?}"
    );
    assert!(names.len() <= 3, "never more than three files: {names:?}");
    assert!(
        size_now < 20_000 + 2_000,
        "the log does not stay over the limit: {size_now}"
    );
    // What the viewer saw was in order, with no line twice (a few can be lost in the copy, never repeated).
    let ticks: Vec<u32> = starting_with(supervisor.log_lines("web", OUTPUT, 10_000), "tick ")
        .iter()
        .map(|l| l[5..].parse().unwrap())
        .collect();
    assert!(ticks.windows(2).all(|w| w[0] < w[1]), "{ticks:?}");
    supervisor.stop("web").await.unwrap();
}

// ---------------------------------------------------------------------------------------------
// The service's own log files
// ---------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn a_services_own_log_files_are_tabs_of_their_own() {
    let bench = private("logs-own-files");
    let (supervisor, _events, batches) = bench.supervisor_with_logs(quick());
    let error_log = bench.home.logs_dir().join("db.err");
    fs::write(&error_log, "old one\nold two\n").unwrap();
    let mut spec = bench.serving("db", free_port(), &[("LUNA_STUB_OUT_COUNT", "1")]);
    spec.log_files = vec![LogFile {
        source: "MySQL error log".to_owned(),
        path: error_log.clone(),
    }];

    supervisor.start(spec).unwrap();
    settled(&supervisor, "db").await;

    assert_eq!(supervisor.log_sources("db"), ["output", "MySQL error log"]);
    let seeded: Vec<String> = supervisor
        .log_lines("db", "MySQL error log", 100)
        .into_iter()
        .map(|l| l.line)
        .collect();
    assert_eq!(
        seeded,
        ["old one", "old two"],
        "the lines it already had are there to scroll back to"
    );

    // The service writes to its own file later.
    fs::OpenOptions::new()
        .append(true)
        .open(&error_log)
        .unwrap()
        .write_all(b"late line\n")
        .unwrap();
    line_appears(&supervisor, "db", "MySQL error log", "late line").await;

    let in_event = batches
        .lock()
        .unwrap()
        .iter()
        .flat_map(|(_, b)| b.lines.iter())
        .any(|l| l.source == "MySQL error log" && l.line == "late line");
    assert!(
        in_event,
        "and it went out in an event, labelled with its source"
    );
    assert_eq!(
        fs::read_to_string(&error_log).unwrap(),
        "old one\nold two\nlate line\n",
        "lunaRun only reads it"
    );
    // The output of the service is a source of its own and is not mixed in.
    assert!(starting_with(supervisor.log_lines("db", OUTPUT, 100), "late").is_empty());
    supervisor.stop("db").await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_log_file_that_does_not_exist_yet_is_picked_up_when_it_appears() {
    let bench = private("logs-later-file");
    let (supervisor, _events, _batches) = bench.supervisor_with_logs(quick());
    let later = bench.home.logs_dir().join("caddy.access");
    let mut spec = bench.serving("caddy", free_port(), &[]);
    spec.log_files = vec![LogFile {
        source: "Access log".to_owned(),
        path: later.clone(),
    }];
    supervisor.start(spec).unwrap();
    settled(&supervisor, "caddy").await;
    assert_eq!(
        supervisor.log_sources("caddy"),
        ["output", "Access log"],
        "its tab exists before its first line"
    );
    assert!(supervisor.log_lines("caddy", "Access log", 10).is_empty());

    fs::write(&later, "GET / 200\n").unwrap();

    line_appears(&supervisor, "caddy", "Access log", "GET /").await;
    supervisor.stop("caddy").await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn an_adopted_service_keeps_its_log_and_goes_on_writing_to_it() {
    let bench = private("logs-adopt");
    let (supervisor, _events, _batches) = bench.supervisor_with_logs(quick());
    let port = free_port();
    let alive = detached(&bench, port, &[]);
    write_pid_file(&bench, "db", alive.pid, &bench.stub, Some(port));
    // What the earlier lunaRun had captured, in the file the service still has open.
    let log = log_file(&bench, "db");
    fs::write(&log, "from before the crash 1\nfrom before the crash 2\n").unwrap();

    supervisor
        .adopt(bench.serving("db", port, &[]))
        .await
        .unwrap();

    let before: Vec<String> = supervisor
        .log_lines("db", OUTPUT, 100)
        .into_iter()
        .map(|l| l.line)
        .collect();
    assert_eq!(
        before,
        ["from before the crash 1", "from before the crash 2"]
    );
    assert_eq!(
        names_in_logs(&bench),
        ["db.log"],
        "adopting does not rotate the log"
    );
    fs::OpenOptions::new()
        .append(true)
        .open(&log)
        .unwrap()
        .write_all(b"written after the adoption\n")
        .unwrap();
    line_appears(&supervisor, "db", OUTPUT, "written after").await;
    supervisor.stop("db").await.unwrap();
}

// ---------------------------------------------------------------------------------------------
// Specs
// ---------------------------------------------------------------------------------------------

#[test]
fn log_files_must_be_named_and_inside_the_data_folder() {
    let bench = private("logs-spec");
    let inside = bench.home.logs_dir().join("db.err");
    let outside = bench.home.root().parent().unwrap().join("elsewhere.log");
    let file = |source: &str, path: &Path| LogFile {
        source: source.to_owned(),
        path: path.to_path_buf(),
    };
    let code = |files: Vec<LogFile>| {
        let mut spec = bench.serving("db", 3306, &[]);
        spec.log_files = files;
        spec.validate(&bench.home).unwrap_err().code
    };
    let ok = |files: Vec<LogFile>| {
        let mut spec = bench.serving("db", 3306, &[]);
        spec.log_files = files;
        spec.validate(&bench.home).unwrap();
    };

    ok(vec![
        file("Error log", &inside),
        file("Slow queries", &bench.home.logs_dir().join("slow.log")),
    ]);
    assert_eq!(
        code(vec![file("Error log", &outside)]),
        ErrorCode::PathOutsideRoot
    );
    assert_eq!(
        code(vec![file("Error log", Path::new("relative.log"))]),
        ErrorCode::InvalidPath
    );
    let dotdot = bench
        .home
        .logs_dir()
        .join("..")
        .join("..")
        .join("elsewhere.log");
    assert_eq!(
        code(vec![file("Error log", &dotdot)]),
        ErrorCode::InvalidPath
    );
    for bad in ["", OUTPUT, "a\nb"] {
        assert_eq!(
            code(vec![file(bad, &inside)]),
            ErrorCode::Internal,
            "{bad:?}"
        );
    }
    assert_eq!(
        code(vec![
            file("Same", &inside),
            file("Same", &bench.home.logs_dir().join("other.log"))
        ]),
        ErrorCode::Internal,
        "two tabs with one name"
    );
    let too_long = "x".repeat(61);
    assert_eq!(code(vec![file(&too_long, &inside)]), ErrorCode::Internal);
    let _ = ReadyCheck::Immediately;
}

#[cfg(unix)]
#[test]
fn a_log_file_that_is_a_link_to_outside_is_refused() {
    let bench = private("logs-spec-link");
    let outside = bench.home.root().parent().unwrap().join("secret.log");
    fs::write(&outside, "secret\n").unwrap();
    let link = bench.home.logs_dir().join("sneaky.log");
    std::os::unix::fs::symlink(&outside, &link).unwrap();
    let mut spec = bench.serving("db", 3306, &[]);
    spec.log_files = vec![LogFile {
        source: "Sneaky".to_owned(),
        path: link,
    }];

    assert_eq!(
        spec.validate(&bench.home).unwrap_err().code,
        ErrorCode::PathOutsideRoot
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_crashed_services_last_words_are_there_when_the_crash_is_published() {
    let bench = private("logs-dying-words");
    // The log is read only every two seconds, so what is there at the moment `Crashed` is announced was put there by
    // the final read of the dead process's output, not by chance.
    let config = LogConfig {
        interval: Duration::from_secs(2),
        ..quick()
    };
    let (supervisor, _events, _batches) = bench.supervisor_with_logs(config);
    let mut spec = bench.serving(
        "web",
        free_port(),
        &[
            ("LUNA_STUB_OUT_COUNT", "1"),
            ("LUNA_STUB_PREFIX", "dying words"),
            ("LUNA_STUB_EXIT_AFTER_MS", "100"),
            ("LUNA_STUB_EXIT_CODE", "7"),
        ],
    );
    spec.restart.backoff = vec![Duration::from_secs(30)]; // stays crashed while we look
    supervisor.start(spec).unwrap();

    supervisor
        .wait_until("web", WAIT, |s| s.state == ServiceState::Crashed)
        .await
        .unwrap();

    let lines: Vec<String> = supervisor
        .log_lines("web", OUTPUT, 5_000)
        .into_iter()
        .map(|l| l.line)
        .collect();
    assert!(lines.contains(&"dying words 0".to_owned()), "{lines:?}");
    assert_eq!(lines.last().unwrap(), "[lunaRun] exited with code 7");
    supervisor.stop("web").await.unwrap();
}
