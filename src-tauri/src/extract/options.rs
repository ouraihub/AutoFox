//! Collapses raw extractor formats into the short list a user picks from.
//!
//! yt-dlp reports every rendition it can see. For one YouTube video that is
//! ~36 rows: storyboard sheets, an HLS copy of each DASH rendition, and
//! video-only and audio-only streams that are useless alone. Presenting that
//! list asks the user to understand DASH, so this module answers the question
//! they actually have -- "which quality, as one playable file?" -- and leaves
//! `MediaManifest::formats` untouched for callers that want the raw rows.

use serde::Serialize;

use super::{AudioSpec, Delivery, MediaFormat, SizeHint, VideoSpec};

/// One entry in the user-facing quality list.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadOption {
    /// Stable identifier, unique within one manifest.
    pub option_id: String,
    /// Primary label, e.g. `1080p` or `音频`.
    pub label: String,
    /// Container the finished file will use.
    pub container: String,
    /// Secondary detail line: codec, fps, delivery notes.
    pub detail: String,
    /// Combined size of every stream this option downloads.
    pub size: SizeHint,
    /// Height in pixels, absent for audio-only options.
    pub height: Option<u64>,
    /// How the frontend must fetch this option.
    pub plan: DeliveryPlan,
}

/// The concrete fetch strategy behind a [`DownloadOption`].
#[derive(Debug, Clone, Serialize)]
// On an enum `rename_all` renames the variants; the fields inside each variant
// need `rename_all_fields`. Without it the plan ships `video_url` while the
// TypeScript type and `MuxRequest` both expect `videoUrl`.
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub enum DeliveryPlan {
    /// One HTTP resource that already carries both tracks.
    Progressive {
        /// Format id passed back to `download_direct`.
        format_id: String,
        /// Resource URL.
        url: String,
    },
    /// Two streams ffmpeg must join.
    Merge {
        /// Video-only stream URL.
        video_url: String,
        /// Audio-only stream URL.
        audio_url: String,
        /// Video format id, for logging and task labels.
        video_format_id: String,
        /// Audio format id, for logging and task labels.
        audio_format_id: String,
    },
    /// A manifest ffmpeg reads directly.
    Manifest {
        /// Format id behind this manifest.
        format_id: String,
        /// HLS or DASH URL.
        url: String,
    },
}

/// Builds the quality list for one set of extractor formats.
///
/// Ordering is highest quality first, then audio-only options, because the
/// common intent is "best video" and the rare one is "just the audio".
#[must_use]
pub fn build(formats: &[MediaFormat]) -> Vec<DownloadOption> {
    let usable: Vec<&MediaFormat> = formats.iter().filter(|f| is_usable(f)).collect();
    let audio_pool: Vec<&MediaFormat> = usable
        .iter()
        .copied()
        .filter(|f| f.video.is_none() && f.audio.is_some())
        .collect();
    // WebM accepts only Opus/Vorbis and MP4 only AAC in practice, so the audio
    // track cannot be picked once globally: it depends on the video codec it
    // gets muxed with. This one stays for the audio-only row.
    let best_audio = audio_pool
        .iter()
        .copied()
        .max_by(|a, b| audio_rank(a).total_cmp(&audio_rank(b)));

    let mut video_options: Vec<DownloadOption> = Vec::new();
    let mut heights: Vec<u64> = usable
        .iter()
        .filter_map(|f| f.video.as_ref().and_then(|v| v.height))
        .collect();
    heights.sort_unstable();
    heights.dedup();
    heights.reverse();

    for height in heights {
        let at_height: Vec<&MediaFormat> = usable
            .iter()
            .copied()
            .filter(|f| f.video.as_ref().and_then(|v| v.height) == Some(height))
            .collect();
        if let Some(option) = option_for_height(height, &at_height, &audio_pool) {
            video_options.push(option);
        }
    }

    // Audio-only rows are collapsed to the single best track: a user asking for
    // audio wants the good one, not five bitrate variants of it.
    if let Some(audio) = best_audio
        && let Some(option) = audio_only_option(audio)
    {
        video_options.push(option);
    }
    video_options
}

/// Drops rows that cannot produce media: storyboards carry neither track.
fn is_usable(format: &MediaFormat) -> bool {
    if format.video.is_none() && format.audio.is_none() {
        return false;
    }
    // Storyboards report `mhtml`, which is a sheet of thumbnails.
    format.container != "mhtml"
}

fn option_for_height(
    height: u64,
    candidates: &[&MediaFormat],
    audio_pool: &[&MediaFormat],
) -> Option<DownloadOption> {
    // A progressive stream needs no ffmpeg pass, so it wins when present.
    if let Some(progressive) = candidates
        .iter()
        .copied()
        .filter(|f| f.video.is_some() && f.audio.is_some())
        .max_by(|a, b| video_rank(a).total_cmp(&video_rank(b)))
        && let Delivery::Direct { url } = &progressive.delivery
    {
        let video = progressive.video.as_ref()?;
        return Some(DownloadOption {
            option_id: format!("p-{}", progressive.format_id),
            label: quality_label(height, video.fps),
            container: progressive.container.clone(),
            detail: progressive_detail(video, progressive.audio.as_ref()),
            size: progressive.size.clone(),
            height: Some(height),
            plan: DeliveryPlan::Progressive {
                format_id: progressive.format_id.clone(),
                url: url.clone(),
            },
        });
    }

    let video_only = candidates
        .iter()
        .copied()
        .filter(|f| f.audio.is_none() && f.video.is_some())
        .max_by(|a, b| video_rank(a).total_cmp(&video_rank(b)))?;
    let video = video_only.video.as_ref()?;

    // Above 1080p YouTube ships no H.264, so a stream copy into MP4 would
    // produce a `.mp4` holding VP9/AV1 that many desktop players refuse. The
    // container follows the video codec, and the audio track must then match
    // it: MP4 takes AAC, WebM takes only Opus/Vorbis.
    let container = if is_mp4_native(&video.codec) {
        "mp4"
    } else {
        "webm"
    };
    let audio = audio_pool
        .iter()
        .copied()
        .filter(|f| {
            f.audio
                .as_ref()
                .is_some_and(|a| is_mp4_native(&a.codec) == (container == "mp4"))
        })
        .max_by(|a, b| audio_rank(a).total_cmp(&audio_rank(b)));

    // Without a compatible audio track a merge would either fail in ffmpeg or
    // produce a silent file, so the stream is offered as a manifest instead.
    let Some(audio) = audio else {
        let url = stream_url(&video_only.delivery);
        return Some(DownloadOption {
            option_id: format!("v-{}", video_only.format_id),
            label: quality_label(height, video.fps),
            container: container.to_owned(),
            detail: format!("{} · 无音轨", video.codec),
            size: video_only.size.clone(),
            height: Some(height),
            plan: DeliveryPlan::Manifest {
                format_id: video_only.format_id.clone(),
                url: url.to_owned(),
            },
        });
    };
    let audio_spec = audio.audio.as_ref()?;
    Some(DownloadOption {
        option_id: format!("m-{}+{}", video_only.format_id, audio.format_id),
        label: quality_label(height, video.fps),
        container: container.to_owned(),
        detail: format!("{} + {} · 合流", video.codec, audio_spec.codec),
        size: combine(&video_only.size, &audio.size),
        height: Some(height),
        plan: DeliveryPlan::Merge {
            video_url: stream_url(&video_only.delivery).to_owned(),
            audio_url: stream_url(&audio.delivery).to_owned(),
            video_format_id: video_only.format_id.clone(),
            audio_format_id: audio.format_id.clone(),
        },
    })
}

/// Reports whether a codec belongs in an MP4 container.
///
/// yt-dlp spells these as RFC 6381 strings (`avc1.64002a`, `mp4a.40.2`) but
/// also plain names on some sites, so both are matched. VP9, AV1, Opus and
/// Vorbis fall through to WebM.
fn is_mp4_native(codec: &str) -> bool {
    let codec = codec.to_ascii_lowercase();
    codec.starts_with("avc1")
        || codec.starts_with("avc3")
        || codec.contains("h264")
        || codec.starts_with("hev1")
        || codec.starts_with("hvc1")
        || codec.starts_with("mp4a")
        || codec.contains("aac")
}

fn audio_only_option(audio: &MediaFormat) -> Option<DownloadOption> {
    let spec = audio.audio.as_ref()?;
    let detail = match spec.bitrate_kbps {
        Some(kbps) => format!("{} · {:.0} kbps", spec.codec, kbps),
        None => spec.codec.clone(),
    };
    let plan = match &audio.delivery {
        Delivery::Direct { url } => DeliveryPlan::Progressive {
            format_id: audio.format_id.clone(),
            url: url.clone(),
        },
        other => DeliveryPlan::Manifest {
            format_id: audio.format_id.clone(),
            url: stream_url(other).to_owned(),
        },
    };
    Some(DownloadOption {
        option_id: format!("a-{}", audio.format_id),
        label: "仅音频".to_owned(),
        container: audio.container.clone(),
        detail,
        size: audio.size.clone(),
        height: None,
        plan,
    })
}

fn progressive_detail(video: &VideoSpec, audio: Option<&AudioSpec>) -> String {
    match audio {
        Some(audio) => format!("{} + {}", video.codec, audio.codec),
        None => video.codec.clone(),
    }
}

fn quality_label(height: u64, fps: Option<f64>) -> String {
    // High frame rate is the one video attribute that changes what the user
    // sees at the same height, so it earns a place in the label.
    match fps {
        Some(fps) if fps >= 50.0 => format!("{height}p{}", fps.round()),
        _ => format!("{height}p"),
    }
}

/// Ranks video candidates at one height.
///
/// H.264 outranks VP9/AV1 at equal delivery because it plays in every desktop
/// player and hardware decoder without a codec pack; a progressive or direct
/// URL outranks a manifest because it needs no ffmpeg pass and reports a size.
fn video_rank(format: &MediaFormat) -> f64 {
    let delivery = match format.delivery {
        Delivery::Direct { .. } => 3.0,
        Delivery::Dash { .. } => 2.0,
        Delivery::Hls { .. } => 1.0,
    };
    let codec = format
        .video
        .as_ref()
        .map(|v| {
            let name = v.codec.to_ascii_lowercase();
            if name.starts_with("avc1") || name.contains("h264") {
                2.0
            } else {
                1.0
            }
        })
        .unwrap_or(0.0);
    delivery * 10.0 + codec
}

/// Ranks audio candidates. AAC beats Opus for MP4 compatibility, then bitrate.
fn audio_rank(format: &MediaFormat) -> f64 {
    let Some(audio) = format.audio.as_ref() else {
        return 0.0;
    };
    let name = audio.codec.to_ascii_lowercase();
    let codec = if name.starts_with("mp4a") || name.contains("aac") {
        2.0
    } else {
        1.0
    };
    let delivery = match format.delivery {
        Delivery::Direct { .. } => 2.0,
        _ => 1.0,
    };
    // Bitrate is scaled down so it only breaks ties inside one codec class.
    delivery * 1_000.0 + codec * 100.0 + audio.bitrate_kbps.unwrap_or(0.0) / 10.0
}

fn stream_url(delivery: &Delivery) -> &str {
    match delivery {
        Delivery::Direct { url } => url,
        Delivery::Hls { playlist_url } => playlist_url,
        Delivery::Dash { manifest_url } => manifest_url,
    }
}

/// Sums two size hints, degrading to the weakest certainty of the pair.
fn combine(left: &SizeHint, right: &SizeHint) -> SizeHint {
    match (left, right) {
        (SizeHint::Exact(a), SizeHint::Exact(b)) => SizeHint::Exact(a + b),
        (SizeHint::Exact(a) | SizeHint::Approx(a), SizeHint::Exact(b) | SizeHint::Approx(b)) => {
            SizeHint::Approx(a + b)
        }
        _ => SizeHint::Unknown,
    }
}

#[cfg(test)]
mod wire_tests {
    use super::DeliveryPlan;

    /// `mux_streams` deserializes `videoUrl`/`audioUrl`, and the TypeScript
    /// `DeliveryPlan` reads the same names. A snake_case key here is invisible
    /// to both the compiler and `svelte-check`: it only shows up at runtime as
    /// "missing field `videoUrl`" once a user clicks download.
    #[test]
    fn merge_plan_serializes_the_keys_mux_streams_expects() {
        let json = serde_json::to_value(DeliveryPlan::Merge {
            video_url: "https://cdn.example/v".to_owned(),
            audio_url: "https://cdn.example/a".to_owned(),
            video_format_id: "299".to_owned(),
            audio_format_id: "140".to_owned(),
        })
        .expect("serialize");
        let object = json.as_object().expect("object");
        let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "audioFormatId",
                "audioUrl",
                "kind",
                "videoFormatId",
                "videoUrl"
            ]
        );
        assert_eq!(object["kind"], "merge");
    }

    #[test]
    fn progressive_and_manifest_plans_expose_camel_case_format_id() {
        for plan in [
            DeliveryPlan::Progressive {
                format_id: "18".to_owned(),
                url: "https://cdn.example/v".to_owned(),
            },
            DeliveryPlan::Manifest {
                format_id: "137".to_owned(),
                url: "https://cdn.example/m3u8".to_owned(),
            },
        ] {
            let json = serde_json::to_value(plan).expect("serialize");
            assert!(json.get("formatId").is_some(), "expected formatId: {json}");
            assert!(json.get("format_id").is_none(), "snake_case leaked: {json}");
        }
    }
}
