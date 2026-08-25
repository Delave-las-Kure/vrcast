//! Тесты механизма задач (T016, T017, T019, T020).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
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

    // Из приостановленной можно и завершиться. Приостановка вступает в силу
    // на ближайшей точке остановки, и работа успевает дойти до конца, пока задача
    // уже помечена приостановленной: передача дописывает последнее окно.
    // Раньше таблица это запрещала, а движок делал всё равно — и расхождение
    // молчало (задолженность T072).
    assert!(Paused.can_transition_to(Completed));
    assert!(Paused.can_transition_to(Failed));
}

#[tokio::test]
async fn задача_завершившаяся_на_паузе_записывается_завершённой() {
    // Проверяется не таблица, а движок: он и раньше писал сюда «завершена»,
    // но таблица это запрещала, и никто не знал, кто из них прав.
    let e = engine();
    let дошла = Arc::new(AtomicUsize::new(0));
    let d = дошла.clone();

    let id = e
        .submit(TaskKind::Upload, None, move |ctx| async move {
            // Работа успевает закончиться, хотя приостановку уже попросили:
            // сама точка остановки — впереди, и до неё дело не дойдёт.
            tokio::time::sleep(Duration::from_millis(150)).await;
            d.fetch_add(1, Ordering::SeqCst);
            let _ = ctx;
            Ok(())
        })
        .await
        .unwrap();

    assert!(wait_for_state(&e, &id, TaskState::Running, Duration::from_secs(3)).await);
    e.pause(&id).expect("задача не приостановилась");

    assert!(
        wait_for_state(&e, &id, TaskState::Completed, Duration::from_secs(5)).await,
        "работа дошла до конца, но записана не завершённой: {:?}",
        e.get(&id).unwrap().unwrap().state
    );
    assert_eq!(дошла.load(Ordering::SeqCst), 1);
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

// ---------- порядок очереди (T096, FR-083) ----------

/// Движок с полосой на одну задачу и общая копилка порядка выполнения.
///
/// Проверять порядок можно только так: изнутри задачи, в момент, когда она пошла.
/// По записям в базе этого не увидеть — там остаётся лишь исход.
fn очередь_на_одного() -> (TaskEngine, Arc<Mutex<Vec<String>>>) {
    let e = TaskEngine::new(Arc::new(Db::open_in_memory().unwrap())).with_limits(LaneLimits {
        compute: 1,
        network: 1,
        light: 1,
    });
    (e, Arc::new(Mutex::new(Vec::new())))
}

/// Поставить задачу, которая отмечается в копилке и ждёт отмашки.
///
/// Ждать нужно, чтобы очередь успела сложиться: без задержки первая задача
/// закончится раньше, чем встанет третья, и переставлять будет нечего. Отмашка —
/// опрашиваемый признак, а не сигнал: сигнал будит только тех, кто ждёт его прямо
/// сейчас, а задачи здесь доходят до ожидания по очереди, освобождая полосу.
async fn поставить(
    e: &TaskEngine,
    имя: &str,
    порядок: Arc<Mutex<Vec<String>>>,
    отпустить: Arc<std::sync::atomic::AtomicBool>,
) -> String {
    let имя = имя.to_owned();
    e.submit(TaskKind::Upload, None, move |_ctx| async move {
        порядок.lock().unwrap().push(имя);
        while !отпустить.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        Ok(())
    })
    .await
    .expect("задача не поставилась")
}

async fn все_завершились(e: &TaskEngine, ids: &[&String]) {
    for id in ids {
        assert!(
            wait_for_state(e, id, TaskState::Completed, Duration::from_secs(10)).await,
            "задача {id} не завершилась"
        );
    }
}

#[tokio::test]
async fn без_перестановки_задачи_идут_в_порядке_постановки() {
    let (e, порядок) = очередь_на_одного();
    let отпустить = Arc::new(std::sync::atomic::AtomicBool::new(false));

    let первая = поставить(&e, "первая", порядок.clone(), отпустить.clone()).await;
    let вторая = поставить(&e, "вторая", порядок.clone(), отпустить.clone()).await;
    let третья = поставить(&e, "третья", порядок.clone(), отпустить.clone()).await;

    assert!(wait_for_state(&e, &первая, TaskState::Running, Duration::from_secs(3)).await);
    assert_eq!(
        e.queue_order(),
        vec![вторая.clone(), третья.clone()],
        "очередь показана не в том порядке, в каком задачи пойдут"
    );

    отпустить.store(true, Ordering::SeqCst);
    все_завершились(&e, &[&первая, &вторая, &третья]).await;

    assert_eq!(
        *порядок.lock().unwrap(),
        vec!["первая", "вторая", "третья"],
        "задачи пошли не в том порядке, в каком их поставили"
    );
}

#[tokio::test]
async fn перестановка_меняет_то_какая_задача_пойдёт_следующей() {
    // Ради этого FR-083 и существует. Проверяется по тому, какая задача пошла
    // в работу, а не по полю в базе: поле можно переставить и без последствий,
    // и тогда кнопка в интерфейсе была бы обманом.
    let (e, порядок) = очередь_на_одного();
    let отпустить = Arc::new(std::sync::atomic::AtomicBool::new(false));

    let первая = поставить(&e, "первая", порядок.clone(), отпустить.clone()).await;
    let вторая = поставить(&e, "вторая", порядок.clone(), отпустить.clone()).await;
    let третья = поставить(&e, "третья", порядок.clone(), отпустить.clone()).await;

    assert!(wait_for_state(&e, &первая, TaskState::Running, Duration::from_secs(3)).await);

    // Человек передумал: третья нужна раньше второй.
    let переставлено = e
        .reorder_queue(&[третья.clone(), вторая.clone()])
        .expect("перестановка не удалась");
    assert_eq!(переставлено, 2);
    assert_eq!(e.queue_order(), vec![третья.clone(), вторая.clone()]);

    отпустить.store(true, Ordering::SeqCst);
    все_завершились(&e, &[&первая, &вторая, &третья]).await;

    assert_eq!(
        *порядок.lock().unwrap(),
        vec!["первая", "третья", "вторая"],
        "перестановка не изменила того, какая задача пошла следующей"
    );
}

#[tokio::test]
async fn перестановка_не_прерывает_начатую_задачу() {
    // Прервать выполняющуюся ради изменения порядка значило бы выбросить уже
    // сделанную работу — на многочасовой заливке это часы.
    let (e, порядок) = очередь_на_одного();
    let отпустить = Arc::new(std::sync::atomic::AtomicBool::new(false));

    let первая = поставить(&e, "первая", порядок.clone(), отпустить.clone()).await;
    let вторая = поставить(&e, "вторая", порядок.clone(), отпустить.clone()).await;

    assert!(wait_for_state(&e, &первая, TaskState::Running, Duration::from_secs(3)).await);

    // Заявка включает уже начатую задачу — так и приходит из списка на экране.
    let переставлено = e
        .reorder_queue(&[вторая.clone(), первая.clone()])
        .expect("перестановка не удалась");
    assert_eq!(
        переставлено, 0,
        "переставлять было нечего: ждёт только одна задача"
    );
    assert_eq!(
        e.get(&первая).unwrap().unwrap().state,
        TaskState::Running,
        "выполняющуюся задачу прервали ради изменения порядка"
    );
}

#[tokio::test]
async fn порядок_переживает_перезапуск_приложения() {
    // Иначе человек расставит очередь на ночь, закроет приложение, а утром
    // обнаружит прежний порядок.
    let db = Arc::new(Db::open_in_memory().unwrap());
    let e = TaskEngine::new(db.clone()).with_limits(LaneLimits {
        compute: 1,
        network: 1,
        light: 1,
    });
    let порядок = Arc::new(Mutex::new(Vec::new()));
    let отпустить = Arc::new(std::sync::atomic::AtomicBool::new(false));

    let первая = поставить(&e, "первая", порядок.clone(), отпустить.clone()).await;
    let вторая = поставить(&e, "вторая", порядок.clone(), отпустить.clone()).await;
    let третья = поставить(&e, "третья", порядок.clone(), отпустить.clone()).await;

    assert!(wait_for_state(&e, &первая, TaskState::Running, Duration::from_secs(3)).await);
    e.reorder_queue(&[третья.clone(), вторая.clone()])
        .expect("перестановка не удалась");

    // Новый запуск читает ту же базу.
    let порядки: Vec<(String, i64)> = store::list(&db)
        .unwrap()
        .into_iter()
        .map(|t| (t.id, t.queue_order))
        .collect();
    let место = |id: &str| порядки.iter().find(|(i, _)| i == id).unwrap().1;
    assert!(
        место(&третья) < место(&вторая),
        "перестановка не дошла до базы и перезапуска не переживёт"
    );
}

#[tokio::test]
async fn новая_задача_встаёт_в_конец_очереди_прошлого_запуска() {
    // Отсчёт мест продолжается, а не начинается заново: иначе задача, поставленная
    // после перезапуска, молча влезла бы в середину чужой очереди.
    let db = Arc::new(Db::open_in_memory().unwrap());
    let mut старая = store::TaskRecord::new("t-старая", TaskKind::Upload, None);
    старая.state = TaskState::Queued;
    старая.queue_order = 100;
    store::upsert(&db, &старая).unwrap();

    let e = TaskEngine::new(db.clone());
    let id = e
        .submit(TaskKind::Upload, None, |_| async { Ok(()) })
        .await
        .unwrap();

    assert!(
        e.get(&id).unwrap().unwrap().queue_order > 100,
        "новая задача встала перед задачей прошлого запуска"
    );
}

#[tokio::test]
async fn задача_прошлого_запуска_поднимается_и_продолжается() {
    // FR-031. Без этого задача после перезапуска приложения видна в списке
    // приостановленной, но продолжить её нечем: рабочая часть живёт только в памяти
    // и умирает вместе с приложением. Человеку это выглядит как «задача есть,
    // а кнопка не работает».
    let db = Arc::new(Db::open_in_memory().unwrap());

    // Изображаем прошлый запуск: задача была в работе, приложение умерло.
    let mut rec = store::TaskRecord::new("t-прошлая", TaskKind::Upload, None);
    rec.state = TaskState::Running;
    rec.progress = 0.4;
    store::upsert(&db, &rec).unwrap();

    let e = TaskEngine::new(db.clone());
    e.recover_after_start().unwrap();
    assert_eq!(
        e.get("t-прошлая").unwrap().unwrap().state,
        TaskState::Paused,
        "прерванная задача должна стать приостановленной, а не завершённой"
    );

    let ran = Arc::new(AtomicUsize::new(0));
    let r = ran.clone();
    e.resubmit_paused("t-прошлая", move |ctx| async move {
        ctx.wait_while_paused().await;
        r.fetch_add(1, Ordering::SeqCst);
        Ok(())
    })
    .expect("задача не поднялась");

    // Поднятая задача ждёт человека и сама не начинается.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        ran.load(Ordering::SeqCst),
        0,
        "поднятая задача продолжилась самовольно — а приложение могли закрыть \
         именно ради её прекращения"
    );
    assert_eq!(
        e.get("t-прошлая").unwrap().unwrap().state,
        TaskState::Paused
    );

    // И продолжается по слову человека — тем же номером, что был.
    e.resume("t-прошлая")
        .expect("поднятая задача не продолжилась");
    assert!(
        wait_for_state(
            &e,
            "t-прошлая",
            TaskState::Completed,
            Duration::from_secs(5)
        )
        .await,
        "продолженная задача не дошла до конца"
    );
    assert_eq!(ran.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn поднятая_задача_не_ждёт_собственного_места_в_полосе() {
    // Она уже числится выполняющейся, и, считая себя, никогда бы не дождалась
    // свободного места в полосе, где помещается одна.
    let db = Arc::new(Db::open_in_memory().unwrap());
    let mut rec = store::TaskRecord::new("t-одна", TaskKind::Convert, None);
    rec.state = TaskState::Running;
    store::upsert(&db, &rec).unwrap();

    let e = TaskEngine::new(db.clone()).with_limits(LaneLimits {
        compute: 1,
        network: 1,
        light: 1,
    });
    e.recover_after_start().unwrap();

    e.resubmit_paused("t-одна", |ctx| async move {
        ctx.wait_while_paused().await;
        Ok(())
    })
    .unwrap();
    e.resume("t-одна").unwrap();

    assert!(
        wait_for_state(&e, "t-одна", TaskState::Completed, Duration::from_secs(5)).await,
        "задача застряла в ожидании места, которое занимает сама"
    );
}

#[tokio::test]
async fn задачу_прошлого_запуска_можно_снять_даже_не_поднимая() {
    // Иначе она навсегда останется в списке приостановленной, и снять её будет нечем.
    let db = Arc::new(Db::open_in_memory().unwrap());
    let mut rec = store::TaskRecord::new("t-ненужная", TaskKind::Upload, None);
    rec.state = TaskState::Paused;
    store::upsert(&db, &rec).unwrap();

    let e = TaskEngine::new(db.clone());
    e.cancel("t-ненужная").expect("задачу не снять");

    assert_eq!(
        e.get("t-ненужная").unwrap().unwrap().state,
        TaskState::Cancelled
    );
    // Повтор безопасен (конституция, принцип V).
    e.cancel("t-ненужная")
        .expect("повторное снятие считается ошибкой");
    // А несуществующую снять нельзя — это не то же самое, что уже снятую.
    assert!(e.cancel("нет-такой").is_err());
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
