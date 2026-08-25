//! T035 — договорные тесты команд управления серверами.
//!
//! Проверяется форма ответа и коды ошибок (`contracts/ipc-commands.md`, «Серверы»).
//! Настоящий сервер здесь не нужен и не используется: профили живут в локальной базе,
//! а единственная команда, которой нужна сеть, проверяется на заведомо закрытом порте —
//! ровно затем, чтобы убедиться, что неудача выглядит как данные, а не как отказ.

use super::support::{state, valid_input};
use vrcast_studio_lib::commands::error::ErrorCode;
use vrcast_studio_lib::commands::servers::{api, StepStatus, TEST_STEPS};
use vrcast_studio_lib::domain::server_profile::AuthKind;
use vrcast_studio_lib::store::secrets::SecretRef;

const SECRET: &str = "пароль-от-сервера-для-теста-9f3a";

#[test]
fn пустой_список_профилей_это_пустой_список_а_не_ошибка() {
    let s = state();
    let list = api::servers_list(&s).expect("список профилей не отдан");
    assert!(list.is_empty());
}

#[test]
fn добавленный_профиль_виден_в_списке_а_секрет_ушёл_в_хранилище() {
    let s = state();
    let id = api::server_add(&s, valid_input("Мой сервер"), SECRET).expect("профиль не добавлен");

    let list = api::servers_list(&s).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].id, id);
    assert_eq!(list[0].name, "Мой сервер");

    // В профиле — только ссылка на секрет. Сам секрет обязан лежать в хранилище ОС.
    assert!(
        !list[0].secret_ref.is_empty(),
        "профиль не ссылается на секрет"
    );
    let stored = s
        .secrets
        .get(&SecretRef::from_stored(&list[0].secret_ref))
        .expect("секрет не найден в хранилище");
    assert_eq!(stored, SECRET);
}

#[test]
fn профиль_с_негодными_полями_не_создаётся() {
    let s = state();
    let mut input = valid_input("Без домена");
    input.domain = String::new();

    let err = api::server_add(&s, input, SECRET).expect_err("профиль без домена создан");
    assert_eq!(err.code, ErrorCode::InvalidInput);
    assert!(
        err.message.contains("омен"),
        "сообщение не называет поле: {}",
        err.message
    );
    assert!(!err.hint.trim().is_empty(), "нет подсказки, что делать");

    assert!(
        api::servers_list(&s).unwrap().is_empty(),
        "негодный профиль всё-таки сохранился"
    );
}

#[test]
fn секрет_негодного_профиля_не_остаётся_в_хранилище() {
    // Иначе после неудачной попытки в системном менеджере паролей копились бы записи,
    // на которые ничто не ссылается, — и удалить их пользователю было бы нечем.
    let s = state();
    let mut input = valid_input("Без домена");
    input.domain = String::new();
    let _ = api::server_add(&s, input, SECRET);

    let leftovers = s.secrets.get(&SecretRef::for_server("")).is_ok();
    assert!(!leftovers, "остался секрет от несозданного профиля");
}

#[test]
fn два_профиля_с_одинаковым_именем_не_заводятся() {
    // Имя — единственное, по чему пользователь отличает серверы в списке.
    let s = state();
    api::server_add(&s, valid_input("Один"), SECRET).unwrap();

    let err = api::server_add(&s, valid_input("Один"), SECRET)
        .expect_err("заведён второй профиль с тем же именем");
    assert_eq!(err.code, ErrorCode::InvalidInput);
    assert_eq!(api::servers_list(&s).unwrap().len(), 1);
}

#[test]
fn изменение_профиля_без_нового_секрета_не_трогает_прежний() {
    // Иначе правка домена стирала бы пароль, и пользователь узнавал бы об этом
    // при следующем подключении.
    let s = state();
    let id = api::server_add(&s, valid_input("Сервер"), SECRET).unwrap();
    let reference = SecretRef::from_stored(&api::servers_list(&s).unwrap()[0].secret_ref);

    let mut input = valid_input("Сервер");
    input.domain = String::from("new.example.com");
    api::server_update(&s, &id, input, None).expect("профиль не изменён");

    assert_eq!(
        s.secrets.get(&reference).expect("секрет исчез"),
        SECRET,
        "секрет заменён, хотя новый не передавали"
    );
    assert_eq!(api::servers_list(&s).unwrap()[0].domain, "new.example.com");
}

#[test]
fn переданный_секрет_заменяет_прежний() {
    let s = state();
    let id = api::server_add(&s, valid_input("Сервер"), SECRET).unwrap();
    let reference = SecretRef::from_stored(&api::servers_list(&s).unwrap()[0].secret_ref);

    let новый = "другой-пароль-совсем-не-похожий";
    api::server_update(&s, &id, valid_input("Сервер"), Some(новый)).unwrap();

    assert_eq!(s.secrets.get(&reference).unwrap(), новый);
}

#[test]
fn удаление_профиля_убирает_и_секрет_из_хранилища() {
    // FR-005: удаляя профиль, приложение забывает и доступ. Оставленный секрет —
    // это доступ к чужому серверу, о котором пользователь уже не помнит.
    let s = state();
    let id = api::server_add(&s, valid_input("Сервер"), SECRET).unwrap();
    let reference = SecretRef::from_stored(&api::servers_list(&s).unwrap()[0].secret_ref);

    api::server_remove(&s, &id).expect("профиль не удалён");

    assert!(api::servers_list(&s).unwrap().is_empty());
    assert!(
        s.secrets.get(&reference).is_err(),
        "секрет удалённого профиля остался в хранилище"
    );
}

#[test]
fn повторное_удаление_безопасно() {
    // Договор, правило 5: повтор той же команды не портит результат.
    let s = state();
    let id = api::server_add(&s, valid_input("Сервер"), SECRET).unwrap();

    api::server_remove(&s, &id).unwrap();
    api::server_remove(&s, &id).expect("повторное удаление считается ошибкой");
}

#[test]
fn активен_ровно_один_профиль() {
    // FR-002. Правило держится базой, а не аккуратностью кода, — но договор обязан
    // это показывать.
    let s = state();
    let first = api::server_add(&s, valid_input("Первый"), SECRET).unwrap();
    let second = api::server_add(&s, valid_input("Второй"), SECRET).unwrap();

    api::server_set_active(&s, &first).unwrap();
    let active: Vec<String> = api::servers_list(&s)
        .unwrap()
        .into_iter()
        .filter(|p| p.is_active)
        .map(|p| p.id)
        .collect();
    assert_eq!(active, vec![first.clone()]);

    api::server_set_active(&s, &second).unwrap();
    let active: Vec<String> = api::servers_list(&s)
        .unwrap()
        .into_iter()
        .filter(|p| p.is_active)
        .map(|p| p.id)
        .collect();
    assert_eq!(active, vec![second], "активными оказались двое сразу");
}

#[test]
fn подтверждённый_отпечаток_запоминается() {
    // FR-092: подтверждение — разовое действие человека, и оно обязано пережить
    // перезапуск, иначе спрашивать будут каждый раз и подтверждать перестанут думая.
    let s = state();
    let id = api::server_add(&s, valid_input("Сервер"), SECRET).unwrap();
    let fp = "SHA256:AbCdEfGhIjKlMnOpQrStUvWxYz0123456789abcdefg";

    api::server_fingerprint_confirm(&s, &id, fp).expect("отпечаток не подтверждён");

    let profile = api::servers_list(&s).unwrap().remove(0);
    assert_eq!(profile.host_fingerprint.as_deref(), Some(fp));
}

#[test]
fn вход_по_ключу_без_пути_к_ключу_отвергается() {
    let s = state();
    let mut input = valid_input("По ключу");
    input.auth_kind = AuthKind::Key;
    input.key_path = None;

    let err = api::server_add(&s, input, SECRET).expect_err("принят вход по ключу без ключа");
    assert_eq!(err.code, ErrorCode::InvalidInput);
}

#[tokio::test]
async fn проверка_подключения_возвращает_все_шаги_с_отметкой_где_остановилось() {
    // FR-003. Это главное свойство команды: пользователь должен видеть, что успело
    // пройти, а не только сообщение о последней беде. Порт заведомо закрыт — первый
    // же шаг обязан провалиться, а остальные прийти помеченными как невыполнявшиеся.
    let s = state();
    let mut input = valid_input("Недоступный");
    input.host = String::from("127.0.0.1");
    input.port = 1;
    let id = api::server_add(&s, input, SECRET).unwrap();

    let steps = api::server_test(&s, &id)
        .await
        .expect("неудача шага не должна быть отказом команды: это данные");

    assert_eq!(
        steps.len(),
        TEST_STEPS.len(),
        "вернулись не все шаги: {steps:?}"
    );
    let ids: Vec<&str> = steps.iter().map(|x| x.id.as_str()).collect();
    let expected: Vec<&str> = TEST_STEPS.iter().map(|(id, _)| *id).collect();
    assert_eq!(ids, expected, "порядок шагов изменён");

    assert_eq!(steps[0].status, StepStatus::Failed, "сеть вдруг доступна");
    assert!(
        steps[0].detail.is_some(),
        "неудача без объяснения бесполезна"
    );
    for step in &steps[1..] {
        assert_eq!(
            step.status,
            StepStatus::Skipped,
            "шаг {} выполнялся после провала предыдущего",
            step.id
        );
    }
    for step in &steps {
        assert!(
            !step.title.trim().is_empty(),
            "у шага {} нет названия",
            step.id
        );
    }
}

#[tokio::test]
async fn проверка_несуществующего_профиля_это_ошибка_а_не_пустой_список() {
    let s = state();
    let err = api::server_test(&s, "нет-такого")
        .await
        .expect_err("проверен несуществующий профиль");
    assert_eq!(err.code, ErrorCode::InvalidInput);
}
