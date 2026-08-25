//! Тесты слоя доступа к серверу, не требующие сервера (T022–T025).
//!
//! Всё, что требует настоящего OpenSSH, проверяется отдельным запуском
//! `cargo run --example ssh_smoke` — тесты не должны зависеть от чужого сервера и сети.
//! Здесь остаётся то, что от сети не зависит и потому обязано быть тестом: разбор ключей,
//! неразглашение секретов при отладочном выводе и хранение отпечатков.

use std::path::{Path, PathBuf};
use vrcast_studio_lib::ssh::{auth, fingerprint, Credentials, ServerAddress, SshError};
use vrcast_studio_lib::store::db::Db;

/// Ключ создаётся на месте и в репозиторий не попадает — см. `tests/support/test_key.rs`.
/// Раньше эти тесты брали готовый файл, и на машине разработчика он был от прошлых
/// прогонов; в непрерывной интеграции его не оказалось, и тесты упали.
fn fixture(_name: &str) -> PathBuf {
    super::test_key::ensure().expect("ключ для тестов не создался")
}

const FIXTURE_PASSPHRASE: &str = super::test_key::PASSPHRASE;

#[test]
fn ключ_создаётся_даже_когда_за_ним_приходят_разом() {
    // Тесты идут в несколько потоков и просят ключ одновременно. Без замка два
    // из них решают, что ключа нет, оба стирают файлы и оба зовут ssh-keygen;
    // второй натыкается на уже созданный файл и выходит с ошибкой, не сказав
    // ни слова. Прогон на Windows поймал это 2026-08-25 — но лишь по везению,
    // и потому гонка проверяется отдельно, а не оставляется на удачу.
    let потоки: Vec<_> = (0..8)
        .map(|_| std::thread::spawn(super::test_key::ensure))
        .collect();

    for (i, п) in потоки.into_iter().enumerate() {
        let итог = п.join().expect("поток упал");
        let path = итог.unwrap_or_else(|e| panic!("поток {i}: {e}"));
        assert!(path.exists(), "поток {i}: ключа нет по обещанному пути");
    }
}

#[test]
fn ключ_с_парольной_фразой_читается_когда_фраза_дана() {
    // FR-096: у пользователей встречаются ключи, защищённые фразой, и это должно работать,
    // а не оборачиваться невнятной ошибкой чтения файла.
    let key = auth::load_key(&fixture("encrypted_ed25519.key"), Some(FIXTURE_PASSPHRASE))
        .expect("защищённый ключ не прочитался при верной парольной фразе");
    assert_eq!(key.algorithm().as_str(), "ssh-ed25519");
}

#[test]
fn ключ_без_фразы_даёт_отдельную_понятную_ошибку() {
    // Именно отдельную: «нужна парольная фраза» и «файл не читается» требуют от пользователя
    // разных действий, и сливать их в одну ошибку значит заставить его гадать (FR-105).
    match auth::load_key(&fixture("encrypted_ed25519.key"), None) {
        Err(SshError::KeyNeedsPassphrase { path }) => {
            assert!(path.contains("encrypted_ed25519"), "путь потерян: {path}");
        }
        Err(other) => panic!("ожидалась KeyNeedsPassphrase, получено: {other}"),
        Ok(_) => panic!("защищённый ключ прочитался без парольной фразы"),
    }
}

#[test]
fn неверная_фраза_не_выдаётся_за_нечитаемый_файл() {
    let err = auth::load_key(&fixture("encrypted_ed25519.key"), Some("не-та-фраза"))
        .expect_err("ключ прочитался с неверной фразой");
    // Какой бы ни была формулировка, в ней не должно оказаться самой фразы.
    let text = err.to_string();
    assert!(
        !text.contains("не-та-фраза"),
        "фраза попала в текст ошибки: {text}"
    );
}

#[test]
fn отсутствующий_файл_ключа_даёт_ошибку_чтения() {
    match auth::load_key(Path::new("нет-такого-файла.key"), None) {
        Err(SshError::KeyUnreadable { path, .. }) => assert!(path.contains("нет-такого-файла")),
        Err(other) => panic!("ожидалась KeyUnreadable, получено: {other}"),
        Ok(_) => panic!("прочитался несуществующий ключ"),
    }
}

#[test]
fn отладочный_вывод_учётных_данных_не_раскрывает_секрет() {
    // Самый частый способ утечки — не «напечатали пароль», а «структура попала в вывод
    // целиком». Поэтому Debug у Credentials написан вручную (конституция, принцип IV).
    let pass = Credentials::Password(String::from("очень-секретный-пароль-1"));
    let shown = format!("{pass:?}");
    assert!(
        !shown.contains("очень-секретный-пароль-1"),
        "пароль виден: {shown}"
    );

    let key = Credentials::Key {
        path: PathBuf::from("/home/u/.ssh/id_ed25519"),
        passphrase: Some(String::from("секретная-фраза-2")),
    };
    let shown = format!("{key:?}");
    assert!(!shown.contains("секретная-фраза-2"), "фраза видна: {shown}");
    // Путь при этом должен остаться: он не секрет и нужен для разбора неполадок.
    assert!(
        shown.contains("id_ed25519"),
        "путь к ключу потерян: {shown}"
    );
    assert!(
        shown.contains("задана"),
        "не видно, что фраза вообще задана: {shown}"
    );
}

#[test]
fn отпечаток_запоминается_и_читается() {
    let db = Db::open_in_memory().unwrap();
    let addr = ServerAddress::new("example.test", 22);

    assert_eq!(
        fingerprint::stored(&db, &addr).unwrap(),
        None,
        "отпечаток взялся из ниоткуда"
    );

    fingerprint::remember(&db, &addr, "SHA256:перваяверсия").unwrap();
    assert_eq!(
        fingerprint::stored(&db, &addr).unwrap().as_deref(),
        Some("SHA256:перваяверсия")
    );

    // Повторная запись того же — не ошибка: повтор обязан быть безопасным (принцип V).
    fingerprint::remember(&db, &addr, "SHA256:перваяверсия").unwrap();
    assert_eq!(
        fingerprint::stored(&db, &addr).unwrap().as_deref(),
        Some("SHA256:перваяверсия")
    );

    // Осознанная замена — например, сервер пересоздан и пользователь это подтвердил.
    fingerprint::remember(&db, &addr, "SHA256:втораяверсия").unwrap();
    assert_eq!(
        fingerprint::stored(&db, &addr).unwrap().as_deref(),
        Some("SHA256:втораяверсия")
    );

    fingerprint::forget(&db, &addr).unwrap();
    assert_eq!(fingerprint::stored(&db, &addr).unwrap(), None);
}

#[test]
fn отпечаток_привязан_к_паре_адрес_и_порт() {
    // Один и тот же хост на разных портах — разные серверы. Смешивать их отпечатки
    // значит либо ложно тревожить, либо пропустить подмену.
    let db = Db::open_in_memory().unwrap();
    let a22 = ServerAddress::new("example.test", 22);
    let a2222 = ServerAddress::new("example.test", 2222);

    fingerprint::remember(&db, &a22, "SHA256:ключ-двадцать-второго").unwrap();
    assert_eq!(fingerprint::stored(&db, &a2222).unwrap(), None);

    fingerprint::remember(&db, &a2222, "SHA256:ключ-другого-порта").unwrap();
    assert_eq!(
        fingerprint::stored(&db, &a22).unwrap().as_deref(),
        Some("SHA256:ключ-двадцать-второго"),
        "запись по другому порту перезаписала чужой отпечаток"
    );
}

// ---------- отчего не удалась файловая операция (T071) ----------

#[test]
fn full_disk_is_not_reported_as_a_permission_problem() {
    // Прежде любая файловая беда объявлялась нехваткой прав с подсказкой
    // «проверьте владельца каталога». При полном диске человек шёл чинить то,
    // что не сломано, а настоящая причина лежала на виду в тексте ошибки.
    use vrcast_studio_lib::commands::error::ErrorCode;
    use vrcast_studio_lib::ssh::SshError;

    let err = SshError::sftp("write failed: No space left on device");
    let app: vrcast_studio_lib::commands::error::AppError = err.into();
    assert_eq!(app.code, ErrorCode::RemoteDiskFull);
    assert!(
        app.hint.to_lowercase().contains("мест"),
        "подсказка не про место: {}",
        app.hint
    );
}

#[test]
fn each_file_failure_leads_to_its_own_answer() {
    use vrcast_studio_lib::commands::error::{AppError, ErrorCode};
    use vrcast_studio_lib::ssh::SshError;

    let cases = [
        ("Permission denied", ErrorCode::VideoDirDenied),
        ("No such file or directory", ErrorCode::FileMissingOnServer),
        ("connection reset by peer", ErrorCode::SshUnreachable),
        ("disk quota exceeded", ErrorCode::RemoteDiskFull),
    ];
    for (text, expected) in cases {
        let app: AppError = SshError::sftp(text).into();
        assert_eq!(app.code, expected, "неверно опознано: {text}");
    }
}

#[test]
fn an_unfamiliar_failure_is_not_guessed_at() {
    // Неверная догадка хуже честного «не знаю»: она уводит чинить не то.
    // Текст при этом обязан сохраниться — его можно найти поиском.
    use vrcast_studio_lib::commands::error::{AppError, ErrorCode};
    use vrcast_studio_lib::ssh::SshError;

    let app: AppError = SshError::sftp("SFTP status 4: something nobody has seen").into();
    assert_eq!(app.code, ErrorCode::Internal);
    assert!(
        app.cause
            .unwrap_or_default()
            .contains("something nobody has seen"),
        "потерян текст, по которому только и можно разобраться"
    );
}
