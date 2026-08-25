//! T043 — parsing `server.env` when carrying settings over into a first profile.
//!
//! The file belongs to the old way of working and goes on being used by it (constitution,
//! principle VII). The application only reads it — and must read it correctly: a wrongly
//! parsed path to a key turns into being unable to log in to the server, for a reason that
//! is not obvious.

use std::io::Write;
use vrcast_studio_lib::domain::server_profile::AuthKind;
use vrcast_studio_lib::server::env_import;

/// Write a temporary file with the given contents.
fn temp_env(content: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("vrcast-env-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir).expect("could not create the temporary directory");
    let path = dir.join("server.env");
    let mut f = std::fs::File::create(&path).expect("could not create the file");
    f.write_all(content.as_bytes())
        .expect("could not write the file");
    path
}

#[test]
fn a_real_sample_of_the_file_parses_whole() {
    // The contents are taken from server.env.example — the very template the old way of
    // working uses, Russian comments and all: the parser meets it exactly like this.
    let path = temp_env(
        r#"
# --- сервер ---
SERVER_IP="203.0.113.10"                       # IP VPS
SERVER_DOMAIN="stream.example.com"             # домен (HTTPS через Caddy/Let's Encrypt)
SSH_USER="root"
SSH_KEY="$HOME/.ssh/vrcast_ed25519"            # приватный ключ
ROOT_PASSWORD=""                               # пусто = запасного пароля нет вообще

# --- пути на сервере ---
VIDEO_DIR="/var/lib/vrcast/videos"             # каталог раздаваемых файлов
VIDEO_OWNER="vrcast:vrcast"

# --- CDN ---
CDN_BASE=""
"#,
    );

    let imported = env_import::read_from(&path).expect("the file would not parse");
    let input = &imported.input;

    assert_eq!(input.host, "203.0.113.10");
    assert_eq!(input.domain, "stream.example.com");
    assert_eq!(input.user, "root");
    assert_eq!(input.video_dir.as_deref(), Some("/var/lib/vrcast/videos"));
    assert_eq!(input.auth_kind, AuthKind::Key);
    assert_eq!(
        input.cdn_base, None,
        "an empty CDN value was taken for a middleman's address"
    );

    // A trailing comment must not reach the value: an address with "# IP VPS" tacked on
    // leads nowhere.
    assert!(
        !input.host.contains('#'),
        "в значение попал комментарий: {}",
        "a comment reached the value: {}",
    );

    // There is no passphrase in the file and cannot be — a person types that in.
    assert!(imported.needs_passphrase);
}

#[test]
fn a_home_directory_in_the_key_path_is_expanded() {
    // In the file the path is written as "$HOME/.ssh/..." — a shell would expand it itself,
    // and we do not run the file, so we must expand it ourselves.
    let path = temp_env(
        r#"SERVER_IP="203.0.113.10"
SERVER_DOMAIN="stream.example.com"
SSH_KEY="$HOME/.ssh/vrcast_ed25519"
"#,
    );

    let imported = env_import::read_from(&path).expect("the file would not parse");
    let key = imported
        .input
        .key_path
        .expect("the path to the key was lost");
    assert!(
        !key.contains("$HOME") && !key.contains('~'),
        "the home directory was not expanded: {key}"
    );
    assert!(
        key.ends_with(".ssh/vrcast_ed25519"),
        "the path is spoilt: {key}"
    );
}

#[test]
fn the_password_from_the_file_is_not_carried_over() {
    // In the file the password is the fallback way in through the hosting console rather
    // than a working one. Carrying it over would teach the application to go where it
    // should not.
    let path = temp_env(
        r#"SERVER_IP="203.0.113.10"
SERVER_DOMAIN="stream.example.com"
SSH_KEY="/home/user/.ssh/k"
ROOT_PASSWORD="очень-секретный-пароль-из-файла"
"#,
    );

    let imported = env_import::read_from(&path).expect("the file would not parse");
    let json = serde_json::to_string(&imported.input).expect("the fields will not serialise");
    assert!(
        !json.contains("очень-секретный-пароль-из-файла"),
        "the password from the file reached the profile's fields: {json}"
    );
    assert_eq!(
        imported.input.auth_kind,
        AuthKind::Key,
        "with a key present, the way in must be by key"
    );
}

#[test]
fn a_file_with_no_address_or_domain_is_unfit_to_carry_over() {
    // Filling in half the settings is worse than filling in none: a person decides
    // everything is filled and does not notice the empty field.
    let path = temp_env(r#"SSH_USER="root""#);
    assert!(env_import::read_from(&path).is_none());

    let path = temp_env(
        r#"SERVER_IP="203.0.113.10"
SSH_USER="root"
"#,
    );
    assert!(
        env_import::read_from(&path).is_none(),
        "a file with no domain was accepted"
    );
}

#[test]
fn a_missing_file_is_not_an_error() {
    // Most people using the application have no such file and never will.
    let path = std::env::temp_dir().join("definitely-no-such-file-vrcast.env");
    assert!(env_import::read_from(&path).is_none());
}

#[test]
fn an_unquoted_value_and_a_hash_inside_a_value_parse_correctly() {
    let path = temp_env(
        r#"SERVER_IP=203.0.113.10
SERVER_DOMAIN=stream.example.com
CDN_BASE="https://zone.example.net/#anchor"
VIDEO_DIR=/var/lib/vrcast/videos   # с комментарием
"#,
    );

    let imported = env_import::read_from(&path).expect("the file would not parse");
    assert_eq!(imported.input.host, "203.0.113.10");
    assert_eq!(imported.input.domain, "stream.example.com");
    // Inside quotes a hash is legitimate and must not cut the value short.
    assert_eq!(
        imported.input.cdn_base.as_deref(),
        Some("https://zone.example.net/#anchor")
    );
    // Without quotes it is a comment.
    assert_eq!(
        imported.input.video_dir.as_deref(),
        Some("/var/lib/vrcast/videos")
    );
}
