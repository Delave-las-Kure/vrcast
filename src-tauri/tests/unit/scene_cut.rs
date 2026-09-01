//! T455, T456 — where to cut, and what to say when the cut cannot land where it should.

use vrcast_studio_lib::domain::scene_cut::{choose, seams_that_missed, A_FRAME_S};

/// Keyframes every two seconds, as an ordinary encode has them.
fn keyframes(every: f64, until: f64) -> Vec<f64> {
    let mut out = Vec::new();
    let mut at = 0.0;
    while at < until {
        out.push(at);
        at += every;
    }
    out
}

#[test]
fn a_cut_lands_on_a_keyframe_and_nowhere_else() {
    // The rule everything else rests on. A piece that begins mid-group cannot be copied out,
    // only re-encoded — and a re-encode here means the converter sees second-generation
    // material before it starts.
    let keys = keyframes(2.0, 600.0);
    let scenes: Vec<(f64, f64)> = (1..30).map(|i| (i as f64 * 21.3, 0.5)).collect();
    let cuts = choose(&scenes, &keys, 60.0, 600.0);

    assert!(!cuts.is_empty());
    for cut in &cuts {
        assert!(
            keys.iter().any(|k| (k - cut.at_s).abs() < 1e-9),
            "a cut at {} is not on any keyframe",
            cut.at_s
        );
    }
}

#[test]
fn the_strongest_scene_in_the_window_is_taken() {
    // A seam is invisible where the picture changes most. Two candidates in the window, and
    // the stronger one wins even though the weaker is nearer the wanted length.
    let keys = keyframes(1.0, 200.0);
    let scenes = vec![(58.0, 0.2), (65.0, 0.9)];
    let cuts = choose(&scenes, &keys, 60.0, 200.0);
    assert_eq!(cuts[0].scene_s, Some(65.0));
}

#[test]
fn without_a_scene_the_cut_is_honest_rather_than_stretched() {
    // Nothing to hide the seam behind. Reaching outside the window for a distant scene change
    // would give pieces of wildly different length; the nearest keyframe to the target is the
    // plain answer, and it says it chose no scene.
    let keys = keyframes(2.0, 400.0);
    let cuts = choose(&[], &keys, 60.0, 400.0);
    assert!(!cuts.is_empty());
    assert!(cuts.iter().all(|c| c.scene_s.is_none()));
    assert!((cuts[0].at_s - 60.0).abs() <= 1.0);
}

#[test]
fn each_piece_is_measured_from_where_the_last_one_ended() {
    // **Not a division of the film into equal parts.** Dividing would put every boundary at a
    // fixed multiple of the target and hunt for a scene near each, so one long piece pushes
    // every later boundary off its scene. Walking forward keeps each piece the asked-for
    // length from the real end of the one before.
    let keys = keyframes(1.0, 400.0);
    // A strong scene late in the first window, then scenes on a rhythm that only lines up if
    // the walk is forward.
    let scenes = vec![(75.0, 0.9), (135.0, 0.9), (195.0, 0.9), (255.0, 0.9)];
    let cuts = choose(&scenes, &keys, 60.0, 400.0);
    // The four scenes, in order. What follows them is a fifth cut with no scene of its own —
    // there are still 145 seconds after 255, and leaving them as one piece would be worse.
    // The walk is what is being checked here, not where the film ends.
    let chosen: Vec<Option<f64>> = cuts.iter().take(4).map(|c| c.scene_s).collect();
    assert_eq!(
        chosen,
        vec![Some(75.0), Some(135.0), Some(195.0), Some(255.0)],
        "the walk did not follow the scenes from where each piece really ended"
    );
}

#[test]
fn a_scrap_at_the_end_is_not_cut_off() {
    // The last piece is whatever is left. Cutting again a few seconds from the end would make
    // a piece to send through a converter and get back for nothing.
    let keys = keyframes(2.0, 130.0);
    let cuts = choose(&[], &keys, 60.0, 130.0);
    let last = cuts.last().map(|c| c.at_s).unwrap_or(0.0);
    assert!(
        130.0 - last >= 60.0 * 0.7,
        "a scrap of {} s was left as its own piece",
        130.0 - last
    );
}

#[test]
fn a_file_whose_scenes_and_keyframes_disagree_says_so() {
    // T456. A broadcast stream, or NVENC with scene-cut detection off, or a file already cut
    // once: the keyframes sit on a fixed grid and the scenes fall between them. Every seam is
    // then a little away from where the picture changes — which is a decision between an
    // inexact cut and a re-encode, and it belongs to the person.
    let keys = keyframes(10.0, 600.0); // a hard ten-second group of pictures
    let scenes: Vec<(f64, f64)> = (1..12).map(|i| (i as f64 * 57.3, 0.8)).collect();
    let cuts = choose(&scenes, &keys, 60.0, 600.0);

    let missed = seams_that_missed(&cuts);
    assert!(
        !missed.is_empty(),
        "scenes on a 57.3 s rhythm against a 10 s keyframe grid, and nothing was reported"
    );
    for cut in &missed {
        assert!(cut.off_by_s > A_FRAME_S);
    }
}

#[test]
fn material_whose_keyframes_follow_its_scenes_reports_nothing() {
    // The negative control, and it is the case that was actually measured: 54 scene changes on
    // Blue Eye Samurai S01E04, all 54 exactly on a keyframe. If this reported anything, the
    // check above would be reporting on every file and would mean nothing.
    let scenes: Vec<(f64, f64)> = (1..12).map(|i| (i as f64 * 57.0, 0.8)).collect();
    let keys: Vec<f64> = scenes.iter().map(|(at, _)| *at).collect();
    let cuts = choose(&scenes, &keys, 60.0, 700.0);
    assert!(!cuts.is_empty());
    assert!(seams_that_missed(&cuts).is_empty());
}

#[test]
fn nothing_to_go_on_is_no_cuts_rather_than_a_guess() {
    // No keyframes read means the file was not understood. Cutting anyway would put every
    // boundary mid-group and turn a lossless job into a re-encode nobody asked for.
    assert!(choose(&[], &[], 60.0, 600.0).is_empty());
    assert!(choose(&[], &[0.0, 2.0], 0.0, 600.0).is_empty());
    assert!(choose(&[], &[0.0, 2.0], 60.0, 0.0).is_empty());
}
