//! T043 — разбор `server.env` при переносе настроек в первый профиль.
//!
//! Файл принадлежит прежнему порядку работы и продолжает им пользоваться
//! (конституция, принцип VII). Приложение только читает его — и обязано читать
//! правильно: неверно разобранный путь к ключу обернётся невозможностью войти
//! на сервер, а причина будет неочевидна.

use std::io::Write;
use vrcast_studio_lib::domain::server_profile::AuthKind;
use vrcast_studio_lib::server::env_import;

/// Записать временный файл с заданным содержимым.
fn temp_env(content: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("vrcast-env-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir).expect("не создать временный каталог");
    let path = dir.join("server.env");
    let mut f = std::fs::File::create(&path).expect("не создать файл");
    f.write_all(content.as_bytes()).expect("не записать файл");
    path
}

#[test]
fn настоящий_образец_файла_разбирается_целиком() {
    // Содержимое взято из server.env.example — того самого шаблона, которым
    // пользуется прежний порядок работы.
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

    let imported = env_import::read_from(&path).expect("файл не разобрался");
    let input = &imported.input;

    assert_eq!(input.host, "203.0.113.10");
    assert_eq!(input.domain, "stream.example.com");
    assert_eq!(input.user, "root");
    assert_eq!(input.video_dir.as_deref(), Some("/var/lib/vrcast/videos"));
    assert_eq!(input.auth_kind, AuthKind::Key);
    assert_eq!(
        input.cdn_base, None,
        "пустое значение CDN принято за адрес посредника"
    );

    // Хвостовой комментарий не должен попасть в значение: путь с припиской
    // «# IP VPS» не приведёт никуда.
    assert!(
        !input.host.contains('#'),
        "в значение попал комментарий: {}",
        input.host
    );

    // Парольной фразы в файле нет и быть не может — её вводит человек.
    assert!(imported.needs_passphrase);
}

#[test]
fn домашний_каталог_в_пути_к_ключу_разворачивается() {
    // В файле путь записан как «$HOME/.ssh/...» — оболочка развернула бы его сама,
    // а мы файл не выполняем и обязаны развернуть сами.
    let path = temp_env(
        r#"SERVER_IP="203.0.113.10"
SERVER_DOMAIN="stream.example.com"
SSH_KEY="$HOME/.ssh/vrcast_ed25519"
"#,
    );

    let imported = env_import::read_from(&path).expect("файл не разобрался");
    let key = imported.input.key_path.expect("путь к ключу потерян");
    assert!(
        !key.contains("$HOME") && !key.contains('~'),
        "домашний каталог не развёрнут: {key}"
    );
    assert!(key.ends_with(".ssh/vrcast_ed25519"), "путь испорчен: {key}");
}

#[test]
fn пароль_из_файла_не_переносится() {
    // В файле пароль — это запасной вход через консоль хостера, а не рабочий способ.
    // Перенести его значило бы приучить приложение ходить туда, куда не следует.
    let path = temp_env(
        r#"SERVER_IP="203.0.113.10"
SERVER_DOMAIN="stream.example.com"
SSH_KEY="/home/user/.ssh/k"
ROOT_PASSWORD="очень-секретный-пароль-из-файла"
"#,
    );

    let imported = env_import::read_from(&path).expect("файл не разобрался");
    let json = serde_json::to_string(&imported.input).expect("поля не сериализуются");
    assert!(
        !json.contains("очень-секретный-пароль-из-файла"),
        "пароль из файла попал в поля профиля: {json}"
    );
    assert_eq!(
        imported.input.auth_kind,
        AuthKind::Key,
        "при наличии ключа способ входа должен быть по ключу"
    );
}

#[test]
fn файл_без_адреса_или_домена_не_годится_для_переноса() {
    // Подставлять половину настроек хуже, чем не подставлять ничего: человек решит,
    // что всё заполнено, и не заметит пустого поля.
    let path = temp_env(r#"SSH_USER="root""#);
    assert!(env_import::read_from(&path).is_none());

    let path = temp_env(
        r#"SERVER_IP="203.0.113.10"
SSH_USER="root"
"#,
    );
    assert!(
        env_import::read_from(&path).is_none(),
        "принят файл без домена"
    );
}

#[test]
fn отсутствующий_файл_это_не_ошибка() {
    // У большинства пользователей приложения этого файла нет и не будет.
    let path = std::env::temp_dir().join("заведомо-нет-такого-файла-vrcast.env");
    assert!(env_import::read_from(&path).is_none());
}

#[test]
fn значение_без_кавычек_и_решётка_внутри_значения_разбираются_верно() {
    let path = temp_env(
        r#"SERVER_IP=203.0.113.10
SERVER_DOMAIN=stream.example.com
CDN_BASE="https://zone.example.net/#anchor"
VIDEO_DIR=/var/lib/vrcast/videos   # с комментарием
"#,
    );

    let imported = env_import::read_from(&path).expect("файл не разобрался");
    assert_eq!(imported.input.host, "203.0.113.10");
    assert_eq!(imported.input.domain, "stream.example.com");
    // Внутри кавычек решётка законна и обрезать по ней нельзя.
    assert_eq!(
        imported.input.cdn_base.as_deref(),
        Some("https://zone.example.net/#anchor")
    );
    // А без кавычек — это комментарий.
    assert_eq!(
        imported.input.video_dir.as_deref(),
        Some("/var/lib/vrcast/videos")
    );
}
