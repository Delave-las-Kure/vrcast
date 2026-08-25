//! T015 — договорные тесты слоя команд.
//!
//! Проверяется форма ответа и коды ошибок, а не поведение внутри: за поведение отвечают
//! тесты соответствующих слоёв. Смысл в том, чтобы договор нельзя было изменить незаметно —
//! договор читает интерфейс, и его расхождение с ядром обнаружилось бы у пользователя.
//!
//! Команды вызываются напрямую как обычные функции: тонкие обёртки для оболочки логики
//! не содержат, а требовать для тестов живого окна с графикой в непрерывной интеграции
//! нельзя.

use std::sync::Arc;
use std::time::Duration;
use vrcast_studio_lib::commands::error::{AppError, ErrorCode};
use vrcast_studio_lib::commands::{api, AppState};
use vrcast_studio_lib::store::db::Db;
use vrcast_studio_lib::store::secrets::InMemorySecretStore;
use vrcast_studio_lib::tasks::state::{TaskKind, TaskState};

fn state() -> AppState {
    AppState::with_db(
        Arc::new(Db::open_in_memory().unwrap()),
        Arc::new(InMemorySecretStore::new()),
    )
    .expect("состояние приложения не собралось")
}

// ---------- полнота договора ----------

#[test]
fn у_каждого_кода_ошибки_есть_сообщение_и_подсказка() {
    // Конституция, раздел «Ограничения качества исполнения»: сообщение без подсказки
    // оставляет человека наедине с проблемой, о которой он ничего не знает.
    // Проверяется здесь, а не оставлено на внимательность при добавлении нового кода.
    for code in ErrorCode::ALL {
        let msg = code.message();
        let hint = code.hint();

        assert!(!msg.trim().is_empty(), "у кода {code} нет сообщения");
        assert!(!hint.trim().is_empty(), "у кода {code} нет подсказки");

        assert!(
            msg.chars().any(|c| ('а'..='я').contains(&c)),
            "сообщение кода {code} не на русском: {msg}"
        );
        assert!(
            hint.chars().any(|c| ('а'..='я').contains(&c)),
            "подсказка кода {code} не на русском: {hint}"
        );

        // Технический код в тексте для человека — признак того, что формулировку
        // не написали, а подставили.
        assert!(
            !msg.contains(code.as_str()),
            "в сообщении кода {code} стоит сам код"
        );
        assert!(
            hint.len() > msg.len() / 2,
            "подсказка кода {code} подозрительно коротка: «{hint}»"
        );
    }
}

#[test]
fn коды_ошибок_не_повторяются() {
    let mut seen = std::collections::HashSet::new();
    for code in ErrorCode::ALL {
        assert!(
            seen.insert(code.as_str()),
            "код {} встречается дважды",
            code.as_str()
        );
    }
    assert_eq!(seen.len(), ErrorCode::ALL.len());
}

#[test]
fn ошибка_сериализуется_по_форме_договора() {
    // Форма { code, message, hint?, cause? } — правило 2 договора.
    let err = AppError::new(ErrorCode::DomainNotPointed).with_cause("stream-test.example.ru");
    let json = serde_json::to_value(&err).unwrap();

    assert_eq!(json["code"], "DOMAIN_NOT_POINTED");
    assert!(json["message"].is_string());
    assert!(json["hint"].is_string());
    assert_eq!(json["cause"], "stream-test.example.ru");

    // Без уточнения поле не появляется вовсе, а не приходит пустым.
    let bare = serde_json::to_value(AppError::new(ErrorCode::Internal)).unwrap();
    assert!(
        bare.get("cause").is_none(),
        "пустое уточнение попало в ответ"
    );
}

#[test]
fn уточнение_ошибки_проходит_вырезание_секретов() {
    // Уточнение нередко приходит от чужой библиотеки, которая о наших правилах не знает
    // (конституция, принцип IV).
    vrcast_studio_lib::store::redact::forget_all();
    let secret = "пароль-от-чужого-сервера-77";
    vrcast_studio_lib::store::redact::register(secret);

    let err = AppError::new(ErrorCode::SshAuthFailed)
        .with_cause(format!("вход не удался, использован {secret}"));
    let json = serde_json::to_string(&err).unwrap();

    assert!(!json.contains(secret), "СЕКРЕТ В ОТВЕТЕ КОМАНДЫ: {json}");
}

// ---------- команды ----------

#[test]
fn app_versions_возвращает_версии() {
    let s = state();
    let v = api::app_versions(&s).unwrap();

    assert!(!v.app.is_empty(), "версия приложения пуста");
    assert!(v.schema >= 1, "версия схемы не заполнена");
    // Версия серверной части появится в Фазе 7 — пока её нет, и это честно.
    assert!(v.server.is_none());
}

#[test]
fn список_задач_у_нового_приложения_пуст() {
    let s = state();
    assert!(api::tasks_list(&s).unwrap().is_empty());
}

#[test]
fn обращение_к_несуществующей_задаче_даёт_код_договора() {
    let s = state();
    let err = api::task_get(&s, "нет-такой").expect_err("несуществующая задача найдена");
    assert_eq!(err.code, ErrorCode::TaskNotFound);
    assert!(!err.hint.is_empty());
}

#[test]
fn отмена_несуществующей_задачи_даёт_код_договора() {
    let s = state();
    let err = api::task_cancel(&s, "нет-такой").expect_err("отменилась несуществующая задача");
    assert_eq!(err.code, ErrorCode::TaskNotFound);
}

#[tokio::test]
async fn приостановка_короткой_задачи_даёт_свой_код() {
    let s = state();
    // Работа долгая и отменяемая, а не «поспать 600 мс»: на загруженной машине
    // короткая задача успевала бы завершиться до task_pause, и вместо проверяемого
    // кода приходил бы TASK_NOT_FOUND — ложное падение.
    let id = s
        .tasks
        .submit(TaskKind::Probe, None, |ctx| async move {
            for _ in 0..600 {
                if ctx.is_cancelled() {
                    return Ok(());
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            Ok(())
        })
        .await
        .unwrap();

    // Ждём, пока задача действительно пойдёт.
    for _ in 0..50 {
        if matches!(
            s.tasks.get(&id).unwrap().map(|t| t.state),
            Some(TaskState::Running)
        ) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }

    let err = api::task_pause(&s, &id).expect_err("короткая задача приостановилась");
    assert_eq!(err.code, ErrorCode::TaskNotPausable);

    api::task_cancel(&s, &id).unwrap();
}

#[tokio::test]
async fn при_закрытии_каждая_задача_объясняется_отдельно() {
    // FR-086. Общего «идут задачи, закрыть?» недостаточно: оно не даёт принять решение.
    let s = state();

    let upload = s
        .tasks
        .submit(TaskKind::Upload, None, |ctx| async move {
            for _ in 0..100 {
                ctx.wait_while_paused().await;
                if ctx.is_cancelled() {
                    return Ok(());
                }
                tokio::time::sleep(Duration::from_millis(30)).await;
            }
            Ok(())
        })
        .await
        .unwrap();

    let convert = s
        .tasks
        .submit(TaskKind::Convert, None, |ctx| async move {
            for _ in 0..100 {
                if ctx.is_cancelled() {
                    return Ok(());
                }
                tokio::time::sleep(Duration::from_millis(30)).await;
            }
            Ok(())
        })
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_millis(300)).await;

    let report = api::tasks_on_close(&s).unwrap();
    assert_eq!(
        report.len(),
        2,
        "не все выполняющиеся задачи попали в отчёт"
    );

    let u = report
        .iter()
        .find(|t| t.id == upload)
        .expect("нет передачи");
    let c = report
        .iter()
        .find(|t| t.id == convert)
        .expect("нет подготовки");

    // Разница между видами задач и есть суть требования.
    assert_eq!(u.outcome, "resumes", "передача должна продолжаться с места");
    assert_eq!(c.outcome, "restarts", "подготовка не переживёт закрытия");
    assert!(
        c.explanation.contains("заново"),
        "объяснение не называет последствие: {}",
        c.explanation
    );
    assert!(u.explanation.contains("продолжится"), "{}", u.explanation);

    api::task_cancel(&s, &upload).unwrap();
    api::task_cancel(&s, &convert).unwrap();
}

#[test]
fn завершённые_задачи_в_отчёт_о_закрытии_не_попадают() {
    use vrcast_studio_lib::tasks::store;
    let s = state();

    let mut done = store::TaskRecord::new("t-готова", TaskKind::Upload, None);
    done.state = TaskState::Completed;
    store::upsert(&s.db, &done).unwrap();

    assert!(
        api::tasks_on_close(&s).unwrap().is_empty(),
        "завершённая задача попала в предупреждение о закрытии"
    );
}
