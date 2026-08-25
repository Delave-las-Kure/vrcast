//! T039 — полнота библиотеки: ни один файл не теряется.
//!
//! FR-015. Файл, который приложение не показало, никуда не делся: он занимает место
//! на диске и продолжает отдаваться по прямой ссылке. Скрыть его — худшее из
//! возможных решений, потому что пользователь считает библиотеку полной и удивляется,
//! куда ушло место.
//!
//! Проверяется равенство: число записей в каталоге раздачи (без служебных) равно
//! числу учтённых — файлов медиа плюс наборов качеств плюс группы «не распознано».

use super::fixture::{key_path, TestServer, KEY_PASSPHRASE};
use std::sync::Arc;
use vrcast_studio_lib::commands::library::api as library_api;
use vrcast_studio_lib::commands::servers::{api as servers_api, ServerInput};
use vrcast_studio_lib::commands::AppState;
use vrcast_studio_lib::domain::server_profile::AuthKind;
use vrcast_studio_lib::store::db::Db;
use vrcast_studio_lib::store::secrets::InMemorySecretStore;

const VIDEO_DIR: &str = "/var/lib/vrcast/videos";

/// Что кладём в каталог раздачи. Имена нарочно разные — с пробелом, с кириллицей,
/// с точками: приложение обязано справляться не только с образцовыми.
const FILES: [&str; 4] = [
    "Backrooms_10.mp4",
    "Backrooms_22.mp4",
    "одинокий ролик.mp4",
    "Blue.Eye.Samurai.S01E01.mp4",
];

fn app_state() -> AppState {
    AppState::with_db(
        Arc::new(Db::open_in_memory().unwrap()),
        Arc::new(InMemorySecretStore::new()),
    )
    .expect("состояние приложения не собралось")
}

fn profile_for(server: &TestServer) -> ServerInput {
    ServerInput {
        name: String::from("Контейнер"),
        host: server.host().to_owned(),
        port: server.port,
        user: String::from("root"),
        auth_kind: AuthKind::Key,
        key_path: Some(key_path().to_string_lossy().into_owned()),
        domain: String::from("stream.example.com"),
        video_dir: Some(String::from(VIDEO_DIR)),
        cdn_base: None,
        ipv6_mode: None,
    }
}

/// Разложить файлы, набор качеств, опись и служебный каталог.
fn prepare(server: &TestServer) {
    for name in FILES {
        server
            .exec_inside(&format!(
                "head -c 4096 /dev/urandom > '{VIDEO_DIR}/{name}'"
            ))
            .unwrap_or_else(|e| panic!("не создать {name}: {e}"));
    }

    // Набор качеств лежит каталогом — для пользователя это одна запись, а не сотня отрезков.
    server
        .exec_inside(&format!(
            "mkdir -p '{VIDEO_DIR}/backrooms' && \
             printf '#EXTM3U\\n' > '{VIDEO_DIR}/backrooms/master.m3u8' && \
             head -c 1024 /dev/urandom > '{VIDEO_DIR}/backrooms/seg1.ts'"
        ))
        .expect("не создать набор качеств");

    // Служебный каталог урезанных описаний: он во владении приложения и в библиотеку
    // не входит.
    server
        .exec_inside(&format!("mkdir -p '{VIDEO_DIR}/_slow'"))
        .expect("не создать служебный каталог");

    // Опись знает про два файла из четырёх и про набор качеств.
    let manifest = r#"{
      "generation": 3,
      "media": [
        { "id": "m_back", "title": "Backrooms", "slug": "backrooms",
          "files": ["Backrooms_10.mp4", "Backrooms_22.mp4"],
          "ladders": ["backrooms/master.m3u8"],
          "created_at": "2026-08-01T10:00:00Z" }
      ]
    }"#;
    server
        .exec_inside(&format!(
            "cat > '{VIDEO_DIR}/library.json' <<'КОНЕЦ'\n{manifest}\nКОНЕЦ"
        ))
        .expect("не записать опись");
}

#[tokio::test]
async fn ни_один_файл_каталога_не_теряется_в_библиотеке() {
    let server = TestServer::start().expect("контейнер не поднялся");
    prepare(&server);

    let state = app_state();
    let server_id =
        servers_api::server_add(&state, profile_for(&server), KEY_PASSPHRASE).expect("нет профиля");

    let view = library_api::library_list(&state, &server_id, true)
        .await
        .expect("библиотека не прочиталась");

    // Считаем средствами самого сервера: сверять число с тем же кодом, которым его
    // получили, значит не проверять ничего.
    let counted = server
        .exec_inside(&format!(
            "ls -A '{VIDEO_DIR}' | grep -v '^library.json$' | grep -v '^_slow$' | wc -l"
        ))
        .expect("не сосчитать записи каталога");
    let expected: usize = counted.trim().parse().expect("число не разобрать");

    assert_eq!(
        expected,
        FILES.len() + 1,
        "тест построен неверно: в каталоге не то, что ожидалось"
    );
    assert_eq!(
        view.accounted_entries(),
        expected,
        "часть записей каталога не показана пользователю: {view:?}"
    );
}

#[tokio::test]
async fn нераспознанные_файлы_показываются_отдельной_группой() {
    let server = TestServer::start().expect("контейнер не поднялся");
    prepare(&server);

    let state = app_state();
    let server_id =
        servers_api::server_add(&state, profile_for(&server), KEY_PASSPHRASE).expect("нет профиля");
    let view = library_api::library_list(&state, &server_id, true)
        .await
        .expect("библиотека не прочиталась");

    let unrecognized: Vec<&str> = view.unrecognized.iter().map(|f| f.path.as_str()).collect();
    assert!(
        unrecognized.contains(&"одинокий ролик.mp4"),
        "файл с пробелом в имени потерялся: {unrecognized:?}"
    );
    assert!(
        unrecognized.contains(&"Blue.Eye.Samurai.S01E01.mp4"),
        "файл вне описи не показан: {unrecognized:?}"
    );
    assert_eq!(unrecognized.len(), 2, "лишнее в группе: {unrecognized:?}");
}

#[tokio::test]
async fn опись_и_служебные_каталоги_не_показываются_как_видео() {
    // Иначе пользователь увидит в библиотеке «library.json» и «_slow» и будет думать,
    // что это его файлы.
    let server = TestServer::start().expect("контейнер не поднялся");
    prepare(&server);

    let state = app_state();
    let server_id =
        servers_api::server_add(&state, profile_for(&server), KEY_PASSPHRASE).expect("нет профиля");
    let view = library_api::library_list(&state, &server_id, true)
        .await
        .expect("библиотека не прочиталась");

    let all: Vec<&str> = view
        .media
        .iter()
        .flat_map(|m| m.files.iter())
        .chain(view.unrecognized.iter())
        .map(|f| f.path.as_str())
        .collect();

    for служебное in ["library.json", "_slow"] {
        assert!(
            !all.iter().any(|p| p.starts_with(служебное)),
            "служебное «{служебное}» показано как видео: {all:?}"
        );
    }
}

#[tokio::test]
async fn файл_из_описи_которого_нет_на_сервере_помечается_пропавшим() {
    // FR-018: файл удалили мимо приложения. Опись про него ещё помнит — и ссылка
    // на него не должна показываться рабочей.
    let server = TestServer::start().expect("контейнер не поднялся");
    prepare(&server);
    server
        .exec_inside(&format!("rm '{VIDEO_DIR}/Backrooms_10.mp4'"))
        .expect("файл не удалился");

    let state = app_state();
    let server_id =
        servers_api::server_add(&state, profile_for(&server), KEY_PASSPHRASE).expect("нет профиля");
    let view = library_api::library_list(&state, &server_id, true)
        .await
        .expect("библиотека не прочиталась");

    let media = view
        .media
        .iter()
        .find(|m| m.slug == "backrooms")
        .expect("медиа пропало вместе с файлом");
    let missing = media
        .files
        .iter()
        .find(|f| f.path == "Backrooms_10.mp4")
        .expect("пропавший файл исчез из медиа — пользователь не узнает о потере");

    assert!(
        !missing.exists_on_server,
        "удалённый файл считается существующим"
    );
    let present = media
        .files
        .iter()
        .find(|f| f.path == "Backrooms_22.mp4")
        .expect("уцелевший файл пропал");
    assert!(present.exists_on_server);
}

#[tokio::test]
async fn параметры_файлов_читаются_из_заголовка_а_не_придумываются() {
    // FR-012 и R-19: разрешение, длительность и кодеки берутся разбором начала файла.
    // У наших заготовок из случайных байтов заголовка нет — и приложение обязано
    // честно сказать «неизвестно», а не подставить правдоподобные числа.
    let server = TestServer::start().expect("контейнер не поднялся");
    prepare(&server);

    let state = app_state();
    let server_id =
        servers_api::server_add(&state, profile_for(&server), KEY_PASSPHRASE).expect("нет профиля");
    let view = library_api::library_list(&state, &server_id, true)
        .await
        .expect("библиотека не прочиталась");

    let any = view
        .media
        .iter()
        .flat_map(|m| m.files.iter())
        .chain(view.unrecognized.iter())
        .next()
        .expect("в библиотеке нет ни одного файла");

    assert!(any.size_bytes > 0, "размер файла не прочитан");
    assert_eq!(
        any.width, None,
        "разрешение взялось из ниоткуда: заголовка в этом файле нет"
    );
    assert_eq!(any.duration_s, None, "длительность придумана");
}

#[tokio::test]
async fn место_на_диске_сервера_показывается() {
    // FR-017.
    let server = TestServer::start().expect("контейнер не поднялся");
    prepare(&server);

    let state = app_state();
    let server_id =
        servers_api::server_add(&state, profile_for(&server), KEY_PASSPHRASE).expect("нет профиля");
    let view = library_api::library_list(&state, &server_id, true)
        .await
        .expect("библиотека не прочиталась");

    let disk = view.disk.expect("место на диске не показано");
    assert!(disk.total_bytes > 0, "объём диска не прочитан");
    assert!(
        disk.free_bytes <= disk.total_bytes,
        "свободного больше, чем всего: {disk:?}"
    );
    assert!(
        disk.used_by_videos_bytes > 0,
        "объём каталога раздачи не посчитан, хотя файлы там есть"
    );
}

#[tokio::test]
async fn при_недоступном_сервере_показывается_последнее_известное_с_пометкой() {
    // Пустой экран или бесконечная загрузка — худший ответ: пользователь не понимает,
    // потерял он библиотеку или связь.
    let server = TestServer::start().expect("контейнер не поднялся");
    prepare(&server);

    let state = app_state();
    let server_id =
        servers_api::server_add(&state, profile_for(&server), KEY_PASSPHRASE).expect("нет профиля");

    let свежее = library_api::library_list(&state, &server_id, true)
        .await
        .expect("библиотека не прочиталась");
    assert!(!свежее.stale, "свежие данные помечены устаревшими");

    // Роняем сервер и спрашиваем снова.
    drop(server);
    let после = library_api::library_list(&state, &server_id, true)
        .await
        .expect("на недоступном сервере библиотека обязана прийти из кеша, а не ошибкой");

    assert!(после.stale, "устаревшие данные не помечены");
    assert_eq!(
        после.accounted_entries(),
        свежее.accounted_entries(),
        "кеш потерял часть записей"
    );
}
