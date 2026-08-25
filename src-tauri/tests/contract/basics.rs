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
use vrcast_studio_lib::commands::error::{AppError, DetailCode, ErrorCode};
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

/// Прочитать словарь интерфейса.
fn catalogue(lang: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("нет родительского каталога у src-tauri")
        .join(format!("src/shared/i18n/{lang}.ts"));
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("не прочитать {}: {e}", path.display()))
}

/// Есть ли в словаре запись с таким ключом.
///
/// Ключ ищется в начале строки, а не где попало: код, случайно упомянутый в
/// комментарии, засчитываться не должен — иначе проверка пройдёт на словаре,
/// в котором формулировки нет.
fn has_entry(catalogue: &str, key: &str) -> bool {
    catalogue
        .lines()
        .any(|line| line.trim_start().starts_with(&format!("{key}:")))
}

#[test]
fn у_каждого_кода_есть_формулировка_на_обоих_языках() {
    // Прежде эта проверка требовала русского сообщения и подсказки прямо в ядре.
    // Ядро больше не сочиняет фраз: оно называет случай кодом, а формулировки живут
    // в словарях интерфейса — по одному на язык (FR-105, FR-106).
    //
    // Требование от этого не ослабло, а усилилось: формулировка обязана быть в
    // КАЖДОМ языке. Пропуск в одном из словарей означает пустое место на экране
    // вместо объяснения — и увидел бы его пользователь, а не мы.
    //
    // Полноту словарей проверяет и компилятор TypeScript (они объявлены как
    // `Record<ErrorCode, …>`), но эта проверка не полагается на то, что сборку
    // интерфейса кто-то запустил: она читает сами файлы.
    for lang in ["ru", "en"] {
        let text = catalogue(lang);
        for code in ErrorCode::ALL {
            assert!(
                has_entry(&text, code.as_str()),
                "в словаре {lang} нет формулировки для кода {code}"
            );
        }
        for detail in DetailCode::ALL {
            assert!(
                has_entry(&text, detail.as_str()),
                "в словаре {lang} нет формулировки для уточнения {detail}"
            );
        }
    }
}

#[test]
fn коды_не_попадают_в_текст_для_человека() {
    // Технический код в тексте — признак того, что формулировку не написали,
    // а подставили заглушку.
    let ru = catalogue("ru");
    for code in ErrorCode::ALL {
        let key = format!("{}:", code.as_str());
        let line = ru
            .lines()
            .find(|l| l.trim_start().starts_with(&key))
            .expect("запись словаря только что была найдена");
        let after = line.split_once(':').map(|(_, r)| r).unwrap_or("");
        assert!(
            !after.contains(code.as_str()),
            "в формулировке кода {code} стоит сам код: {line}"
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
    // Форма { code, details?, cause? } — правило 2 договора. Готовых фраз в ней нет:
    // ядро называет случай, формулировку берёт интерфейс из словаря выбранного языка.
    use vrcast_studio_lib::domain::wording::Detail;

    let err = AppError::new(ErrorCode::RemoteDiskFull)
        .with_detail(Detail::new(DetailCode::NotEnoughSpace).with("short_by", 1024_u64))
        .with_cause("stream-test.example.ru");
    let json = serde_json::to_value(&err).unwrap();

    assert_eq!(json["code"], "REMOTE_DISK_FULL");
    assert_eq!(json["details"][0]["key"], "NOT_ENOUGH_SPACE");
    // Число уходит числом: единицы и разделитель дробной части у языков разные,
    // и выбирать их — дело интерфейса, а не ядра.
    assert_eq!(json["details"][0]["params"]["short_by"], 1024);
    assert_eq!(json["cause"], "stream-test.example.ru");

    assert!(
        json.get("message").is_none() && json.get("hint").is_none(),
        "ядро снова сочиняет фразы: {json}"
    );

    // Пустые поля не появляются вовсе, а не приходят пустыми.
    let bare = serde_json::to_value(AppError::new(ErrorCode::Internal)).unwrap();
    assert!(
        bare.get("cause").is_none(),
        "пустое уточнение попало в ответ"
    );
    assert!(
        bare.get("details").is_none(),
        "пустой перечень уточнений попал в ответ"
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

#[test]
fn подстановка_в_уточнении_проходит_вырезание_секретов() {
    // Новый путь наружу, появившийся вместе с двумя языками: раньше подробность была
    // одной строкой и вырезание стояло на ней, а теперь рядом идут подстановки —
    // имя файла, путь, имя профиля. Любая из них может прийти оттуда же, откуда
    // приходит подробность, и остаться незамеченной (конституция, принцип IV).
    use vrcast_studio_lib::domain::wording::Detail;

    vrcast_studio_lib::store::redact::forget_all();
    let secret = "парольная-фраза-ключа-4242";
    vrcast_studio_lib::store::redact::register(secret);

    let err = AppError::new(ErrorCode::InvalidInput).with_detail(
        Detail::new(DetailCode::UploadSourceUnreadable)
            .with("path", format!("F:/{secret}/film.mp4")),
    );
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
    // Подробность сохранена: по ней можно найти, о какой задаче речь.
    assert!(err.cause.is_some());
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
    // Объяснение — код с подстановкой, а не готовая фраза: формулировку подберёт
    // интерфейс на том языке, который выбран сейчас.
    assert_eq!(c.explanation.key, DetailCode::OnCloseRestartsLosing);
    assert_eq!(u.explanation.key, DetailCode::OnCloseResumesFrom);
    assert!(
        u.explanation.params.contains_key("percent"),
        "объяснение не называет, сколько уже сделано: {:?}",
        u.explanation
    );

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
