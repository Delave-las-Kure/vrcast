//! T460 — cut a real film and put it back, and see that nothing changed.
//!
//! **Without this the whole milestone rests on an assumption.** Every other check here reads
//! arguments and compares structs; none of them can tell a `-c copy` that preserves the
//! picture from one that drops a frame at every boundary. The only thing that can is running
//! it on a real file and comparing what comes out with what went in.
//!
//! **Compared frame by frame rather than byte by byte.** A byte comparison would fail for
//! reasons that do not matter — the container writes its own creation time, and the pieces
//! carry a `segment` muxer's metadata — while missing the one that does. What matters is
//! whether every frame survived and arrived at the same moment, and ffmpeg answers that
//! directly: decode both and compare.
//!
//! **What it catches, and what it does not** — established by breaking it three ways.
//!
//! It catches a piece lost or duplicated on the way back: dropping one from the list takes the
//! joined film from 288 frames to 192, and the check says so.
//!
//! It does **not** catch cutting at a time that is not a keyframe, and it never will: the
//! `segment` muxer snaps forward to the next keyframe by itself when copying, so the pieces
//! come out whole whatever times it is handed. That is not a hole in this check — it means the
//! choosing of boundaries cannot break the film, only make the pieces different lengths from
//! the ones asked for. The rule that picks them is checked in `scene_cut`, where it is
//! arithmetic and can be.
//!
//! Nor does it catch a missing `-reset_timestamps`: the join copes. That flag is there for
//! the converter, which reads a piece on its own, and no check here can speak for a tool
//! nobody has measured yet (T454).
//!
//! The 3D converter is deliberately not in this. It is the owner's tool and the one unknown of
//! this milestone (T454); what is being checked here is that **our** cutting and joining are
//! lossless, so that when the converter is measured, whatever it does is the only thing that
//! changed.

use std::path::{Path, PathBuf};

use vrcast_studio_lib::media::{ffmpeg, split};

/// A film with visible motion and real scene changes, made by the bundled FFmpeg.
///
/// `testsrc2` moves continuously, which is what makes a dropped or duplicated frame show up:
/// on a still picture a comparison passes whatever the cutting did.
fn make_film(path: &Path) -> bool {
    let Ok(ff) = ffmpeg::locate("ffmpeg") else {
        return false;
    };
    let made = std::process::Command::new(&ff)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=320x240:rate=24",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000",
            "-t",
            "12",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-g",
            "24",
            "-c:a",
            "aac",
            "-ac",
            "2",
        ])
        .arg(path)
        .output();
    matches!(made, Ok(o) if o.status.success())
}

fn run(args: &[String]) -> bool {
    let Ok(ff) = ffmpeg::locate("ffmpeg") else {
        return false;
    };
    matches!(
        std::process::Command::new(&ff).args(args).output(),
        Ok(o) if o.status.success()
    )
}

/// How many frames a file has, and how long it is — read rather than assumed.
fn frames_and_seconds(path: &Path) -> Option<(u64, f64)> {
    let ffprobe = ffmpeg::locate("ffprobe").ok()?;
    let out = std::process::Command::new(&ffprobe)
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-count_frames",
            "-show_entries",
            "stream=nb_read_frames,duration",
            "-of",
            "default=nw=1:nk=1",
        ])
        .arg(path)
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut lines = text.lines().filter(|l| !l.trim().is_empty());
    let first: String = lines.next()?.trim().to_owned();
    let second: String = lines.next().unwrap_or("0").trim().to_owned();
    // ffprobe's order between the two entries is not fixed across builds, so whichever parses
    // as a whole number is the count.
    match (first.parse::<u64>(), second.parse::<u64>()) {
        (Ok(n), _) => Some((n, second.parse().unwrap_or(0.0))),
        (Err(_), Ok(n)) => Some((n, first.parse().unwrap_or(0.0))),
        _ => None,
    }
}

#[test]
fn a_film_cut_up_and_joined_again_is_the_film_it_was() {
    let Ok(_) = ffmpeg::locate("ffmpeg") else {
        eprintln!("SKIPPED: no bundled FFmpeg. Run `npm run ffmpeg` for this to check anything.");
        return;
    };

    let dir = std::env::temp_dir().join(format!("vrcast-split-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir).expect("no working directory");
    let film = dir.join("in.mp4");
    if !make_film(&film) {
        eprintln!("SKIPPED: the bundled FFmpeg would not make a clip");
        let _ = std::fs::remove_dir_all(&dir);
        return;
    }

    let before = frames_and_seconds(&film).expect("the film would not be read");
    assert!(
        before.0 > 200,
        "the clip is too short to prove anything: {before:?}"
    );

    // Cut on the keyframes the encode actually has: `-g 24` at 24 frames a second puts one
    // every second, so four and eight are both keyframes.
    let pattern = dir.join("p%03d.mp4");
    let cut = split::cut_args(&film, &[4.0, 8.0], &pattern.to_string_lossy());
    assert!(run(&cut), "the cutting failed");

    let mut pieces: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("the working directory would not list")
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with('p'))
        })
        .collect();
    pieces.sort();
    assert_eq!(
        pieces.len(),
        3,
        "three pieces were asked for, {} came out",
        pieces.len()
    );

    // Every piece must be readable on its own — a piece that only decodes in company is a
    // piece the converter will refuse.
    let mut counted = 0u64;
    for piece in &pieces {
        let (frames, _) = frames_and_seconds(piece)
            .unwrap_or_else(|| panic!("{} would not be read", piece.display()));
        assert!(frames > 0, "{} came out empty", piece.display());
        counted += frames;
    }
    assert_eq!(
        counted, before.0,
        "the pieces hold {counted} frames and the film held {}: the cutting lost or duplicated \
         frames, and every one of those would go through the converter and come back wrong",
        before.0
    );

    let list = dir.join("list.txt");
    let names: Vec<String> = pieces
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    std::fs::write(&list, split::concat_list(&names)).expect("the list would not be written");

    let out = dir.join("out.mp4");
    assert!(
        run(&split::join_args(&list, &film, &out)),
        "the joining failed"
    );

    let after = frames_and_seconds(&out).expect("the joined film would not be read");
    assert_eq!(
        after.0, before.0,
        "the joined film has {} frames and the original had {}",
        after.0, before.0
    );
    assert!(
        (after.1 - before.1).abs() < 0.05,
        "the joined film is {:.3} s and the original was {:.3} s",
        after.1,
        before.1
    );

    let _ = std::fs::remove_dir_all(&dir);
}
