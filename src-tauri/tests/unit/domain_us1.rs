//! T034 — tests for the pure logic of the first user story.
//!
//! What is checked is not "the function returned something" but the rules everything else
//! leans on: a profile with no domain cannot be saved, a short name goes into a file name,
//! the catalogue's generation guards against a second copy of the application, a link does
//! not break on an unusual file name, and no file is lost in the grouping.

use vrcast_studio_lib::domain::grouping::{self, GroupReason};
use vrcast_studio_lib::domain::links;
use vrcast_studio_lib::domain::manifest::{Manifest, ManifestProblem};
use vrcast_studio_lib::domain::media::{self, Media, MediaFile, SlugError};
use vrcast_studio_lib::domain::server_profile::{
    AuthKind, ServerProfile, DEFAULT_SSH_PORT, DEFAULT_VIDEO_DIR,
};
use vrcast_studio_lib::domain::wording::DetailCode;

// ---------- the server profile (T029) ----------

fn valid_profile() -> ServerProfile {
    let mut p = ServerProfile::new("srv_1", "My server");
    p.host = String::from("203.0.113.10");
    p.user = String::from("root");
    p.auth_kind = AuthKind::Key;
    p.key_path = Some(String::from("/home/user/.ssh/id_ed25519"));
    p.secret_ref = String::from("vrcast/srv_1/passphrase");
    p.domain = String::from("stream.example.com");
    p
}

#[test]
fn a_correct_profile_passes_validation() {
    let p = valid_profile();
    assert!(p.validate().is_ok(), "{:?}", p.validate());
    assert_eq!(p.port, DEFAULT_SSH_PORT);
    assert_eq!(p.video_dir, DEFAULT_VIDEO_DIR);
}

#[test]
fn a_profile_with_no_domain_cannot_be_saved() {
    // The domain is required for more than looks: without it there is no viewer's link to
    // hand out and no way to check that the serving works at all (FR-125).
    let mut p = valid_profile();
    p.domain = String::new();

    let problems = p
        .validate()
        .expect_err("a profile with no domain was accepted");
    assert!(
        problems.iter().any(|x| x.field == "domain"),
        "the domain field is not named: {problems:?}"
    );
    // The objection names the case with a code: the interface picks the wording, and it
    // exists in both languages (FR-105, FR-106).
    assert_eq!(problems[0].detail.key, DetailCode::DomainEmpty);
}

#[test]
fn validation_names_every_error_at_once_rather_than_the_first() {
    // In the setup wizard a person fills in the whole form. Showing the errors one at a time
    // sends them round in circles over every typo.
    let mut p = valid_profile();
    p.name = String::new();
    p.host = String::new();
    p.domain = String::new();
    p.user = String::new();

    let problems = p.validate().expect_err("an empty profile was accepted");
    let fields: Vec<&str> = problems.iter().map(|x| x.field).collect();
    for expected in ["name", "host", "domain", "user"] {
        assert!(
            fields.contains(&expected),
            "the field {expected} is not named: {fields:?}"
        );
    }
}

#[test]
fn a_domain_pasted_from_the_address_bar_is_normalised() {
    // People paste a domain along with "https://" and a slash. Rejecting it for that is
    // pedantry: the intent is unambiguous.
    let mut p = valid_profile();
    p.domain = String::from("  HTTPS://Stream.Example.COM/  ");
    p.normalize();

    assert_eq!(p.domain, "stream.example.com");
    assert!(p.validate().is_ok());
}

#[test]
fn a_domain_with_a_path_is_rejected() {
    // A path, though, cannot be normalised away: what the person meant is unclear.
    let mut p = valid_profile();
    p.domain = String::from("https://stream.example.com/videos");
    p.normalize();

    let problems = p.validate().expect_err("a domain with a path was accepted");
    assert!(problems.iter().any(|x| x.field == "domain"));
}

#[test]
fn a_path_holding_two_dots_is_not_allowed() {
    // A path from here goes into commands on the server: one ".." segment takes an entry
    // outside the serving directory.
    let mut p = valid_profile();
    p.video_dir = String::from("/var/lib/vrcast/../../etc");

    let problems = p.validate().expect_err("a path with \"..\" was accepted");
    assert!(problems.iter().any(|x| x.field == "video_dir"));
}

#[test]
fn login_by_key_needs_a_key_path_while_login_by_password_forbids_one() {
    let mut p = valid_profile();
    p.key_path = None;
    assert!(
        p.validate()
            .expect_err("login by key with no key was accepted")
            .iter()
            .any(|x| x.field == "key_path"),
        "the path to the key was not demanded"
    );

    // With login by password the key path is dropped by normalising rather than counted an
    // error: the person may simply have switched the way in.
    let mut p = valid_profile();
    p.auth_kind = AuthKind::Password;
    p.normalize();
    assert_eq!(p.key_path, None, "the key path stayed on login by password");
    assert!(p.validate().is_ok());
}

#[test]
fn a_profile_has_nowhere_to_hold_a_secret() {
    // Constitution, principle IV. It is not the intent that is checked but the shape of the
    // record: what goes to disk must hold nothing resembling the secret itself.
    let mut p = valid_profile();
    p.secret_ref = String::from("vrcast/srv_1/passphrase");
    let json = serde_json::to_string(&p).unwrap();

    assert!(
        json.contains("secret_ref"),
        "the pointer to the secret must be kept"
    );
    for forbidden in ["password", "passphrase\":", "secret\":"] {
        assert!(
            !json.contains(forbidden),
            "the profile holds a field for the secret itself ({forbidden}): {json}"
        );
    }
}

// ---------- the short name (T030) ----------

#[test]
fn a_short_name_allows_only_safe_characters() {
    assert!(media::validate_slug("nazvanie-filma").is_ok());
    assert!(media::validate_slug("Backrooms_22").is_ok());

    // Non-Latin in a file name and in a link is trouble for nothing.
    assert!(matches!(
        media::validate_slug("название"),
        Err(SlugError::BadChars { .. })
    ));
    // A slash would take the file into another directory.
    assert!(matches!(
        media::validate_slug("a/b"),
        Err(SlugError::BadChars { first_bad: '/' })
    ));
    assert!(matches!(media::validate_slug(""), Err(SlugError::Empty)));
    assert!(matches!(
        media::validate_slug(".."),
        Err(SlugError::BadChars { first_bad: '.' })
    ));
    assert!(matches!(
        media::validate_slug("_slow"),
        Err(SlugError::Reserved),
    ));
}

#[test]
fn a_short_name_is_made_from_a_title_in_the_person_s_own_language() {
    // The very example from the server contract.
    assert_eq!(
        media::slugify("Название фильма").as_deref(),
        Some("nazvanie-filma")
    );
    assert_eq!(
        media::slugify("Щи да каша — пища наша!").as_deref(),
        Some("schi-da-kasha-pischa-nasha")
    );
    // Separators do not pile up: consecutive spaces and marks give one hyphen.
    assert_eq!(
        media::slugify("  Один   —   Два  ").as_deref(),
        Some("odin-dva")
    );
    // A name that was made must pass validation of its own — otherwise the application
    // would offer what it would later reject itself.
    let slug = media::slugify("Ёжик в тумане").expect("the name would not be made");
    assert!(media::validate_slug(&slug).is_ok(), "made \"{slug}\"");
    assert_eq!(slug, "ezhik-v-tumane");
}

#[test]
fn a_title_with_no_latin_counterpart_yields_no_invented_name() {
    // Better to ask a person than to put in rubbish that goes into a file name and into a
    // link.
    assert_eq!(media::slugify("日本語"), None);
    assert_eq!(media::slugify("!!! ??? ..."), None);
    assert_eq!(media::slugify(""), None);
}

#[test]
fn a_link_to_a_file_that_is_gone_does_not_count_as_working() {
    // FR-018: the file was deleted outside the application — the link must not be shown.
    let mut f = MediaFile::known("Backrooms_22.mp4", 1024);
    assert!(f.link_is_usable());
    f.exists_on_server = false;
    assert!(!f.link_is_usable());
}

// ---------- the catalogue and its generation (T031) ----------

fn media_entry(id: &str, slug: &str, files: &[&str]) -> Media {
    let mut m = Media::new(id, slug, slug, "2026-08-01T10:00:00Z");
    m.files = files.iter().map(|s| (*s).to_owned()).collect();
    m
}

#[test]
fn a_catalogue_reads_and_writes_in_the_same_shape() {
    let text = r#"{
      "generation": 42,
      "media": [
        { "id": "m_a1b2", "title": "The film's title", "slug": "nazvanie-filma",
          "files": ["nazvanie-filma_22.mp4", "nazvanie-filma_9.mp4"],
          "ladders": ["nazvanie-filma/master.m3u8"],
          "created_at": "2026-08-01T10:00:00Z" }
      ]
    }"#;

    let m = Manifest::parse(text).expect("the catalogue would not read");
    assert_eq!(m.generation, 42);
    assert_eq!(m.media.len(), 1);
    assert_eq!(m.media[0].files.len(), 2);
    assert_eq!(m.media[0].ladders[0], "nazvanie-filma/master.m3u8");

    let again = Manifest::parse(&m.to_json()).expect("our own record would not read back");
    assert_eq!(again, m, "writing and reading disagree");
}

#[test]
fn a_missing_catalogue_is_an_empty_library_rather_than_a_fault() {
    // On a fresh server the file is not there yet. Failing here would declare an empty
    // library a malfunction.
    let m = Manifest::parse("").expect("empty contents were not accepted");
    assert_eq!(m.generation, 0);
    assert!(m.media.is_empty());
}

#[test]
fn unfamiliar_catalogue_fields_survive_a_rewrite() {
    // The catalogue may have been written by a newer copy of the application. Quietly
    // throwing away what is not understood is the quietest way to lose what somebody else
    // recorded (FR-131).
    let text = r#"{
      "generation": 7,
      "media": [{ "id": "m1", "title": "t", "slug": "t", "files": [], "ladders": [],
                  "created_at": "2026-08-01T10:00:00Z", "field_from_the_future": 5 }],
      "catalogue_from_the_future": { "something": "important" }
    }"#;

    let m = Manifest::parse(text).unwrap();
    let written = m.prepared_for_write().to_json();

    assert!(
        written.contains("catalogue_from_the_future") && written.contains("important"),
        "an unfamiliar catalogue field was lost: {written}"
    );
    assert!(
        written.contains("field_from_the_future"),
        "an unfamiliar medium field was lost: {written}"
    );
}

#[test]
fn the_generation_grows_by_one_on_a_write() {
    let m = Manifest {
        generation: 42,
        media: vec![media_entry("m1", "film", &["film_22.mp4"])],
        ..Manifest::empty()
    };
    let next = m.prepared_for_write();

    assert_eq!(next.generation, 43);
    assert_eq!(
        m.generation, 42,
        "the original catalogue was changed in place"
    );
    assert_eq!(
        next.media, m.media,
        "changing the generation touched the contents"
    );
}

#[test]
fn a_write_is_allowed_only_when_the_generation_has_not_changed() {
    // Exactly the case the counter exists for: two copies of the application against one
    // server. The second must not wipe out the first one's work.
    assert!(Manifest::write_allowed(42, 42));
    assert!(
        !Manifest::write_allowed(42, 43),
        "a write over somebody else's was allowed"
    );
    assert!(
        !Manifest::write_allowed(42, 41),
        "a write was allowed with the generation rolled back — that is a divergence too"
    );
}

#[test]
fn a_self_contradictory_catalogue_fails_validation() {
    let m = Manifest {
        generation: 1,
        media: vec![
            media_entry("m1", "film", &["shared.mp4"]),
            media_entry("m2", "film", &["shared.mp4"]),
        ],
        ..Manifest::empty()
    };

    let problems = m
        .validate()
        .expect_err("a self-contradictory catalogue was accepted");
    assert!(
        problems
            .iter()
            .any(|p| matches!(p, ManifestProblem::DuplicateSlug(s) if s == "film")),
        "a repeated short name went unnoticed: {problems:?}"
    );
    // A file counted under two media is no trifle: deleting one takes the file from the
    // other as well.
    assert!(
        problems.iter().any(
            |p| matches!(p, ManifestProblem::FileClaimedTwice { path, .. } if path == "shared.mp4")
        ),
        "a file counted twice went unnoticed: {problems:?}"
    );
}

#[test]
fn whether_a_short_name_is_taken_allows_for_renaming_oneself() {
    let m = Manifest {
        generation: 1,
        media: vec![media_entry("m1", "film", &[])],
        ..Manifest::empty()
    };

    assert!(
        !m.slug_available("film", None),
        "a name already taken was declared free"
    );
    assert!(m.slug_available("other", None));
    // A medium does not conflict with itself: otherwise a rename form could not be saved
    // without changing the short name.
    assert!(m.slug_available("film", Some("m1")));
    assert!(!m.slug_available("film", Some("m2")));
}

// ---------- links (T032) ----------

#[test]
fn a_link_is_built_from_the_domain_and_the_file_name() {
    let l = links::for_path("stream.example.com", None, "Backrooms_22.mp4");
    assert_eq!(
        l.origin,
        "https://stream.example.com/videos/Backrooms_22.mp4"
    );
    assert_eq!(l.cdn, None, "with no CDN there must be no second link");
    assert_eq!(l.preferred(), l.origin);
}

#[test]
fn with_a_cdn_set_both_links_are_handed_out() {
    // FR-016: the choice is left to a person — the options cost differently.
    let l = links::for_path(
        "stream.example.com",
        Some("https://cdn.example.net/"),
        "backrooms/master.m3u8",
    );
    assert_eq!(
        l.origin,
        "https://stream.example.com/videos/backrooms/master.m3u8"
    );
    assert_eq!(
        l.cdn.as_deref(),
        Some("https://cdn.example.net/videos/backrooms/master.m3u8"),
        "the CDN's trailing slash was doubled"
    );
}

#[test]
fn an_unusual_file_name_does_not_break_the_link() {
    // A hash turns the rest of the name into an anchor and the link leads nowhere — quietly.
    // A space tears it in half when copied. The name is deliberately not Latin: that is the
    // case this is about.
    let l = links::for_path("stream.example.com", None, "Фильм №1 #финал.mp4");

    assert!(
        !l.origin.contains('#') && !l.origin.contains(' '),
        "dangerous characters were left in the link: {}",
        l.origin
    );
    assert!(
        l.origin.starts_with("https://stream.example.com/videos/"),
        "the link was built wrongly: {}",
        l.origin
    );
    // Directory separators must not be encoded — the path would turn into one name.
    let nested = links::for_path("stream.example.com", None, "мультик/master.m3u8");
    assert!(
        nested.origin.ends_with("/master.m3u8"),
        "the directory separator was encoded: {}",
        nested.origin
    );
}

#[test]
fn a_domain_with_a_scheme_does_not_double_it_in_the_link() {
    // The data also comes from a database written by an earlier version — normalising has to
    // work here too, not only in the form.
    let l = links::for_path("https://stream.example.com/", None, "a.mp4");
    assert_eq!(l.origin, "https://stream.example.com/videos/a.mp4");
}

// ---------- grouping by name (T033) ----------

fn owned(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| (*s).to_owned()).collect()
}

#[test]
fn bitrate_variants_come_together_into_one_medium() {
    let files = owned(&[
        "Backrooms_10.mp4",
        "Backrooms_22.mp4",
        "Backrooms_35.mp4",
        "Another_22.mp4",
        "Another_10.mp4",
    ]);
    let s = grouping::suggest(&files);

    assert_eq!(s.groups.len(), 2, "groups: {:?}", s.groups);
    assert_eq!(s.groups[0].key, "Backrooms");
    assert_eq!(s.groups[0].files.len(), 3);
    assert_eq!(s.groups[0].reason, GroupReason::BitrateVariants);
    assert!(s.singles.is_empty(), "surplus singletons: {:?}", s.singles);
}

#[test]
fn files_in_one_directory_are_a_quality_ladder() {
    let files = owned(&[
        "backrooms/master.m3u8",
        "backrooms/v22/seg1.ts",
        "backrooms/v10/seg1.ts",
    ]);
    let s = grouping::suggest(&files);

    assert_eq!(s.groups.len(), 1);
    assert_eq!(s.groups[0].key, "backrooms");
    assert_eq!(s.groups[0].reason, GroupReason::SameDirectory);
    assert_eq!(s.groups[0].files.len(), 3);
}

#[test]
fn a_lone_file_does_not_become_a_group_but_does_not_vanish_either() {
    // A lone `Backrooms_22.mp4` with no neighbours proves nothing — there are no grounds for
    // creating a medium for it unbidden. But hiding it will not do either (FR-015).
    let files = owned(&["Backrooms_22.mp4", "just-a-clip.mp4"]);
    let s = grouping::suggest(&files);

    assert!(
        s.groups.is_empty(),
        "groups made of one file: {:?}",
        s.groups
    );
    assert_eq!(s.singles.len(), 2, "files were lost: {:?}", s);
}

#[test]
fn not_one_file_is_lost_in_the_grouping() {
    // The property the library's completeness check leans on: the number of files in the
    // directory equals the sum over the media plus the "not recognised" group.
    let files = owned(&[
        "A_10.mp4",
        "A_22.mp4",
        "dir/master.m3u8",
        "dir/seg.ts",
        "singleton.mp4",
        "B_35.mp4",
        "odd_name_with_no_number.mp4",
        "_leading_underscore_1.mp4",
    ]);
    let s = grouping::suggest(&files);

    assert_eq!(
        s.total_files(),
        files.len(),
        "some files vanished in the grouping: {s:?}"
    );

    // And not one may land in two places at once.
    let mut seen: Vec<&str> = s
        .groups
        .iter()
        .flat_map(|g| g.files.iter())
        .chain(s.singles.iter())
        .map(String::as_str)
        .collect();
    seen.sort_unstable();
    let before = seen.len();
    seen.dedup();
    assert_eq!(before, seen.len(), "a file landed in two groups at once");
}

#[test]
fn the_suggested_title_is_readable() {
    let files = owned(&["Blue_Eye_Samurai_10.mp4", "Blue_Eye_Samurai_22.mp4"]);
    let s = grouping::suggest(&files);

    assert_eq!(s.groups[0].suggested_title, "Blue Eye Samurai");
    assert_eq!(
        s.groups[0].key, "Blue_Eye_Samurai",
        "the short name must stay fit for a file name"
    );
}
