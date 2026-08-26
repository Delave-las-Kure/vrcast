//! T233, T234 — the parts of the quality measurement that can be checked without a film.
//!
//! Continuous integration has neither a graphics card nor two hours of video, and the
//! arithmetic is what goes wrong rather than the process.

use vrcast_studio_lib::media::ffmpeg::filter_present;
use vrcast_studio_lib::media::vmaf::{ceiling_mbps, pooled_mean};

/// The shape FFmpeg really prints, legend and all.
const LISTING: &str = "Filters:
  T.. = Timeline support
  .S. = Slice threading
  V = Video input/output
  | = Source or sink filter
  ------
 TS aap               AA->A      Apply Affine Projection algorithm to first audio stream.
 .. libvmaf           VV->V      Calculate the VMAF between two video streams.
 .. vmafmotion        V->V       Calculate the VMAF Motion score.
";

#[test]
fn the_filter_listing_is_read_by_column_rather_than_by_word() {
    assert!(filter_present(LISTING, "libvmaf"));
    assert!(filter_present(LISTING, "vmafmotion"));

    // `libvmaf` also stands in the description column of nothing here, but `VMAF` stands in
    // two of them. A word search over the whole text would find a filter called `VMAF`,
    // declare quality measurable and fall over at the first point of the grid.
    assert!(!filter_present(LISTING, "VMAF"));
    assert!(!filter_present(LISTING, "Calculate"));
    // The legend is not a list of filters.
    assert!(!filter_present(LISTING, "="));
    assert!(!filter_present(LISTING, "Timeline"));
}

#[test]
fn the_ceiling_stays_above_the_target_where_the_tenth_disappears() {
    // `MR=$(( BR * 11 / 10 ))` in integer arithmetic. Below ten megabits the tenth is lost
    // entirely and the ceiling comes out equal to the target — which is constant bitrate,
    // and constant bitrate lost the comparison this project ran.
    assert_eq!(ceiling_mbps(1), 2);
    assert_eq!(ceiling_mbps(5), 6);
    assert_eq!(ceiling_mbps(9), 10);
    // From ten upwards the tenth survives on its own.
    assert_eq!(ceiling_mbps(10), 11);
    assert_eq!(ceiling_mbps(22), 24);
    assert_eq!(ceiling_mbps(35), 38);

    // Whatever the rung, the ceiling is above it: a rung that cannot exceed its own target
    // is not being measured under the conditions it will be served under.
    for bitrate in 1..=100u64 {
        assert!(
            ceiling_mbps(bitrate) > bitrate,
            "the ceiling at {bitrate} did not clear the target"
        );
    }
}

#[test]
fn the_score_is_the_pooled_mean_and_nothing_else() {
    let report = r#"{
        "frames": [{"frameNum": 0, "metrics": {"vmaf": 71.5}}],
        "pooled_metrics": {
            "vmaf": {"min": 71.5, "max": 99.2, "mean": 96.104, "harmonic_mean": 95.8}
        }
    }"#;
    assert_eq!(pooled_mean(report).unwrap(), 96.104);
}

#[test]
fn a_report_with_no_score_in_it_is_not_a_score_of_zero() {
    // A quality of zero would be taken for a rung that looks terrible, and the hull would
    // step around it as a measured fact. It is not a fact — it is a failed measurement, and
    // the difference decides whether a bitrate is dropped from the ladder or retried.
    assert!(pooled_mean("{}").is_err());
    assert!(pooled_mean(r#"{"pooled_metrics": {}}"#).is_err());
    assert!(pooled_mean(r#"{"pooled_metrics": {"psnr": {"mean": 41.0}}}"#).is_err());
    assert!(pooled_mean("not json at all").is_err());
}
