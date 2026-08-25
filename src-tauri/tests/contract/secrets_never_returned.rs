//! T037 — ни одна команда не возвращает секрет наружу.
//!
//! Конституция, принцип IV; договор слоя команд, правило 3: секрет пересекает границу
//! **ровно один раз** — когда интерфейс передаёт его при создании или изменении профиля.
//! Обратно он не возвращается никогда.
//!
//! Проверка устроена как поиск, а не как разбор полей: ответ команды переводится
//! в то, что действительно уходит интерфейсу — в JSON, — и в нём ищется сам секрет.
//! Разбор полей проверял бы только те места, о которых автор теста подумал; поиск
//! ловит и то, о чём никто не подумал, — например, секрет, попавший в текст ошибки
//! от чужой библиотеки.

use super::support::{state, valid_input};
use vrcast_studio_lib::commands::servers::api as servers_api;
use vrcast_studio_lib::commands::{api, AppState};
use vrcast_studio_lib::domain::server_profile::AuthKind;

/// Секреты нарочно длинные и ни на что не похожие: короткое значение могло бы
/// случайно встретиться в ответе и дать ложное спокойствие в обратную сторону.
const PASSWORD: &str = "пароль-который-не-должен-выйти-наружу-a1b2c3";
const PASSPHRASE: &str = "парольная-фраза-ключа-которая-тоже-не-должна-выйти-d4e5f6";

/// Найти секрет в том виде, в каком ответ уходит интерфейсу.
///
/// Проверяются оба представления: как есть и как записывает JSON (кириллица уезжает
/// в escape-последовательности у части сериализаторов, и поиск по исходной строке
/// такую утечку пропустил бы).
fn contains_secret(json: &str, secret: &str) -> bool {
    if json.contains(secret) {
        return true;
    }
    let escaped: String = secret
        .chars()
        .map(|c| {
            if c.is_ascii() {
                c.to_string()
            } else {
                format!("\\u{:04x}", c as u32)
            }
        })
        .collect();
    json.contains(&escaped)
}

fn assert_clean<T: serde::Serialize>(what: &str, value: &T) {
    let json = serde_json::to_string(value).expect("ответ команды не сериализуется");
    for secret in [PASSWORD, PASSPHRASE] {
        assert!(
            !contains_secret(&json, secret),
            "СЕКРЕТ В ОТВЕТЕ команды {what}: {json}"
        );
    }
}

fn assert_error_clean(what: &str, err: &vrcast_studio_lib::commands::error::AppError) {
    let json = serde_json::to_string(err).expect("ошибка не сериализуется");
    for secret in [PASSWORD, PASSPHRASE] {
        assert!(
            !contains_secret(&json, secret),
            "СЕКРЕТ В ОШИБКЕ команды {what}: {json}"
        );
    }
}

fn state_with_two_profiles() -> (AppState, String, String) {
    let s = state();

    let by_password = servers_api::server_add(&s, valid_input("По паролю"), PASSWORD)
        .expect("профиль по паролю не создан");

    let mut key_input = valid_input("По ключу");
    key_input.auth_kind = AuthKind::Key;
    key_input.key_path = Some(String::from("/home/user/.ssh/id_ed25519"));
    key_input.host = String::from("127.0.0.1");
    key_input.port = 1;
    let by_key =
        servers_api::server_add(&s, key_input, PASSPHRASE).expect("профиль по ключу не создан");

    (s, by_password, by_key)
}

#[test]
fn список_профилей_не_содержит_секретов() {
    let (s, _, _) = state_with_two_profiles();
    let list = servers_api::servers_list(&s).expect("список не отдан");

    assert_eq!(list.len(), 2, "тест построен неверно: профилей не два");
    assert_clean("servers_list", &list);
}

#[test]
fn прочие_читающие_команды_не_содержат_секретов() {
    let (s, _, _) = state_with_two_profiles();

    assert_clean("app_versions", &api::app_versions(&s).unwrap());
    assert_clean("tasks_list", &api::tasks_list(&s).unwrap());
    assert_clean("tasks_on_close", &api::tasks_on_close(&s).unwrap());
}

#[test]
fn ссылки_не_содержат_секретов() {
    let (s, by_password, _) = state_with_two_profiles();
    let links = vrcast_studio_lib::commands::library::api::links_for(&s, &by_password, "a.mp4")
        .expect("ссылки не построены");
    assert_clean("links_for", &links);
}

#[tokio::test]
async fn неудачная_проверка_подключения_не_выносит_секрет_в_подробностях() {
    // Самый вероятный путь утечки: подробность приходит от чужой библиотеки, которая
    // о наших правилах ничего не знает, и оседает в тексте ошибки. Порт закрыт —
    // значит, шаги провалятся и подробности будут настоящие.
    let (s, _, by_key) = state_with_two_profiles();

    match servers_api::server_test(&s, &by_key).await {
        Ok(steps) => {
            assert!(
                steps.iter().any(|x| x.detail.is_some()),
                "тест построен неверно: ни одной подробности, искать нечего"
            );
            assert_clean("server_test", &steps);
        }
        Err(e) => assert_error_clean("server_test", &e),
    }
}

#[tokio::test]
async fn ошибки_команд_библиотеки_не_выносят_секрет() {
    let (s, by_password, _) = state_with_two_profiles();

    if let Err(e) =
        vrcast_studio_lib::commands::library::api::library_list(&s, &by_password, true).await
    {
        assert_error_clean("library_list", &e);
    }
}

#[test]
fn изменение_профиля_не_возвращает_секрет_обратно() {
    // Обратная сторона правила: интерфейс передаёт секрет, но никогда не получает
    // его назад — даже той же командой, которой только что передал.
    let (s, by_password, _) = state_with_two_profiles();

    let result = servers_api::server_update(&s, &by_password, valid_input("По паролю"), None);
    match result {
        Ok(nothing) => assert_clean("server_update", &nothing),
        Err(e) => assert_error_clean("server_update", &e),
    }

    assert_clean(
        "servers_list после изменения",
        &servers_api::servers_list(&s).unwrap(),
    );
}

#[test]
fn проверка_поиска_умеет_находить_секрет() {
    // Тест на сам тест. Поиск, который ничего не находит по устройству, дал бы
    // спокойствие вместо проверки — а это хуже отсутствия проверки.
    let подделка = serde_json::json!({ "поле": PASSWORD });
    let json = serde_json::to_string(&подделка).unwrap();
    assert!(
        contains_secret(&json, PASSWORD),
        "поиск не находит секрет, положенный прямо в ответ: {json}"
    );
}
