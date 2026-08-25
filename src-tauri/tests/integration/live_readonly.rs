//! T060 — сценарий 1 из `quickstart.md` на настоящем сервере, **только на чтение**.
//!
//! Это приёмочная проверка вехи A, а не часть обычного набора: она помечена
//! `#[ignore]` и запускается только по прямой просьбе:
//!
//! ```text
//! cargo test --features integration --test integration -- --ignored --nocapture живой_сервер
//! ```
//!
//! **Что она делает с сервером: ничего.** Перечисляет каталог, читает опись, читает
//! начала файлов, спрашивает место на диске. Ни одной записи — конституция запрещает
//! проверять на боевом сервере то, что меняет его состояние: там идёт настоящая
//! раздача, и оборвать её ради проверки недопустимо.
//!
//! Шаги сценария, которые меняют сервер (переименование короткого имени, удаление
//! с подтверждением), проверяются против одноразового контейнера — `library_ops.rs`.
//! Смысл этой проверки в другом: убедиться, что приложение справляется с настоящей
//! библиотекой, а не только с той, которую само же и разложило.
//!
//! Настройки берутся из `server.env` через тот же перенос, что предлагается
//! пользователю (T043) — так заодно проверяется и он. Секрет в код теста не попадает.

use std::sync::Arc;
use vrcast_studio_lib::commands::library::api as library;
use vrcast_studio_lib::commands::servers::{api as servers, StepStatus};
use vrcast_studio_lib::commands::AppState;
use vrcast_studio_lib::server::env_import;
use vrcast_studio_lib::store::db::Db;
use vrcast_studio_lib::store::secrets::InMemorySecretStore;

fn state() -> AppState {
    AppState::with_db(
        Arc::new(Db::open_in_memory().unwrap()),
        Arc::new(InMemorySecretStore::new()),
    )
    .expect("состояние приложения не собралось")
}

#[tokio::test]
#[ignore = "приёмочная проверка вехи A: обращается к настоящему серверу, запускать вручную"]
async fn живой_сервер_только_чтение() {
    let Some(path) = env_import::default_location() else {
        panic!(
            "рядом не нашлось server.env — проверка рассчитана на машину автора, \
             где он есть; на чужой её запускать нечего"
        );
    };
    let imported = env_import::read_from(&path).expect("server.env не разобрался");

    // Журнал включаем нарочно: он служит второй половиной проверки (T064, SC-011).
    // Прогон идёт с настоящим ключом от настоящего сервера, и если вырезание
    // секретов где-то не сработает, след останется именно здесь. Уровень задаётся
    // через VRCAST_LOG — для поиска утечек его ставят в trace, чтобы разговорчивые
    // библиотеки выложили всё, что знают.
    vrcast_studio_lib::logging::init();

    println!("\n=== Сценарий 1, только чтение ===");
    println!("настройки взяты из {}", imported.source.display());
    println!(
        "сервер {}@{}:{}, домен {}",
        imported.input.user, imported.input.host, imported.input.port, imported.input.domain
    );

    let state = state();

    // Шаг 2 сценария: заведомо неверные данные не должны никуда пустить.
    // Проверяем безопасным способом: несуществующий порт того же адреса.
    {
        let mut wrong = imported.input.clone();
        wrong.name = String::from("Заведомо неверный");
        wrong.port = 64_999;
        let id = servers::server_add(&state, wrong, "заведомо-неверный-секрет")
            .expect("профиль не создан");
        let steps = servers::server_test(&state, &id)
            .await
            .expect("проверка обязана вернуть шаги, а не ошибку");
        assert_eq!(
            steps[0].status,
            StepStatus::Failed,
            "закрытый порт вдруг открыт"
        );
        assert!(
            steps[1..].iter().all(|s| s.status == StepStatus::Skipped),
            "после провала сети проверка пошла дальше"
        );
        println!("шаг 2: неверные данные останавливают проверку на первом же шаге — верно");
        servers::server_remove(&state, &id).expect("временный профиль не удалился");
    }

    // Шаг 3: верные данные.
    let secret = String::new(); // ключ автора без парольной фразы
    let id =
        servers::server_add(&state, imported.input.clone(), &secret).expect("профиль не создан");

    let fingerprint = vrcast_studio_lib::commands::api::server_probe_fingerprint(
        &imported.input.host,
        imported.input.port,
    )
    .await
    .expect("отпечаток не получен");
    println!("отпечаток сервера: {fingerprint}");
    servers::server_fingerprint_confirm(&state, &id, &fingerprint)
        .expect("отпечаток не подтвердился");

    let steps = servers::server_test(&state, &id)
        .await
        .expect("проверка вернула ошибку вместо шагов");
    println!("\n--- шаги проверки подключения ---");
    for s in &steps {
        println!(
            "  [{}] {} — {}",
            match s.status {
                StepStatus::Ok => "готово",
                StepStatus::Failed => "СБОЙ  ",
                StepStatus::Skipped => "мимо  ",
            },
            s.id,
            s.detail.as_ref().map(|d| d.key.as_str()).unwrap_or("")
        );
    }
    assert!(
        steps.iter().all(|s| s.status == StepStatus::Ok),
        "не все шаги проверки прошли"
    );

    // Шаги 4 и 4a: библиотека и параметры файлов.
    let view = library::library_list(&state, &id, true)
        .await
        .expect("библиотека не прочиталась");

    println!("\n--- библиотека ---");
    println!(
        "медиа: {}, не распознано: {}, учтено записей: {}",
        view.media.len(),
        view.unrecognized.len(),
        view.accounted_entries()
    );
    if let Some(d) = view.disk {
        println!(
            "диск: свободно {} из {}, видео занимают {}",
            d.free_bytes, d.total_bytes, d.used_by_videos_bytes
        );
        assert!(d.total_bytes > 0 && d.free_bytes <= d.total_bytes);
    }
    assert!(
        !view.stale,
        "данные пришли из кеша: до сервера не достучались"
    );

    println!("\n--- файлы и их параметры (из заголовка, без скачивания) ---");
    let все: Vec<_> = view
        .media
        .iter()
        .flat_map(|m| m.files.iter())
        .chain(view.unrecognized.iter())
        .collect();
    for f in &все {
        println!(
            "  {:<58} {:>10} Б  {}  {}  {}  {}",
            f.path,
            f.size_bytes,
            match (f.width, f.height) {
                (Some(w), Some(h)) => format!("{w}x{h}"),
                _ => String::from("размер неизвестен"),
            },
            f.duration_s
                .map(|d| format!("{:.0} с", d))
                .unwrap_or_else(|| String::from("длит. неизв.")),
            f.video_codec.as_deref().unwrap_or("кодек неизв."),
            match f.faststart_ok {
                Some(true) => "готов к раздаче",
                Some(false) => "ЗАГОЛОВОК В КОНЦЕ",
                None => "заголовок не прочитан",
            }
        );
    }

    // Шаг 5: ссылки. Проверяем, что они собраны из домена профиля и указывают
    // на существующие файлы.
    println!("\n--- зрительские ссылки ---");
    for f in все.iter().take(3) {
        println!("  {}", f.origin_url);
        assert!(
            f.origin_url
                .starts_with(&format!("https://{}/", imported.input.domain)),
            "ссылка собрана не из домена профиля: {}",
            f.origin_url
        );
    }

    assert!(
        !все.is_empty(),
        "на сервере не нашлось ни одного файла — проверять нечего"
    );
    println!("\n=== чтение сервера завершено, ничего не изменено ===\n");
}
