//! T082 — договорные тесты команд заливки.
//!
//! Договор: `contracts/ipc-commands.md`, раздел «Заливка».
//!
//! Здесь проверяется только то, что видно снаружи: форма ответа, коды отказов
//! и то, какие из них снимаются подтверждением, а какие нет. Сама передача
//! проверяется на одноразовом сервере в контейнере — договор к ней отношения
//! не имеет.
//!
//! Все проверки идут по путям, которые обрываются **до** соединения с сервером:
//! договорный тест не должен зависеть от того, есть ли под рукой сеть.

use super::support::{state, valid_input};
use vrcast_studio_lib::commands::error::{DetailCode, ErrorCode};
use vrcast_studio_lib::commands::servers::api as servers;
use vrcast_studio_lib::commands::upload::{
    api as upload, space_error, warning_error, Preflight, SpaceShortage, UploadRequest,
};
use vrcast_studio_lib::commands::AppState;

/// Состояние с заведённым профилем.
///
/// Профиль нужен настоящий: заливка ищет его первым делом, и без него любая заявка
/// отвергается ещё до разбора остального. Тест, проверяющий отказ по имени файла
/// на несуществующем сервере, прошёл бы, ничего при этом не проверив.
fn state_with_server() -> (AppState, String) {
    let state = state();
    let id =
        servers::server_add(&state, valid_input("Сервер"), "пароль").expect("профиль не завёлся");
    (state, id)
}

/// Заведомо годная заявка. Тесты меняют то, что проверяют.
fn request(server_id: &str, local_path: &str) -> UploadRequest {
    UploadRequest {
        server_id: String::from(server_id),
        local_path: String::from(local_path),
        remote_name: String::from("film_22.mp4"),
        media_id: None,
        limit_bps: None,
        confirmed: false,
    }
}

fn temp_file(name: &str) -> std::path::PathBuf {
    let dir =
        std::env::temp_dir().join(format!("vrcast-contract-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir).expect("не создать временный каталог");
    let path = dir.join(name);
    std::fs::write(&path, "не видео, но файл").expect("не записать файл");
    path
}

#[tokio::test]
async fn заливка_на_несуществующий_сервер_отвергается_как_ошибка_ввода() {
    let state = state();
    let file = temp_file("film.mp4");

    let err = upload::upload_start(&state, request("нет-такого", &file.to_string_lossy()))
        .await
        .expect_err("заливка на несуществующий сервер прошла");

    assert_eq!(err.code, ErrorCode::InvalidInput);
    assert!(
        err.says(DetailCode::ProfileNotFound),
        "не сказано, что дело в сервере: {err}"
    );
}

#[tokio::test]
async fn отсутствующий_файл_называется_отдельно_от_поломок() {
    // Опечатка в пути — не сбой, и интерфейс обязан подсветить поле, а не показать
    // уведомление об ошибке. Различить одно от другого можно только по коду.
    let (state, id) = state_with_server();

    let err = upload::upload_start(&state, request(&id, "F:/такого/файла/нет.mp4"))
        .await
        .expect_err("заливка несуществующего файла прошла");

    assert_eq!(err.code, ErrorCode::InvalidInput);
    assert!(
        err.says(DetailCode::UploadFileUnreadable),
        "не сказано, что дело в файле: {err}"
    );
}

#[tokio::test]
async fn каталог_вместо_файла_отвергается() {
    let (state, id) = state_with_server();
    let dir = std::env::temp_dir().join(format!("vrcast-dir-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir).expect("не создать каталог");

    let err = upload::upload_start(&state, request(&id, &dir.to_string_lossy()))
        .await
        .expect_err("каталог принят за файл");

    assert_eq!(err.code, ErrorCode::InvalidInput);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn пустое_имя_в_раздаче_отвергается_до_соединения() {
    // Начать передачу под пустым именем нельзя: файл потом не найти ни по ссылке,
    // ни глазами. Проверка обязана сработать до соединения — иначе отказ придёт
    // через время ожидания сети, а профиль здесь нарочно указывает в никуда.
    let (state, id) = state_with_server();
    let file = temp_file("film.mp4");

    let mut req = request(&id, &file.to_string_lossy());
    // Одни пробелы: поле заполнено на вид, а после очистки в нём ничего нет.
    req.remote_name = String::from("   ");

    let started = std::time::Instant::now();
    let err = upload::upload_start(&state, req)
        .await
        .expect_err("пустое имя принято");

    assert_eq!(err.code, ErrorCode::InvalidInput);
    assert!(
        started.elapsed() < std::time::Duration::from_secs(2),
        "отказ по имени пришёл через {:?} — значит, до него успели полезть в сеть",
        started.elapsed()
    );
}

#[test]
fn имя_с_переходом_по_каталогам_остаётся_именем() {
    // Имя приходит из поля ввода и попадает в путь на сервере. Знаки разделения
    // не отвергаются, а заменяются: отказывать было бы вернее формально, но человек
    // просто вписал имя файла с диска — а вот выйти за каталог раздачи оно не должно
    // ни при каком написании.
    use vrcast_studio_lib::domain::remote_name::sanitize;

    for попытка in ["../../etc/passwd", "..\\..\\windows\\system32", "a/b/c.mp4"] {
        let clean = sanitize(попытка);
        assert!(
            !clean.contains('/') && !clean.contains('\\'),
            "в имени «{clean}» остался переход по каталогам"
        );
    }

    // И перевод строки: имя уходит в команду сервера, где вторая строка стала бы
    // отдельной командой.
    let clean = sanitize("film.mp4\nrm -rf /");
    assert!(
        !clean.contains('\n'),
        "в имени остался перевод строки: {clean}"
    );
}

#[test]
fn продолжение_несуществующей_задачи_не_молчит() {
    let state = state();
    let err = upload::upload_resume(&state, "нет-такой-задачи")
        .expect_err("продолжение несуществующей задачи прошло молча");
    // Код обязан быть узнаваемым: по нему интерфейс и найдёт, что сказать.
    assert_eq!(err.code, ErrorCode::TaskNotFound);
}

// ---------- отказы до начала передачи ----------

#[test]
fn нехватка_места_подтверждением_не_снимается() {
    // Разница между запретом и предупреждением — не оттенок вежливости. Если
    // нехватку места показать как предупреждение, у человека появится кнопка
    // «всё равно залить», после которой передача упрётся в конец диска
    // на середине тридцати гигабайт.
    let checks = Preflight {
        not_enough_space: Some(SpaceShortage {
            needed: 32 * 1024 * 1024 * 1024,
            free: 10 * 1024 * 1024 * 1024,
            short_by: 22 * 1024 * 1024 * 1024,
        }),
        active_connections: 0,
        name_exists: false,
        cdn_cached: false,
    };

    assert!(checks.is_blocking(), "нехватка места объявлена снимаемой");

    let err = space_error(checks.not_enough_space.unwrap());
    assert_eq!(err.code, ErrorCode::RemoteDiskFull);

    // Числа уходят как числа, а не как «22.0 ГБ»: единицы и разделитель дробной
    // части у языков разные, и выбирать их — дело интерфейса. Ядро отвечает за то,
    // что все три числа названы, — без них подтверждать нечего.
    let detail = err
        .details
        .iter()
        .find(|d| d.key == DetailCode::NotEnoughSpace)
        .unwrap_or_else(|| panic!("в отказе не названа нехватка: {err}"));
    for (name, expected) in [
        ("short_by", 23_622_320_128_u64),
        ("needed", 34_359_738_368),
        ("free", 10_737_418_240),
    ] {
        assert_eq!(
            detail.params.get(name).and_then(|v| v.as_u64()),
            Some(expected),
            "в отказе нет значения «{name}»: {detail:?}"
        );
    }
}

#[test]
fn идущий_просмотр_называется_своим_кодом_и_последствием() {
    let checks = Preflight {
        not_enough_space: None,
        active_connections: 3,
        name_exists: false,
        cdn_cached: false,
    };

    assert!(checks.has_warnings());
    assert!(!checks.is_blocking(), "предупреждение объявлено запретом");

    let err = warning_error(&checks, "film_22.mp4");
    assert_eq!(err.code, ErrorCode::ViewersActive);
    let detail = err
        .details
        .iter()
        .find(|d| d.key == DetailCode::ViewersActiveUpload)
        .unwrap_or_else(|| panic!("не сказано, что идёт просмотр: {err}"));
    assert_eq!(
        detail.params.get("connections").and_then(|v| v.as_u64()),
        Some(3),
        "не сказано, сколько соединений открыто: {detail:?}"
    );
}

#[test]
fn занятое_имя_называется_своим_кодом() {
    let checks = Preflight {
        not_enough_space: None,
        active_connections: 0,
        name_exists: true,
        cdn_cached: false,
    };

    let err = warning_error(&checks, "film_22.mp4");
    assert_eq!(err.code, ErrorCode::NameExists);
    let detail = err
        .details
        .iter()
        .find(|d| d.key == DetailCode::NameWillBeReplaced)
        .unwrap_or_else(|| panic!("не сказано, что файл будет заменён: {err}"));
    assert_eq!(
        detail.params.get("name").and_then(|v| v.as_str()),
        Some("film_22.mp4"),
        "не сказано, какой именно файл: {detail:?}"
    );
    assert!(
        !err.says(DetailCode::CdnKeepsOldCopy),
        "про кеш CDN сказано там, где CDN не задан: {err}"
    );
}

#[test]
fn при_заданном_кеше_замена_предупреждает_и_о_нём() {
    // Иначе человек заменит файл, откроет ссылку, увидит старое видео и решит,
    // что заливка не сработала.
    let checks = Preflight {
        not_enough_space: None,
        active_connections: 0,
        name_exists: true,
        cdn_cached: true,
    };

    let err = warning_error(&checks, "film_22.mp4");
    assert_eq!(err.code, ErrorCode::NameExists);
    assert!(
        err.says(DetailCode::CdnKeepsOldCopy),
        "про закешированную копию не сказано: {err}"
    );
}

#[test]
fn когда_предупреждать_не_о_чем_отказа_нет() {
    let checks = Preflight {
        not_enough_space: None,
        active_connections: 0,
        name_exists: false,
        cdn_cached: false,
    };
    assert!(!checks.has_warnings());
    assert!(!checks.is_blocking());
}

#[test]
fn проверки_до_старта_переживают_запись_и_чтение() {
    // Они уходят в интерфейс как есть — значит, обязаны переноситься без потерь.
    let checks = Preflight {
        not_enough_space: Some(SpaceShortage {
            needed: 100,
            free: 40,
            short_by: 60,
        }),
        active_connections: 2,
        name_exists: true,
        cdn_cached: true,
    };
    let json = serde_json::to_string(&checks).expect("не записалось");
    let back: Preflight = serde_json::from_str(&json).expect("не прочиталось");
    assert_eq!(back, checks);
}
