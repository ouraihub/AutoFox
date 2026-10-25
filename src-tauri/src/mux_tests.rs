//! Mux tests: real ffmpeg, real HTTP, no network.

use std::{collections::BTreeMap, path::PathBuf, sync::Arc};

use crate::queue::TaskSignal;

use super::publish;

use tokio::net::TcpListener;

use super::{MuxRequest, header_block, mux_streams_inner};

/// Builds a tiny real media file with ffmpeg so the mux path gets genuine
/// input. Parameterized by container because the WebM branch needs VP9/Opus:
/// MP4 codecs would prove nothing about it.
async fn sample_media(kind: &str, container: &str) -> Vec<u8> {
    let ffmpeg = crate::system::paths::bundled_ffmpeg().expect("ffmpeg path");
    let path = std::env::temp_dir().join(format!(
        "autofox-mux-src-{}-{kind}.{container}",
        uuid::Uuid::new_v4()
    ));
    let is_video = kind == "video";
    // A quarter second at 8 fps: VP9 is slow enough that a 1s 64x64 clip
    // noticeably lengthens the suite for no extra coverage.
    let source = if is_video {
        "testsrc=size=64x64:rate=8:duration=0.25"
    } else {
        "sine=frequency=440:duration=0.25"
    };
    let codec = match (container, is_video) {
        ("webm", true) => "libvpx-vp9",
        ("webm", false) => "libopus",
        (_, true) => "libx264",
        (_, false) => "aac",
    };
    let mut command = tokio::process::Command::new(ffmpeg);
    command.args([
        "-hide_banner",
        "-v",
        "error",
        "-y",
        "-f",
        "lavfi",
        "-i",
        source,
    ]);
    command.args([if is_video { "-c:v" } else { "-c:a" }, codec]);
    if codec == "libvpx-vp9" {
        // Single-threaded deadline=realtime keeps the fixture build near
        // instant; quality is irrelevant to a container assertion.
        command.args(["-deadline", "realtime", "-cpu-used", "8"]);
    }
    let status = command
        .arg(path.to_string_lossy().as_ref())
        .status()
        .await
        .expect("spawn ffmpeg");
    assert!(
        status.success(),
        "building {kind} {container} fixture failed"
    );
    let bytes = tokio::fs::read(&path).await.expect("read fixture");
    let _ = tokio::fs::remove_file(&path).await;
    bytes
}

/// Serves `payload` over loopback until `count` requests have been answered.
async fn serve(payload: Vec<u8>, count: usize) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let address = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        for _ in 0..count {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let body = payload.clone();
            tokio::spawn(async move {
                let mut request = [0_u8; 2048];
                let _ = socket.read(&mut request).await;
                // ffmpeg probes with a ranged GET, so advertising byte ranges
                // keeps it from giving up before it reads the moov atom.
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = socket.write_all(head.as_bytes()).await;
                let _ = socket.write_all(&body).await;
                let _ = socket.shutdown().await;
            });
        }
    });
    format!("http://{address}")
}

/// The temp file is `.mp4.part`, an extension ffmpeg cannot map to a container.
/// Without an explicit `-f mp4` every mux fails with "Unable to choose an
/// output format", so this asserts on the produced file, not on the arguments.
#[tokio::test]
async fn writes_a_real_mp4_through_the_part_file() {
    let video = sample_media("video", "mp4").await;
    let audio = sample_media("audio", "mp4").await;
    let video_url = format!("{}/v.mp4", serve(video, 4).await);
    let audio_url = format!("{}/a.mp4", serve(audio, 4).await);
    let out = std::env::temp_dir().join(format!("autofox-mux-{}", uuid::Uuid::new_v4()));
    let mut event_ids = Vec::new();
    let result = mux_streams_inner(
        None,
        "task-mp4",
        MuxRequest {
            video_url,
            audio_url,
            output_directory: out.to_string_lossy().into_owned(),
            file_name: "merged".to_owned(),
            container: "mp4".to_owned(),
            http_headers: Default::default(),
            duration_secs: None,
        },
        &TaskSignal::default(),
        |event| event_ids.push(event.task_id),
    )
    .await
    .expect("mux should produce an mp4");
    // Events carry the registry id, so the UI can tell concurrent muxes apart.
    assert!(
        !event_ids.is_empty(),
        "ffmpeg always writes a final progress block"
    );
    assert!(event_ids.iter().all(|id| id == "task-mp4"), "{event_ids:?}");

    let bytes = tokio::fs::read(&result.output_path).await.expect("read");
    // `ftyp` is the MP4 signature: proof the container was chosen, not guessed.
    assert_eq!(&bytes[4..8], b"ftyp", "output is not an MP4");
    assert!(result.output_path.ends_with(".mp4"));
    assert!(
        !PathBuf::from(format!("{}.part", result.output_path)).exists(),
        "temp file outlived the mux"
    );
    let _ = tokio::fs::remove_dir_all(out).await;
}

/// The WebM branch exists because VP9/AV1 in MP4 plays badly. `+faststart` is
/// an MP4-only muxer flag, so this also proves it is not passed here.
#[tokio::test]
async fn writes_a_real_webm_when_the_option_asks_for_one() {
    let video = sample_media("video", "webm").await;
    let audio = sample_media("audio", "webm").await;
    let video_url = format!("{}/v.webm", serve(video, 4).await);
    let audio_url = format!("{}/a.webm", serve(audio, 4).await);
    let out = std::env::temp_dir().join(format!("autofox-mux-{}", uuid::Uuid::new_v4()));
    let result = mux_streams_inner(
        None,
        "task-webm",
        MuxRequest {
            video_url,
            audio_url,
            output_directory: out.to_string_lossy().into_owned(),
            file_name: "merged".to_owned(),
            container: "webm".to_owned(),
            http_headers: Default::default(),
            duration_secs: None,
        },
        &TaskSignal::default(),
        |_| {},
    )
    .await
    .expect("mux should produce a webm");

    let bytes = tokio::fs::read(&result.output_path).await.expect("read");
    // 0x1A45DFA3 is the EBML magic every Matroska/WebM file starts with.
    assert_eq!(&bytes[..4], &[0x1A, 0x45, 0xDF, 0xA3], "output is not WebM");
    assert!(result.output_path.ends_with(".webm"));
    let _ = tokio::fs::remove_dir_all(out).await;
}

/// An unvalidated container would reach ffmpeg as `-f <junk>` and surface its
/// usage text instead of naming the real problem.
#[tokio::test]
async fn rejects_a_container_it_cannot_mux() {
    let out = std::env::temp_dir().join(format!("autofox-mux-{}", uuid::Uuid::new_v4()));
    let error = mux_streams_inner(
        None,
        "task-mkv",
        MuxRequest {
            video_url: "https://example.test/v".to_owned(),
            audio_url: "https://example.test/a".to_owned(),
            output_directory: out.to_string_lossy().into_owned(),
            file_name: "merged".to_owned(),
            container: "mkv".to_owned(),
            http_headers: Default::default(),
            duration_secs: None,
        },
        &TaskSignal::default(),
        |_| {},
    )
    .await
    .expect_err("mkv is not offered by the option list");
    assert_eq!(error.field, Some("container"));
    let _ = tokio::fs::remove_dir_all(out).await;
}

/// Accepts connections and never answers, so ffmpeg blocks on its first read
/// exactly like a stalled CDN: the case the 200ms cancel watch exists for.
async fn stall() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let address = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((socket, _)) = listener.accept().await {
            held.push(socket);
        }
    });
    format!("http://{address}")
}

/// Cancel must stop ffmpeg promptly, report `Cancelled` (not `Failed`), and
/// leave no `.part`. The `.part` is seeded beforehand so the cleanup assertion
/// cannot pass just because ffmpeg never got far enough to create one.
#[tokio::test]
async fn cancel_kills_ffmpeg_and_removes_the_part_file() {
    let base = stall().await;
    let out = std::env::temp_dir().join(format!("autofox-mux-{}", uuid::Uuid::new_v4()));
    tokio::fs::create_dir_all(&out).await.expect("mkdir");
    let part = out.join("merged.mp4.part");
    tokio::fs::write(&part, b"stale").await.expect("seed part");

    let cancel = Arc::new(TaskSignal::default());
    let flag = Arc::clone(&cancel);
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        assert!(flag.stop(), "a running mux accepts a stop");
    });
    let started = std::time::Instant::now();
    let error = mux_streams_inner(
        None,
        "task-cancel",
        MuxRequest {
            video_url: format!("{base}/v.mp4"),
            audio_url: format!("{base}/a.mp4"),
            output_directory: out.to_string_lossy().into_owned(),
            file_name: "merged".to_owned(),
            container: "mp4".to_owned(),
            http_headers: Default::default(),
            duration_secs: None,
        },
        &cancel,
        |_| {},
    )
    .await
    .expect_err("a cancelled mux must not succeed");
    let took = started.elapsed();

    assert_eq!(error.code, crate::ipc::ErrorCode::Cancelled);
    // 300ms until cancel + 200ms poll + kill; 1.5s leaves CI headroom while
    // still failing loudly if cancel waited for the 30-minute timeout.
    assert!(
        took < std::time::Duration::from_millis(1500),
        "cancel took {took:?}"
    );
    assert!(!part.exists(), "cancelled mux left its .part behind");
    let _ = tokio::fs::remove_dir_all(out).await;
}

/// A real ffmpeg failure (input that is not media) must not strand the
/// seeded `.part` either: only success may leave bytes on disk.
#[tokio::test]
async fn failed_mux_removes_the_part_file() {
    let base = serve(b"not media".to_vec(), 8).await;
    let out = std::env::temp_dir().join(format!("autofox-mux-{}", uuid::Uuid::new_v4()));
    tokio::fs::create_dir_all(&out).await.expect("mkdir");
    let part = out.join("merged.mp4.part");
    tokio::fs::write(&part, b"stale").await.expect("seed part");
    let error = mux_streams_inner(
        None,
        "task-fail",
        MuxRequest {
            video_url: format!("{base}/v.mp4"),
            audio_url: format!("{base}/a.mp4"),
            output_directory: out.to_string_lossy().into_owned(),
            file_name: "merged".to_owned(),
            container: "mp4".to_owned(),
            http_headers: Default::default(),
            duration_secs: None,
        },
        &TaskSignal::default(),
        |_| {},
    )
    .await
    .expect_err("garbage input must fail");
    assert_eq!(error.code, crate::ipc::ErrorCode::MuxFailed);
    assert!(!part.exists(), "failed mux left its .part behind");
    let _ = tokio::fs::remove_dir_all(out).await;
}

#[test]
fn renders_headers_as_a_crlf_block() {
    let headers = BTreeMap::from([
        ("User-Agent".to_owned(), "AutoFox/0.1".to_owned()),
        ("Accept".to_owned(), "*/*".to_owned()),
    ]);
    assert_eq!(
        header_block(&headers).as_deref(),
        Some("Accept: */*\r\nUser-Agent: AutoFox/0.1\r\n")
    );
}

#[test]
fn drops_entries_that_would_inject_extra_headers() {
    let headers = BTreeMap::from([
        ("User-Agent".to_owned(), "ok\r\nX-Injected: 1".to_owned()),
        ("Bad: Name".to_owned(), "v".to_owned()),
        ("Accept".to_owned(), "*/*".to_owned()),
    ]);
    assert_eq!(header_block(&headers).as_deref(), Some("Accept: */*\r\n"));
}

#[test]
fn omits_the_header_flag_when_nothing_survives() {
    assert_eq!(header_block(&BTreeMap::new()), None);
    let only_bad = BTreeMap::from([("X".to_owned(), "a\nb".to_owned())]);
    assert_eq!(header_block(&only_bad), None);
}

/// Guards the bug this test was written for: the muxer used to call
/// `Command::output`, which buffers `-progress pipe:1` until the child exits,
/// so the UI got exactly one event after the file was already written.
///
/// The runner is driven directly with a `-re` paced encode, which makes ffmpeg
/// take about a second of wall time and report several times. Real muxes are
/// paced by the CDN read instead; a local stream copy of a tiny clip finishes
/// in one progress block, so going through `mux_streams_inner` here would
/// assert nothing about streaming. `-re` stays out of production, where
/// throttling the download would be a bug.
#[tokio::test]
async fn reports_progress_while_ffmpeg_is_still_running() {
    let ffmpeg = crate::system::paths::bundled_ffmpeg().expect("ffmpeg path");
    let out = std::env::temp_dir().join(format!("autofox-progress-{}.mp4", uuid::Uuid::new_v4()));
    let mut command = tokio::process::Command::new(ffmpeg);
    command.args([
        "-hide_banner",
        "-v",
        "error",
        "-y",
        // Consume the synthetic source at playback speed so the child lives
        // long enough to emit more than one progress block.
        "-re",
        "-f",
        "lavfi",
        "-i",
        "testsrc=size=64x64:rate=8:duration=2",
        "-c:v",
        "libx264",
        "-progress",
        "pipe:1",
        // Same cadence production pins, for the same reason: the default can
        // report once for a whole job.
        "-stats_period",
        "0.5",
        "-nostats",
        out.to_string_lossy().as_ref(),
    ]);
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let started = std::time::Instant::now();
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<(Option<f64>, f64)>::new()));
    let collected = std::sync::Arc::clone(&seen);
    let output = super::run_with_progress(
        command,
        "mux",
        Some(2.0),
        &TaskSignal::default(),
        move |event| {
            let at = started.elapsed().as_secs_f64();
            collected
                .lock()
                .expect("sink lock")
                .push((event.percent, at));
        },
    )
    .await
    .expect("ffmpeg should run");
    let finished_at = started.elapsed().as_secs_f64();
    assert!(output.status.success(), "ffmpeg exited non-zero");

    let reports = seen.lock().expect("sink lock").clone();
    assert!(
        reports.len() > 1,
        "expected several progress reports, got {reports:?}"
    );
    // The decisive assertion: a buffered reader would stamp every report at
    // process exit, so an early one proves stdout was consumed concurrently.
    assert!(
        reports[0].1 < finished_at * 0.9,
        "first report at {:.3}s did not precede exit at {finished_at:.3}s",
        reports[0].1
    );
    // ffmpeg reports `out_time_ms=N/A` until the first frame is written, so the
    // early blocks legitimately carry no percent. Once a percent appears it
    // must only advance.
    let percents: Vec<f64> = reports.iter().filter_map(|(percent, _)| *percent).collect();
    assert!(
        !percents.is_empty(),
        "expected at least one percent once frames flow: {reports:?}"
    );
    assert!(
        percents.windows(2).all(|pair| pair[1] >= pair[0]),
        "percent must not go backwards: {reports:?}"
    );
    assert!(
        percents.iter().all(|percent| (0.0..=1.0).contains(percent)),
        "percent must stay a fraction: {reports:?}"
    );
    let _ = tokio::fs::remove_file(&out).await;
}

/// Dropping the runner (what `timeout` does on expiry) must kill ffmpeg.
/// ffmpeg ignores the closed progress pipe, so without `kill_on_drop` a paced
/// encode keeps growing its output after the task is gone.
#[tokio::test]
async fn dropping_the_runner_kills_ffmpeg() {
    let ffmpeg = crate::system::paths::bundled_ffmpeg().expect("ffmpeg path");
    let out = std::env::temp_dir().join(format!("autofox-drop-{}.ts", uuid::Uuid::new_v4()));
    let mut command = tokio::process::Command::new(ffmpeg);
    command
        .args(["-hide_banner", "-v", "error", "-y", "-re", "-f", "lavfi"])
        .args([
            "-i",
            "testsrc=size=64x64:rate=8:duration=30",
            "-c:v",
            // Intra-only: x264's lookahead buffers ~5s at 8fps, so it would
            // write nothing inside the window and the test would prove nothing.
            "mjpeg",
        ])
        .args([
            "-progress",
            "pipe:1",
            "-stats_period",
            "0.5",
            // ffmpeg buffers 32 KiB before writing; tiny frames stay under that
            // for seconds, which would read as "not writing" either way.
            "-flush_packets",
            "1",
            "-f",
            "avi",
        ])
        .arg(out.to_string_lossy().as_ref())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let never = TaskSignal::default();
    let run = super::run_with_progress(command, "drop", None, &never, |_| {});
    let _ = tokio::time::timeout(std::time::Duration::from_millis(1500), run).await;

    // Give a live process time to flush more packets, then require no growth.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let before = std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0);
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
    let after = std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0);
    assert!(before > 0, "ffmpeg never started writing");
    assert_eq!(
        before, after,
        "ffmpeg kept writing after the runner was dropped"
    );
    let _ = tokio::fs::remove_file(&out).await;
}

/// The window the arbitration exists for: ffmpeg has exited and the `.part` is
/// complete, then the stop lands before the rename. Nothing may be published,
/// and the abandoned `.part` must go.
#[tokio::test]
async fn a_stop_after_ffmpeg_exits_prevents_publishing() {
    let out = std::env::temp_dir().join(format!("autofox-pub-{}", uuid::Uuid::new_v4()));
    tokio::fs::create_dir_all(&out).await.expect("mkdir");
    let part = out.join("merged.mp4.part");
    let final_path = out.join("merged.mp4");
    tokio::fs::write(&part, b"complete output")
        .await
        .expect("seed part");

    let signal = TaskSignal::default();
    assert!(signal.stop(), "running task accepts the stop");
    let error = publish(&signal, &part, &final_path)
        .await
        .expect_err("a stopped task must not publish");

    assert_eq!(error.code, crate::ipc::ErrorCode::Cancelled);
    assert!(
        !final_path.exists(),
        "output was published despite the stop"
    );
    assert!(!part.exists(), "abandoned .part was left behind");
    let _ = tokio::fs::remove_dir_all(out).await;
}
