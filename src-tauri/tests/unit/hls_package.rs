//! T196, T197 — reading what the cutting reports, and judging what is served.

use vrcast_studio_lib::domain::hls_master::{init_name, playlist_paths, segment_names};
use vrcast_studio_lib::domain::hls_package::{
    container_for, read_facts, read_log, script_text, Container, FactsProblem, SEGMENT_SECONDS,
};
use vrcast_studio_lib::server::hls_verify::{LadderVerdict, VariantVerdict};

fn a_variant(sub: &str) -> VariantVerdict {
    VariantVerdict {
        sub: sub.to_owned(),
        playlist_served: true,
        segments: 900,
        complete: true,
        first_segment_served: true,
        init_served: None,
        trouble: None,
    }
}

// ---------- how the cutting is told what to do ----------

#[test]
fn hevc_and_av1_cannot_go_into_a_transport_stream_and_do_not() {
    // Not a preference: neither can be wrapped in one at all, so for those it is fragmented
    // MP4 or nothing.
    assert_eq!(container_for("hevc"), Container::Fmp4);
    assert_eq!(container_for("HEVC"), Container::Fmp4);
    assert_eq!(container_for("av1"), Container::Fmp4);
    assert_eq!(container_for("h264"), Container::Ts);
    // An unknown codec takes the classic path rather than refusing: it is what most things
    // are, and being wrong here shows up at once rather than quietly.
    assert_eq!(container_for("something-else"), Container::Ts);

    assert_eq!(Container::Ts.extension(), "ts");
    assert_eq!(Container::Fmp4.extension(), "m4s");
}

#[test]
fn the_script_carries_the_segment_length_and_the_things_that_were_bought() {
    let script = script_text();
    assert!(
        script.contains(&format!("-hls_time {SEGMENT_SECONDS}")),
        "the segment length did not reach the script"
    );
    assert!(
        !script.contains("SEGSECONDS"),
        "the substitution was not made and the server would be handed a word"
    );

    // `-nostdin` is not a nicety: without it ffmpeg reads the rest of the script as its own
    // input and everything below it silently stops happening.
    assert_eq!(
        script.matches("ffmpeg -nostdin").count(),
        2,
        "both ways of cutting must keep ffmpeg's hands off the script"
    );
    // Independent segments, or a player cannot start at a segment boundary — which is the
    // whole point of cutting a ladder into segments.
    assert_eq!(script.matches("-hls_flags independent_segments").count(), 2);
    // Copying, never re-encoding: the variants were already encoded, and encoding them
    // again would both cost hours and lose quality that was measured.
    assert!(!script.contains("-c:v"), "the cutting must not re-encode");
    assert_eq!(script.matches("-c copy").count(), 2);
}

// ---------- reading what it says ----------

#[test]
fn the_log_says_which_variants_landed_and_whether_it_finished() {
    let log = "\
ffmpeg version 6.1
VRCAST_HLS_CUT v22
VRCAST_HLS_CUT v12
VRCAST_HLS_ALL_DONE
";
    let progress = read_log(log);
    assert_eq!(progress.cut, vec!["v22", "v12"]);
    assert!(progress.all_done);
    assert_eq!(progress.failed, None);
}

#[test]
fn a_cutting_that_stopped_halfway_is_not_read_as_finished() {
    // The marker is what says it finished, not the absence of complaints. A process can be
    // gone because it was killed, and the difference decides whether a person is shown a
    // ladder or an apology.
    let progress = read_log("VRCAST_HLS_CUT v22\n");
    assert_eq!(progress.cut, vec!["v22"]);
    assert!(!progress.all_done);

    let failed = read_log("VRCAST_HLS_CUT v22\nVRCAST_HLS_FAILED v12: no such file\n");
    assert_eq!(failed.failed.as_deref(), Some("v12: no such file"));
    assert!(!failed.all_done);
}

#[test]
fn what_a_variant_turned_out_to_be_is_read_back_whole() {
    let facts = "\
sub=v22
width=3840
height=2160
fps=23.976
level=51
codec=h264
seg 4.000 2013265
seg 4.000 1998877
seg 0.041 20134
";
    let read = read_facts(facts).expect("the facts would not read");
    assert_eq!(read.sub, "v22");
    assert_eq!((read.width, read.height), (3840, 2160));
    // The frame rate stays as written. Rounded to 24 it would be a lie about the material,
    // and it goes straight into the description a player reads.
    assert_eq!(read.frame_rate, "23.976");
    assert_eq!(read.level, "51");
    assert_eq!(read.segments.len(), 3);
    assert_eq!(read.segments[2].bytes, 20134);
}

#[test]
fn facts_with_something_missing_are_a_failure_rather_than_a_guess() {
    // A master built on a guessed level cuts the lowest rung off from the weak devices it
    // exists for. Guessing here would be quiet and expensive.
    let missing_level = "sub=v22\nwidth=3840\nheight=2160\nfps=24.000\ncodec=h264\n";
    assert_eq!(
        read_facts(missing_level),
        Err(FactsProblem::Incomplete("level"))
    );
    assert!(read_facts("").is_err());
}

// ---------- reading a description back ----------

#[test]
fn the_variants_of_a_master_are_found_even_in_a_mangled_one() {
    // Checking what is served must not depend on the description being well formed: a
    // person is better served by "this variant does not answer" than by "the description
    // could not be read".
    let master = "\
#EXTM3U
#EXT-X-VERSION:3
#EXT-X-STREAM-INF:BANDWIDTH=24000000,RESOLUTION=3840x2160
v22/stream.m3u8
#EXT-X-STREAM-INF:this line is nonsense
v12/stream.m3u8
";
    assert_eq!(
        playlist_paths(master),
        vec!["v22/stream.m3u8", "v12/stream.m3u8"]
    );
    assert!(playlist_paths("#EXTM3U\n").is_empty());
}

#[test]
fn the_segments_and_the_initialisation_piece_are_found_in_a_playlist() {
    let ts = "\
#EXTM3U
#EXT-X-TARGETDURATION:4
#EXTINF:4.000,
seg_00000.ts
#EXTINF:4.000,
seg_00001.ts
#EXT-X-ENDLIST
";
    assert_eq!(segment_names(ts), vec!["seg_00000.ts", "seg_00001.ts"]);
    assert_eq!(init_name(ts), None);

    let fmp4 = "\
#EXTM3U
#EXT-X-MAP:URI=\"init.mp4\"
#EXTINF:4.000,
seg_00000.m4s
#EXT-X-ENDLIST
";
    assert_eq!(segment_names(fmp4), vec!["seg_00000.m4s"]);
    assert_eq!(init_name(fmp4).as_deref(), Some("init.mp4"));
}

// ---------- judging what is served ----------

#[test]
fn success_means_every_variant_answered_and_not_merely_the_first() {
    // This project has already shipped a ladder with a half-empty variant nobody noticed,
    // because the check stopped at the first one.
    let mut whole = LadderVerdict {
        master_served: true,
        variants_in_master: 3,
        variants_expected: 3,
        variants: vec![a_variant("v22"), a_variant("v12"), a_variant("v6")],
    };
    assert!(whole.ok());
    assert!(whole.broken().is_empty());

    whole.variants[2].segments = 0;
    assert!(!whole.ok(), "a variant with no segments passed");
    assert_eq!(whole.broken(), vec!["v6"]);
}

#[test]
fn a_variant_that_never_says_where_it_ends_is_not_fit() {
    // Without `EXT-X-ENDLIST` a player treats it as a live stream that has not finished: it
    // waits for more, and a viewer sees it stall at the end rather than stop.
    let mut open = a_variant("v22");
    open.complete = false;
    assert!(!open.ok());
}

#[test]
fn fragmented_segments_without_their_init_are_not_fit_either() {
    // The segments carry no stream headers at all without it, and a playlist can name it
    // correctly while the file is not there — which looks identical until somebody presses
    // play.
    let mut no_init = a_variant("v22");
    no_init.init_served = Some(false);
    assert!(!no_init.ok());

    let mut with_init = a_variant("v22");
    with_init.init_served = Some(true);
    assert!(with_init.ok());
}

#[test]
fn a_master_missing_a_variant_is_not_a_success_even_if_what_it_names_all_works() {
    // Every variant in the master answering is not the same as every variant being in the
    // master. A rung that was built and then left out of the description is a rung nobody
    // will ever be given.
    let short = LadderVerdict {
        master_served: true,
        variants_in_master: 2,
        variants_expected: 3,
        variants: vec![a_variant("v22"), a_variant("v12")],
    };
    assert!(!short.ok());

    // And a master that is not served at all fails whatever it contains.
    let unserved = LadderVerdict {
        master_served: false,
        variants_in_master: 3,
        variants_expected: 3,
        variants: vec![a_variant("v22"), a_variant("v12"), a_variant("v6")],
    };
    assert!(!unserved.ok());
}
