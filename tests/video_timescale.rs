//! Video media-timescale selection.
//!
//! The historical default of 1000 truncates frame durations that are not whole
//! milliseconds. 24 fps on a 90 kHz timebase is 3750 ticks (41.666 ms); written
//! at a 1000 timescale it becomes 41 ms, and the error accumulates across the
//! stream. Callers whose timebase divides their frame rate exactly can now say
//! so.

use oxideav_core::{CodecId, CodecParameters, Packet, StreamInfo, TimeBase, WriteSeek};
use oxideav_mp4::muxer::open_with_options;
use oxideav_mp4::options::Mp4MuxerOptions;

fn video_stream(timebase: TimeBase) -> StreamInfo {
    let mut params = CodecParameters::video(CodecId::new("av1"));
    params.width = Some(320);
    params.height = Some(240);
    params.extradata = vec![0x81, 0x04, 0x0c, 0x00];
    StreamInfo {
        index: 0,
        time_base: timebase,
        duration: None,
        start_time: Some(0),
        params,
    }
}

fn packet(stream: &StreamInfo, pts: i64, duration: i64) -> Packet {
    let mut packet = Packet::new(0, stream.time_base, vec![0x12, 0x00, 0xaa]);
    packet.pts = Some(pts);
    packet.dts = Some(pts);
    packet.duration = Some(duration);
    packet.flags.keyframe = true;
    packet
}

fn mux(name: &str, options: Mp4MuxerOptions, stream: &StreamInfo) -> Vec<u8> {
    let path = std::env::temp_dir().join(format!(
        "oxideav-mp4-timescale-{}-{name}.mp4",
        std::process::id()
    ));
    {
        let file = std::fs::File::create(&path).unwrap();
        let output: Box<dyn WriteSeek> = Box::new(file);
        let mut muxer = open_with_options(output, std::slice::from_ref(stream), options).unwrap();
        muxer.write_header().unwrap();
        muxer.write_packet(&packet(stream, 0, 3750)).unwrap();
        muxer.write_packet(&packet(stream, 3750, 3750)).unwrap();
        muxer.write_trailer().unwrap();
    }
    let bytes = std::fs::read(&path).unwrap();
    let _ = std::fs::remove_file(&path);
    bytes
}

/// Read the first `mdhd` timescale (version 0 or 1).
fn media_timescale(bytes: &[u8]) -> u32 {
    let pos = bytes
        .windows(4)
        .position(|window| window == b"mdhd")
        .expect("mdhd box");
    let version = bytes[pos + 4];
    let body = pos + 8; // fourcc + version/flags
    let timescale_at = if version == 1 { body + 16 } else { body + 8 };
    u32::from_be_bytes(bytes[timescale_at..timescale_at + 4].try_into().unwrap())
}

/// Read the first `stts` sample delta.
fn sample_delta(bytes: &[u8]) -> u32 {
    let pos = bytes
        .windows(4)
        .position(|window| window == b"stts")
        .expect("stts box");
    // fourcc + version/flags + entry_count + sample_count, then delta
    let at = pos + 4 + 4 + 4 + 4;
    u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap())
}

#[test]
fn explicit_video_timescale_preserves_frame_durations() {
    let stream = video_stream(TimeBase::new(1, 90_000));
    let bytes = mux(
        "explicit",
        Mp4MuxerOptions {
            video_media_timescale: Some(90_000),
            ..Mp4MuxerOptions::default()
        },
        &stream,
    );
    assert_eq!(media_timescale(&bytes), 90_000);
    assert_eq!(
        sample_delta(&bytes),
        3750,
        "24 fps is exactly 3750 ticks at 90 kHz"
    );
}

#[test]
fn default_video_timescale_is_still_1000() {
    let stream = video_stream(TimeBase::new(1, 90_000));
    let bytes = mux("default", Mp4MuxerOptions::default(), &stream);
    assert_eq!(media_timescale(&bytes), 1000);
    assert_eq!(
        sample_delta(&bytes),
        41,
        "3750 / 90000 s truncates to 41 ms"
    );
}

#[test]
fn audio_tracks_keep_their_sample_rate() {
    let mut params = CodecParameters::audio(CodecId::new("aac"));
    params.channels = Some(2);
    params.sample_rate = Some(48_000);
    params.extradata = vec![0x11, 0x90]; // AudioSpecificConfig
    let stream = StreamInfo {
        index: 0,
        time_base: TimeBase::new(1, 48_000),
        duration: None,
        start_time: Some(0),
        params,
    };
    let bytes = mux("audio", Mp4MuxerOptions::default(), &stream);
    assert_eq!(
        media_timescale(&bytes),
        48_000,
        "video option must not affect audio tracks"
    );
}
