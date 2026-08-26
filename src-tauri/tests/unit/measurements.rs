//! T236, T237 — a measurement that survives, and one that is lent.

use vrcast_studio_lib::domain::measured_ladder::Point;
use vrcast_studio_lib::store::db::Db;
use vrcast_studio_lib::store::measurements::{
    all, begin, forget, lend, points, record, run, LendRefusal, Run,
};

fn a_run(key: &str, path: &str) -> Run {
    Run {
        source_key: key.to_owned(),
        codec: String::from("h264"),
        source_path: path.to_owned(),
        width: 3840,
        height: 2160,
        fps: 24,
        native_height: Some(1080),
        anchor_mbps: 16,
        chunk_starts: vec![233, 590, 947],
        chunk_s: 10,
        borrowed_from: None,
    }
}

fn a_point(bitrate_mbps: u64, height: u32, vmaf: f64) -> Point {
    Point {
        bitrate_mbps,
        height,
        actual_bps: bitrate_mbps * 1_000_000,
        vmaf,
    }
}

#[test]
fn what_was_measured_survives_the_run_being_cut_short() {
    // Half an hour of somebody's machine. A cancellation, a crash or a closed application
    // must cost the points not yet taken and nothing more.
    let db = Db::open_in_memory().expect("the database would not open");
    let job = a_run("1234:film.mp4", "F:/films/film.mp4");

    begin(&db, &job).unwrap();
    record(&db, &job.source_key, &job.codec, &a_point(6, 1728, 91.2)).unwrap();
    record(&db, &job.source_key, &job.codec, &a_point(10, 1728, 94.8)).unwrap();

    // The run begins again — as it does when a person starts it a second time.
    begin(&db, &job).unwrap();

    let kept = points(&db, &job.source_key, &job.codec).unwrap();
    assert_eq!(kept.len(), 2, "beginning again threw the measurement away");
    assert_eq!(kept[0], a_point(6, 1728, 91.2));

    // And the chunks are kept with it, so the rest of the grid is measured on the same
    // scenes. Measured on different ones, the later points would not be comparable with the
    // earlier: the percentiles would agree and the material would not.
    assert_eq!(
        run(&db, &job.source_key, &job.codec)
            .unwrap()
            .unwrap()
            .chunk_starts,
        vec![233, 590, 947]
    );
}

#[test]
fn a_measurement_taken_for_one_codec_is_not_offered_for_another() {
    // AV1's advantage over H.264 melts as the bitrate rises — +5.42 VMAF at 4 Mbit/s
    // against +1.09 at 22 — so there is no multiplier that carries one ladder to the other,
    // and the heights change places as well.
    let db = Db::open_in_memory().expect("the database would not open");
    let job = a_run("1234:film.mp4", "F:/films/film.mp4");
    begin(&db, &job).unwrap();
    record(&db, &job.source_key, &job.codec, &a_point(10, 1728, 94.8)).unwrap();

    assert_eq!(points(&db, &job.source_key, "hevc").unwrap().len(), 0);
    assert!(run(&db, &job.source_key, "hevc").unwrap().is_none());
}

#[test]
fn lending_carries_the_chunks_and_says_plainly_that_it_was_lent() {
    let db = Db::open_in_memory().expect("the database would not open");
    let first = a_run("1234:s01e01.mp4", "F:/films/s01e01.mp4");
    begin(&db, &first).unwrap();
    for point in [a_point(6, 1728, 91.2), a_point(10, 1728, 94.8)] {
        record(&db, &first.source_key, &first.codec, &point).unwrap();
    }

    let second = a_run("5678:s01e02.mp4", "F:/films/s01e02.mp4");
    let borrowed = lend(&db, &first.source_key, "h264", &second).expect("the episode was refused");

    // The same scenes, by position: chosen afresh the percentiles would be the same and the
    // scenes different, and the difference between episodes would mix into the difference
    // between rungs.
    assert_eq!(borrowed.chunk_starts, first.chunk_starts);
    assert_eq!(borrowed.anchor_mbps, first.anchor_mbps);
    assert_eq!(points(&db, &second.source_key, "h264").unwrap().len(), 2);

    // And it does not pass for a measurement of this episode.
    assert_eq!(
        borrowed.borrowed_from.as_deref(),
        Some("F:/films/s01e01.mp4")
    );
    assert!(!borrowed.is_measured_here());
    assert!(first.is_measured_here());
}

#[test]
fn a_measurement_is_not_lent_to_material_of_another_kind() {
    let db = Db::open_in_memory().expect("the database would not open");
    let native = a_run("1234:native.mp4", "F:/films/native.mp4");
    begin(&db, &native).unwrap();
    record(
        &db,
        &native.source_key,
        &native.codec,
        &a_point(10, 2160, 95.0),
    )
    .unwrap();

    // The same frame and frame rate, but this one was upscaled from 720 rather than 1080.
    // The height above which there is no detail left sits somewhere else, and so does every
    // point where the resolution should drop.
    let mut upscaled = a_run("9999:upscaled.mp4", "F:/films/upscaled.mp4");
    upscaled.native_height = Some(720);
    assert_eq!(
        lend(&db, &native.source_key, "h264", &upscaled),
        Err(LendRefusal::DifferentMaterial)
    );

    let mut faster = a_run("8888:48fps.mp4", "F:/films/48fps.mp4");
    faster.fps = 48;
    assert_eq!(
        lend(&db, &native.source_key, "h264", &faster),
        Err(LendRefusal::DifferentMaterial)
    );

    // And there is nothing to lend from a film nobody measured.
    assert_eq!(
        lend(&db, "0:never-measured.mp4", "h264", &upscaled),
        Err(LendRefusal::NothingToLend)
    );
}

#[test]
fn forgetting_a_measurement_takes_its_points_with_it() {
    let db = Db::open_in_memory().expect("the database would not open");
    let job = a_run("1234:film.mp4", "F:/films/film.mp4");
    begin(&db, &job).unwrap();
    record(&db, &job.source_key, &job.codec, &a_point(10, 1728, 94.8)).unwrap();
    assert_eq!(all(&db).unwrap().len(), 1);

    forget(&db, &job.source_key, &job.codec).unwrap();
    assert!(run(&db, &job.source_key, &job.codec).unwrap().is_none());
    assert!(points(&db, &job.source_key, &job.codec).unwrap().is_empty());
    assert!(all(&db).unwrap().is_empty());
}
