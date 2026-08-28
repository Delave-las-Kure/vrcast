//! T236, T237 — a measurement that survives, and one that is lent.

use vrcast_studio_lib::domain::measure_grid::seconds_per_point;
use vrcast_studio_lib::domain::measured_ladder::Point;
use vrcast_studio_lib::store::db::Db;
use vrcast_studio_lib::store::measurements::{
    all, begin, differs, forget, lend, machine_factor, points, record, run, LendRefusal, Material,
    Mismatch, Run,
};

fn a_run(key: &str, path: &str) -> Run {
    Run {
        source_key: key.to_owned(),
        codec: String::from("h264"),
        source_path: path.to_owned(),
        width: 3840,
        height: 2160,
        fps: 24,
        source_bitrate_bps: 60_000_000,
        heavier_codec: false,
        native_height: Some(1080),
        anchor_mbps: 16,
        chunk_starts: vec![233, 590, 947],
        chunk_s: 10,
        borrowed_from: None,
        donor_anchor_mbps: None,
        material: Some(Material {
            codec: String::from("h264"),
            pix_fmt: String::from("yuv420p"),
            color_transfer: Some(String::from("bt709")),
            duration_s: 2700.0,
            peak_bps: Some(41_000_000),
        }),
    }
}

/// A plausible time for one point on the machine the model was measured on.
fn took() -> std::time::Duration {
    std::time::Duration::from_millis(12_600)
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
    record(
        &db,
        &job.source_key,
        &job.codec,
        &a_point(6, 1728, 91.2),
        took(),
    )
    .unwrap();
    record(
        &db,
        &job.source_key,
        &job.codec,
        &a_point(10, 1728, 94.8),
        took(),
    )
    .unwrap();

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
    record(
        &db,
        &job.source_key,
        &job.codec,
        &a_point(10, 1728, 94.8),
        took(),
    )
    .unwrap();

    assert_eq!(points(&db, &job.source_key, "hevc").unwrap().len(), 0);
    assert!(run(&db, &job.source_key, "hevc").unwrap().is_none());
}

#[test]
fn lending_carries_the_chunks_and_says_plainly_that_it_was_lent() {
    let db = Db::open_in_memory().expect("the database would not open");
    let first = a_run("1234:s01e01.mp4", "F:/films/s01e01.mp4");
    begin(&db, &first).unwrap();
    for point in [a_point(6, 1728, 91.2), a_point(10, 1728, 94.8)] {
        record(&db, &first.source_key, &first.codec, &point, took()).unwrap();
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
        took(),
    )
    .unwrap();

    // The same frame and frame rate, but this one was upscaled from 720 rather than 1080.
    // The height above which there is no detail left sits somewhere else, and so does every
    // point where the resolution should drop.
    let mut upscaled = a_run("9999:upscaled.mp4", "F:/films/upscaled.mp4");
    upscaled.native_height = Some(720);
    assert_eq!(
        lend(&db, &native.source_key, "h264", &upscaled),
        Err(LendRefusal::DifferentMaterial(Mismatch::NativeHeight))
    );

    let mut faster = a_run("8888:48fps.mp4", "F:/films/48fps.mp4");
    faster.fps = 48;
    assert_eq!(
        lend(&db, &native.source_key, "h264", &faster),
        Err(LendRefusal::DifferentMaterial(Mismatch::Fps))
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
    record(
        &db,
        &job.source_key,
        &job.codec,
        &a_point(10, 1728, 94.8),
        took(),
    )
    .unwrap();
    assert_eq!(all(&db).unwrap().len(), 1);

    forget(&db, &job.source_key, &job.codec).unwrap();
    assert!(run(&db, &job.source_key, &job.codec).unwrap().is_none());
    assert!(points(&db, &job.source_key, &job.codec).unwrap().is_empty());
    assert!(all(&db).unwrap().is_empty());
}

#[test]
fn the_estimate_learns_what_this_machine_really_does() {
    // The shipped model was measured on one machine. Somebody else's is faster or slower,
    // and the difference between twenty minutes and two hours is the whole decision.
    let db = Db::open_in_memory().expect("the database would not open");
    assert_eq!(
        machine_factor(&db).unwrap(),
        None,
        "a machine that has measured nothing was given a correction anyway"
    );

    let job = a_run("1234:film.mp4", "F:/films/film.mp4");
    begin(&db, &job).unwrap();

    // Three points, each taking twice what the model expects of this material.
    let expected = seconds_per_point(job.width, job.height, job.fps, job.chunk_s, 3);
    let slow = std::time::Duration::from_secs_f64(expected * 2.0);
    for (i, height) in [1728u32, 1382, 1104].iter().enumerate() {
        record(
            &db,
            &job.source_key,
            &job.codec,
            &a_point(6 + i as u64, *height, 90.0),
            slow,
        )
        .unwrap();
    }

    let speed = machine_factor(&db).unwrap().expect("nothing was learned");
    assert_eq!(speed.points, 3);
    assert!(
        (speed.factor - 2.0).abs() < 0.01,
        "this machine is twice as slow and the correction says {}",
        speed.factor
    );
    // What a point actually took here, which is the half a person can recognise (T423). The
    // factor alone is not something anybody can check against what they watched happen.
    assert!(
        speed.seconds_per_point > 0.0,
        "the factor was worked out and how long a point took was thrown away"
    );
}

#[test]
fn a_lent_point_is_not_counted_as_work_this_machine_did() {
    // Lending copies somebody else's points. Counting them as time spent here would
    // flatter every estimate afterwards — an episode borrowed in a second would say the
    // machine measures a point in no time at all.
    let db = Db::open_in_memory().expect("the database would not open");
    let first = a_run("1234:s01e01.mp4", "F:/films/s01e01.mp4");
    begin(&db, &first).unwrap();
    record(
        &db,
        &first.source_key,
        &first.codec,
        &a_point(10, 1728, 94.8),
        took(),
    )
    .unwrap();

    let before = machine_factor(&db)
        .unwrap()
        .expect("nothing was learned")
        .points;
    let second = a_run("5678:s01e02.mp4", "F:/films/s01e02.mp4");
    lend(&db, &first.source_key, "h264", &second).expect("the episode was refused");

    let after = machine_factor(&db)
        .unwrap()
        .expect("nothing was learned")
        .points;
    assert_eq!(
        after, before,
        "a borrowed point was counted as time this machine spent"
    );
}

// ---------- lending, and what it must not lose (T429, T430) ----------

#[test]
fn a_chain_of_loans_names_the_file_the_points_really_came_from() {
    // **The fault: the middleman gets the credit.** A lends to B, B lends to C. Every point
    // C holds was encoded on A's material, and C's mark said B — a file that is itself only
    // a copy. Somebody deciding whether to trust the ladder would go and look at B, find a
    // measurement that says "borrowed" and have to walk the chain by hand; and if B is
    // deleted, the trail ends at a file that no longer exists.
    let db = Db::open_in_memory().expect("the database would not open");
    let first = a_run("1:e01.mp4", "F:/films/e01.mp4");
    begin(&db, &first).unwrap();
    record(
        &db,
        &first.source_key,
        &first.codec,
        &a_point(10, 1728, 94.8),
        took(),
    )
    .unwrap();

    let second = a_run("2:e02.mp4", "F:/films/e02.mp4");
    lend(&db, &first.source_key, "h264", &second).expect("the second was refused");
    let third = a_run("3:e03.mp4", "F:/films/e03.mp4");
    let borrowed = lend(&db, &second.source_key, "h264", &third).expect("the third was refused");

    assert_eq!(
        borrowed.borrowed_from.as_deref(),
        Some("F:/films/e01.mp4"),
        "the loan named the file it came through rather than the one it came from"
    );
    // And it is what the store says too, not only what the call returned.
    let stored = run(&db, &third.source_key, "h264").unwrap().unwrap();
    assert_eq!(stored.borrowed_from.as_deref(), Some("F:/films/e01.mp4"));
}

#[test]
fn a_borrower_keeps_its_own_anchor() {
    // **What the anchor is for.** It is the top of the grid this film's own complexity probe
    // asked for. Overwriting it with the donor's throws away the one number that was measured
    // on *this* material — so the borrowed ladder and the check that would one day compare
    // the two both stand on the donor's figure, and nothing is left to disagree with it.
    let db = Db::open_in_memory().expect("the database would not open");
    let donor = a_run("1:e01.mp4", "F:/films/e01.mp4");
    begin(&db, &donor).unwrap();
    record(
        &db,
        &donor.source_key,
        &donor.codec,
        &a_point(10, 1728, 94.8),
        took(),
    )
    .unwrap();

    let mut borrower = a_run("2:e02.mp4", "F:/films/e02.mp4");
    borrower.anchor_mbps = 22; // its own probe asked for more than the donor's 16
    let borrowed = lend(&db, &donor.source_key, "h264", &borrower).expect("refused");

    assert_eq!(
        borrowed.anchor_mbps, 22,
        "the borrower's own probe was overwritten by the donor's"
    );
    assert_eq!(
        borrowed.donor_anchor_mbps,
        Some(16),
        "the donor's anchor was thrown away, so the two can never be compared"
    );
}

#[test]
fn measuring_after_a_loan_does_not_pass_the_lent_points_off_as_ours() {
    // **The quiet lie.** `prepare()` builds a run with `borrowed_from: None`, `begin()`
    // overwrites the column from it, and the measuring task skips cells that already have an
    // answer — which, after a loan, is all of them. So a run that measured nothing came out
    // marked "measured here", and every rung resting on somebody else's points said so.
    let db = Db::open_in_memory().expect("the database would not open");
    let donor = a_run("1:e01.mp4", "F:/films/e01.mp4");
    begin(&db, &donor).unwrap();
    record(
        &db,
        &donor.source_key,
        &donor.codec,
        &a_point(10, 1728, 94.8),
        took(),
    )
    .unwrap();

    let borrower = a_run("2:e02.mp4", "F:/films/e02.mp4");
    lend(&db, &donor.source_key, "h264", &borrower).expect("refused");

    // Exactly what a measurement does when it starts: the same run, freshly prepared, which
    // knows nothing about any loan.
    begin(&db, &borrower).unwrap();

    let after = run(&db, &borrower.source_key, "h264").unwrap().unwrap();
    assert_eq!(
        after.borrowed_from.as_deref(),
        Some("F:/films/e01.mp4"),
        "starting a measurement wiped the mark, and borrowed points now read as measured here"
    );
    assert!(!after.is_measured_here());
}

// ---------- what may be lent, and what may not (T431) ----------

/// Two runs of the same shape, differing only in what the test changes.
fn pair() -> (Run, Run) {
    (
        a_run("1:e01.mp4", "F:/films/e01.mp4"),
        a_run("2:e02.mp4", "F:/films/e02.mp4"),
    )
}

#[test]
fn a_measurement_of_one_codec_is_not_lent_to_another() {
    // **The fault, exactly.** The comparison asked "is this HEVC" and nothing else about the
    // codec — so AV1 and VP9 were both "not HEVC", the same as H.264, and a measurement of an
    // AV1 master was lent to an H.264 file as the same material. The codec decides how much
    // picture a bit buys, which is the whole question a measurement answers.
    for other in ["av1", "vp9", "hevc"] {
        let (from, mut to) = pair();
        to.material.as_mut().unwrap().codec = String::from(other);
        assert_eq!(
            differs(&from, &to),
            Some(Mismatch::Codec),
            "a measurement of h264 was lent to {other} as the same material"
        );
    }
}

#[test]
fn the_same_codec_spelled_differently_is_still_the_same_codec() {
    // ffprobe is not consistent about case between builds, and refusing a loan over "H264"
    // against "h264" would send somebody off to measure half an hour for nothing.
    let (from, mut to) = pair();
    to.material.as_mut().unwrap().codec = String::from("H264");
    assert_eq!(differs(&from, &to), None);
}

#[test]
fn colour_is_not_lent_across() {
    // A different pixel format or transfer curve is a different picture at the same bitrate:
    // 10-bit holds a gradient where 8-bit bands, and HDR spends its bits somewhere else
    // entirely. Both were read on every measurement and both were thrown away.
    let (from, mut to) = pair();
    to.material.as_mut().unwrap().pix_fmt = String::from("yuv420p10le");
    assert_eq!(differs(&from, &to), Some(Mismatch::PixelFormat));

    let (from, mut to) = pair();
    to.material.as_mut().unwrap().color_transfer = Some(String::from("smpte2084"));
    assert_eq!(differs(&from, &to), Some(Mismatch::ColourTransfer));
}

#[test]
fn a_film_that_ends_before_the_chunks_do_is_refused() {
    // **A rule, not a threshold.** The borrower is measured on the donor's chunk positions,
    // so those positions have to exist in it. An episode ending before the donor's last chunk
    // begins would be measured on nothing; a shorter one still would have its chunks land in
    // whatever scene happened to be there, and the comparison would be of two different
    // things. No invented tolerance is needed: either the film covers the chunks or it does
    // not.
    let (from, mut to) = pair();
    // The donor's chunks start at 233, 590 and 947, each ten seconds long.
    to.material.as_mut().unwrap().duration_s = 900.0;
    assert_eq!(differs(&from, &to), Some(Mismatch::TooShort));

    let (from, mut to) = pair();
    to.material.as_mut().unwrap().duration_s = 957.0;
    assert_eq!(
        differs(&from, &to),
        None,
        "a film that just covers them is fine"
    );
}

#[test]
fn a_trailer_length_difference_alone_is_not_a_refusal() {
    // The negative control. Episodes of a season differ by a minute or two, and refusing over
    // that would make lending useless for the one case it exists for.
    let (from, mut to) = pair();
    to.material.as_mut().unwrap().duration_s = from.material.as_ref().unwrap().duration_s + 180.0;
    assert_eq!(differs(&from, &to), None);
}

#[test]
fn a_measurement_from_before_the_material_was_kept_is_not_lent() {
    // **Not knowing is a refusal.** A row written before these columns existed says nothing
    // about what it was measured on, and lending it would be vouching for something nobody
    // looked at. Measuring again costs half an hour; a ladder built on the wrong material
    // costs the encode and then the viewer.
    let (mut from, to) = pair();
    from.material = None;
    assert_eq!(differs(&from, &to), Some(Mismatch::NotKnown));

    let (from, mut to) = pair();
    to.material = None;
    assert_eq!(differs(&from, &to), Some(Mismatch::NotKnown));
}

#[test]
fn the_same_material_is_lent() {
    // The negative control for the lot. Without it, "refuse when they differ" is satisfied by
    // refusing always — and lending that never lends would pass every check above.
    let (from, to) = pair();
    assert_eq!(differs(&from, &to), None);
}
