//! Тесты механизма задач (T016, T017, T019, T020).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use vrcast_studio_lib::store::db::Db;
use vrcast_studio_lib::tasks::engine::{TaskEngine, TaskEvent};
use vrcast_studio_lib::tasks::progress::ProgressThrottle;
use vrcast_studio_lib::tasks::state::{Lane, LaneLimits, PauseKind, TaskKind, TaskState};
use vrcast_studio_lib::tasks::store;

fn engine() -> TaskEngine {
    TaskEngine::new(Arc::new(Db::open_in_memory().unwrap()))
}

async fn wait_for_state(e: &TaskEngine, id: &str, want: TaskState, limit: Duration) -> bool {
    let deadline = std::time::Instant::now() + limit;
    while std::time::Instant::now() < deadline {
        if let Ok(Some(rec)) = e.get(id) {
            if rec.state == want {
                return true;
            }
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    false
}

// ---------- переходы состояний: чистая логика ----------

#[test]
fn переходы_состояний_подчиняются_таблице() {
    use TaskState::*;
    assert!(Queued.can_transition_to(Running));
    assert!(Queued.can_transition_to(Cancelled));
    assert!(Running.can_transition_to(Paused));
    assert!(Paused.can_transition_to(Running));
    assert!(Running.can_transition_to(Completed));

    // Из очереди сразу в завершённые — нельзя: задача не выполнялась.
    assert!(!Queued.can_transition_to(Completed));
    // Из завершённых переходов нет вовсе.
    assert!(!Completed.can_transition_to(Running));
    assert!(!Cancelled.can_transition_to(Running));
    assert!(!Failed.can_transition_to(Running));

    // Переход в себя разрешён: повторная отмена не ошибка (принцип V).
    assert!(Cancelled.can_transition_to(Cancelled));
}

#[test]
fn полосы_разводят_задачи_по_ресурсам() {
    // Подготовка упирается в вычисления, передача — в канал: мешать друг другу
    // им незачем, а две подготовки сразу вдвое медленнее каждая.
    assert_eq!(TaskKind::Convert.lane(), Lane::Compute);
    assert_eq!(TaskKind::Upload.lane(), Lane::Network);
    assert_eq!(TaskKind::Probe.lane(), Lane::Light);
    assert_ne!(TaskKind::Convert.lane(), TaskKind::Upload.lane());

    let l = LaneLimits::default();
    assert_eq!(l.for_lane(Lane::Compute), 1);
    assert_eq!(l.for_lane(Lane::Network), 1);
    assert!(l.for_lane(Lane::Light) > 1);
}

#[test]
fn виды_задач_по_разному_переносят_приостановку() {
    // Разница не косметическая: от неё зависит, что сказать пользователю при закрытии
    // приложения (FR-086).
    assert_eq!(
        TaskKind::Upload.pause_kind(),
        PauseKind::ResumableAcrossRestart
    );
    assert_eq!(TaskKind::Convert.pause_kind(), PauseKind::SuspendedProcess);
    assert_eq!(TaskKind::Probe.pause_kind(), PauseKind::NotPausable);
}

// ---------- ограничение частоты событий ----------

#[test]
fn частые_события_прогресса_отсеиваются() {
    let t = ProgressThrottle::new(Duration::from_millis(250));
    let start = std::time::Instant::now();

    assert!(t.allow_at(start, false), "первое событие должно пройти");
    assert!(!t.allow_at(start + Duration::from_millis(50), false));
    assert!(!t.allow_at(start + Duration::from_millis(200), false));
    assert!(t.allow_at(start + Duration::from_millis(300), false));
}

#[test]
fn важное_событие_проходит_всегда() {
    // Без этого исключения показатель застрянет на 87 % у задачи, которая уже кончилась.
    let t = ProgressThrottle::new(Duration::from_millis(250));
    let start = std::time::Instant::now();

    assert!(t.allow_at(start, false));
    assert!(!t.allow_at(start + Duration::from_millis(10), false));
    assert!(
        t.allow_at(start + Duration::from_millis(11), true),
        "важное событие отсеяно ограничителем"
    );
}

// ---------- очередь ----------

#[tokio::test]
async fn задача_выполняется_и_завершается() {
    let e = engine();
    let id = e
        .submit(TaskKind::Probe, None, |ctx| async move {
            ctx.report_important(0.5, "середина");
            Ok(())
        })
        .await
        .unwrap();

    assert!(
        wait_for_state(&e, &id, TaskState::Completed, Duration::from_secs(5)).await,
        "задача не завершилась"
    );
    let rec = e.get(&id).unwrap().unwrap();
    assert_eq!(
        rec.progress, 1.0,
        "у завершённой задачи прогресс должен быть полным"
    );
    assert!(rec.error.is_none());
}

#[tokio::test]
async fn неудача_записывается_как_неудача_а_не_как_успех() {
    let e = engine();
    let id = e
        .submit(TaskKind::Probe, None, |_ctx| async move {
            Err(String::from("файл не читается"))
        })
        .await
        .unwrap();

    assert!(wait_for_state(&e, &id, TaskState::Failed, Duration::from_secs(5)).await);
    let rec = e.get(&id).unwrap().unwrap();
    assert!(rec.error.unwrap().contains("не читается"));
    assert!(
        rec.progress < 1.0,
        "у неудавшейся задачи прогресс не должен быть полным"
    );
}

#[tokio::test]
async fn вторая_задача_той_же_полосы_ждёт_первую() {
    let e = engine();
    let running = Arc::new(AtomicUsize::new(0));
    let max_seen = Arc::new(AtomicUsize::new(0));

    let mut ids = Vec::new();
    for _ in 0..3 {
        let r = running.clone();
        let m = max_seen.clone();
        let id = e
            .submit(TaskKind::Convert, None, move |_ctx| async move {
                let now = r.fetch_add(1, Ordering::SeqCst) + 1;
                m.fetch_max(now, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(250)).await;
                r.fetch_sub(1, Ordering::SeqCst);
                Ok(())
            })
            .await
            .unwrap();
        ids.push(id);
    }

    for id in &ids {
        assert!(
            wait_for_state(&e, id, TaskState::Completed, Duration::from_secs(10)).await,
            "задача {id} не завершилась"
        );
    }

    assert_eq!(
        max_seen.load(Ordering::SeqCst),
        1,
        "в полосе вычислений одновременно шло больше одной задачи"
    );
}

#[tokio::test]
async fn задачи_разных_полос_идут_одновременно() {
    // Обратная проверка к предыдущей: общий предел на все задачи был бы неверен.
    let e = engine();
    let together = Arc::new(AtomicUsize::new(0));
    let max_seen = Arc::new(AtomicUsize::new(0));

    let mut ids = Vec::new();
    for kind in [TaskKind::Convert, TaskKind::Upload] {
        let t = together.clone();
        let m = max_seen.clone();
        let id = e
            .submit(kind, None, move |_ctx| async move {
                let now = t.fetch_add(1, Ordering::SeqCst) + 1;
                m.fetch_max(now, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(400)).await;
                t.fetch_sub(1, Ordering::SeqCst);
                Ok(())
            })
            .await
            .unwrap();
        ids.push(id);
    }

    for id in &ids {
        assert!(wait_for_state(&e, id, TaskState::Completed, Duration::from_secs(10)).await);
    }

    assert_eq!(
        max_seen.load(Ordering::SeqCst),
        2,
        "подготовка и передача не пошли одновременно, хотя занимают разные ресурсы"
    );
}

// ---------- отмена ----------

#[tokio::test]
async fn отмена_прерывает_выполняющуюся_задачу() {
    let e = engine();
    let finished_work = Arc::new(AtomicUsize::new(0));
    let fw = finished_work.clone();

    let id = e
        .submit(TaskKind::Convert, None, move |ctx| async move {
            for _ in 0..100 {
                ctx.bail_if_cancelled().map_err(|e| e.to_string())?;
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            fw.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
        .await
        .unwrap();

    assert!(wait_for_state(&e, &id, TaskState::Running, Duration::from_secs(5)).await);
    e.cancel(&id).unwrap();

    assert!(
        wait_for_state(&e, &id, TaskState::Cancelled, Duration::from_secs(5)).await,
        "задача не перешла в отменённые"
    );
    assert_eq!(
        finished_work.load(Ordering::SeqCst),
        0,
        "работа доделалась, хотя задачу отменили"
    );
}

#[tokio::test]
async fn отменённая_задача_не_считается_упавшей() {
    // Разница видна пользователю: снятая им задача не должна выглядеть ошибкой.
    // Работа долгая с точками отмены, а не «поспать 200 мс»: короткая успевала бы
    // завершиться до cancel на загруженной машине, и unwrap ронял бы тест ни за что.
    let e = engine();
    let id = e
        .submit(TaskKind::Convert, None, |ctx| async move {
            for _ in 0..600 {
                ctx.bail_if_cancelled().map_err(|e| e.to_string())?;
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            Ok(())
        })
        .await
        .unwrap();

    assert!(wait_for_state(&e, &id, TaskState::Running, Duration::from_secs(5)).await);
    e.cancel(&id).unwrap();
    assert!(wait_for_state(&e, &id, TaskState::Cancelled, Duration::from_secs(5)).await);

    let rec = e.get(&id).unwrap().unwrap();
    assert_eq!(rec.state, TaskState::Cancelled);
    assert!(
        rec.error.is_none(),
        "у отменённой задачи не должно быть ошибки"
    );
}

#[tokio::test]
async fn задачу_можно_снять_прямо_из_очереди() {
    // Стоящая в очереди задача ещё не начиналась — снимать её надо, не дожидаясь запуска.
    let e = TaskEngine::new(Arc::new(Db::open_in_memory().unwrap())).with_limits(LaneLimits {
        compute: 1,
        network: 1,
        light: 1,
    });
    let started = Arc::new(AtomicUsize::new(0));

    let s1 = started.clone();
    let blocker = e
        .submit(TaskKind::Convert, None, move |_c| async move {
            s1.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(900)).await;
            Ok(())
        })
        .await
        .unwrap();

    let s2 = started.clone();
    let queued = e
        .submit(TaskKind::Convert, None, move |_c| async move {
            s2.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
        .await
        .unwrap();

    assert!(wait_for_state(&e, &blocker, TaskState::Running, Duration::from_secs(5)).await);
    e.cancel(&queued).unwrap();

    assert!(
        wait_for_state(&e, &queued, TaskState::Cancelled, Duration::from_secs(5)).await,
        "задача в очереди не снялась"
    );
    assert_eq!(
        started.load(Ordering::SeqCst),
        1,
        "снятая из очереди задача всё-таки запустилась"
    );

    e.cancel(&blocker).unwrap();
}

// ---------- приостановка ----------

#[tokio::test]
async fn приостановка_останавливает_работу_а_продолжение_возобновляет() {
    let e = engine();
    let steps = Arc::new(AtomicUsize::new(0));
    let s = steps.clone();

    let id = e
        .submit(TaskKind::Upload, None, move |ctx| async move {
            for _ in 0..200 {
                ctx.wait_while_paused().await;
                if ctx.is_cancelled() {
                    return Ok(());
                }
                s.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            Ok(())
        })
        .await
        .unwrap();

    assert!(wait_for_state(&e, &id, TaskState::Running, Duration::from_secs(5)).await);
    tokio::time::sleep(Duration::from_millis(150)).await;

    e.pause(&id).unwrap();
    // Начатой итерации даём долететь до точки приостановки с запасом: короткое окно
    // здесь оборачивалось ложным «работа продолжалась» при преемпции под нагрузкой.
    tokio::time::sleep(Duration::from_millis(400)).await;
    let frozen = steps.load(Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(
        steps.load(Ordering::SeqCst),
        frozen,
        "работа продолжалась после приостановки"
    );

    e.resume(&id).unwrap();
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(
        steps.load(Ordering::SeqCst) > frozen,
        "работа не возобновилась после продолжения"
    );

    e.cancel(&id).unwrap();
}

#[tokio::test]
async fn отмена_будит_и_снимает_задачу_стоящую_на_паузе() {
    // Дефект, ради которого тест: отмена не сбрасывает флаг приостановки, и задача,
    // спящая в wait_while_paused, просыпалась от notify, видела «всё ещё пауза» и
    // засыпала обратно — навсегда. Ни отмены, ни события, ни записи в базе.
    let e = engine();
    let id = e
        .submit(TaskKind::Upload, None, move |ctx| async move {
            for _ in 0..600 {
                ctx.wait_while_paused().await;
                if ctx.is_cancelled() {
                    return Ok(());
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            Ok(())
        })
        .await
        .unwrap();

    assert!(wait_for_state(&e, &id, TaskState::Running, Duration::from_secs(5)).await);
    e.pause(&id).unwrap();
    // Даём задаче дойти до точки приостановки и заснуть в ней по-настоящему.
    tokio::time::sleep(Duration::from_millis(200)).await;

    e.cancel(&id).unwrap();
    assert!(
        wait_for_state(&e, &id, TaskState::Cancelled, Duration::from_secs(5)).await,
        "отменённая на паузе задача зависла навсегда"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn одновременный_старт_не_обходит_предел_полосы() {
    // Дефект, ради которого тест: проверка места и смена состояния шли под разными
    // захватами замка, и две задачи, проснувшиеся одновременно на разных потоках,
    // обе видели одно свободное место — две подготовки в полосе на одну.
    // Однопоточный исполнитель эту гонку скрывает, поэтому здесь многопоточный.
    let e = engine();
    let running = Arc::new(AtomicUsize::new(0));
    let max_seen = Arc::new(AtomicUsize::new(0));

    let mut ids = Vec::new();
    for _ in 0..6 {
        let r = running.clone();
        let m = max_seen.clone();
        let id = e
            .submit(TaskKind::Convert, None, move |_ctx| async move {
                let now = r.fetch_add(1, Ordering::SeqCst) + 1;
                m.fetch_max(now, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(60)).await;
                r.fetch_sub(1, Ordering::SeqCst);
                Ok(())
            })
            .await
            .unwrap();
        ids.push(id);
    }

    for id in &ids {
        assert!(
            wait_for_state(&e, id, TaskState::Completed, Duration::from_secs(15)).await,
            "задача {id} не завершилась"
        );
    }

    assert_eq!(
        max_seen.load(Ordering::SeqCst),
        1,
        "две задачи одновременно заняли полосу вычислений"
    );
}

#[tokio::test]
async fn короткую_задачу_приостановить_нельзя() {
    let e = engine();
    let id = e
        .submit(TaskKind::Probe, None, |_c| async move {
            tokio::time::sleep(Duration::from_millis(600)).await;
            Ok(())
        })
        .await
        .unwrap();

    assert!(wait_for_state(&e, &id, TaskState::Running, Duration::from_secs(5)).await);
    let err = e
        .pause(&id)
        .expect_err("разбор исходника не должен приостанавливаться");
    assert!(
        err.to_string().contains("нельзя приостановить"),
        "получено: {err}"
    );
}

// ---------- переживание перезапуска ----------

#[tokio::test]
async fn прерванная_задача_становится_приостановленной_а_не_завершённой() {
    // Конституция, принцип III и SC-010. «Завершено» означало бы, что результат готов,
    // а он оборван на середине — и это самая опасная из возможных подмен.
    let db = Arc::new(Db::open_in_memory().unwrap());

    let mut rec = store::TaskRecord::new("t-прерванная", TaskKind::Upload, None);
    rec.state = TaskState::Running;
    rec.progress = 0.42;
    rec.resume_token = Some(String::from("12400000000"));
    store::upsert(&db, &rec).unwrap();

    let e = TaskEngine::new(db.clone());
    let report = e.recover_after_start().unwrap();

    assert_eq!(report.interrupted, vec!["t-прерванная".to_string()]);
    let after = store::get(&db, "t-прерванная").unwrap().unwrap();
    assert_eq!(
        after.state,
        TaskState::Paused,
        "состояние не приостановленное"
    );
    assert_ne!(after.state, TaskState::Completed);
    assert_eq!(
        after.resume_token.as_deref(),
        Some("12400000000"),
        "потеряна позиция возобновления"
    );
}

#[test]
fn точечная_запись_не_затирает_чужие_поля() {
    // Дефект, ради которого тест: и токен, и состояние писались через
    // «прочитать-изменить-записать» всей записи, и параллельные пауза и запись токена
    // затирали друг друга. Точечные обновления обязаны не трогать чужие поля.
    let db = Db::open_in_memory().unwrap();
    let mut rec = store::TaskRecord::new("t-точечная", TaskKind::Upload, None);
    rec.resume_token = Some(String::from("старый-токен"));
    store::upsert(&db, &rec).unwrap();

    assert!(store::save_state(&db, "t-точечная", TaskState::Paused, None).unwrap());
    store::save_resume_token(&db, "t-точечная", "свежий-токен").unwrap();

    let after = store::get(&db, "t-точечная").unwrap().unwrap();
    assert_eq!(
        after.state,
        TaskState::Paused,
        "запись токена затёрла состояние"
    );
    assert_eq!(
        after.resume_token.as_deref(),
        Some("свежий-токен"),
        "запись состояния затёрла токен"
    );

    // Ошибка дописывается, не стирая уже записанного токена.
    assert!(store::save_state(&db, "t-точечная", TaskState::Failed, Some("обрыв связи")).unwrap());
    let after = store::get(&db, "t-точечная").unwrap().unwrap();
    assert_eq!(after.resume_token.as_deref(), Some("свежий-токен"));
    assert_eq!(after.error.as_deref(), Some("обрыв связи"));

    // Записи нет — save_state честно говорит об этом, а не молчит.
    assert!(!store::save_state(&db, "t-нет-такой", TaskState::Failed, None).unwrap());
}

#[tokio::test]
async fn позиция_возобновления_сохраняется_и_читается() {
    let e = engine();
    let id = e
        .submit(TaskKind::Upload, None, |ctx| async move {
            assert!(
                ctx.resume_token().unwrap().is_none(),
                "позиция взялась из ниоткуда"
            );
            ctx.save_resume_token("8388608")
                .map_err(|e| e.to_string())?;
            assert_eq!(ctx.resume_token().unwrap().as_deref(), Some("8388608"));
            Ok(())
        })
        .await
        .unwrap();

    assert!(wait_for_state(&e, &id, TaskState::Completed, Duration::from_secs(5)).await);
    assert_eq!(
        e.get(&id).unwrap().unwrap().resume_token.as_deref(),
        Some("8388608")
    );
}

// ---------- события ----------

#[tokio::test]
async fn о_завершении_сообщается_событием() {
    let e = engine();
    let mut rx = e.subscribe();

    let id = e
        .submit(TaskKind::Probe, None, |ctx| async move {
            ctx.report_important(0.5, "работаем");
            Ok(())
        })
        .await
        .unwrap();

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let mut got_done = false;
    while tokio::time::Instant::now() < deadline && !got_done {
        if let Ok(Ok(TaskEvent::Done {
            id: done_id, state, ..
        })) = tokio::time::timeout(Duration::from_millis(500), rx.recv()).await
        {
            if done_id == id {
                assert_eq!(state, TaskState::Completed);
                got_done = true;
            }
        }
    }
    assert!(got_done, "событие о завершении не пришло");
}
